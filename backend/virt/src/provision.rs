use std::net::Ipv4Addr;
use std::os::unix::io::FromRawFd;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use vmm::VMM;

/// Handle to a running VM. Stopping or dropping it terminates the VM.
pub struct VmHandle {
    pub guest_ip: Ipv4Addr,
    pub tap_name: String,
    running: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl VmHandle {
    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for VmHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

pub struct VmConfig {
    pub kernel_path: PathBuf,
    pub initramfs_path: PathBuf,
    pub tap_name: String,
    pub guest_ip: Ipv4Addr,
    pub host_ip: Ipv4Addr,
    pub netmask: Ipv4Addr,
    /// RAM in bytes (default: 512 MiB)
    pub memory: usize,
    pub vcpus: u8,
    /// Where to redirect the VM's serial console output. None discards it.
    pub log_path: Option<PathBuf>,
}

#[derive(Debug)]
pub enum ProvisionError {
    Io(std::io::Error),
    Vmm(String),
    ThreadSetup(String),
}

impl std::fmt::Display for ProvisionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProvisionError::Io(e) => write!(f, "IO error: {}", e),
            ProvisionError::Vmm(e) => write!(f, "VMM error: {}", e),
            ProvisionError::ThreadSetup(e) => write!(f, "Thread setup error: {}", e),
        }
    }
}

impl std::error::Error for ProvisionError {}

impl From<std::io::Error> for ProvisionError {
    fn from(e: std::io::Error) -> Self {
        ProvisionError::Io(e)
    }
}

/// Create a VM with the given config, start it in a background thread and
/// return a [`VmHandle`] that can be used to stop it later.
///
/// The VMM is created inside the thread to avoid needing `VMM: Send`.
/// A channel is used to receive the `running` flag (or any setup error) back.
///
/// After this function returns, call
/// `virt::network::setup_guest_iface(&config.tap_name, bridge_name)` to
/// attach the TAP to your bridge.
pub fn spawn_vm(config: VmConfig) -> Result<VmHandle, ProvisionError> {
    let kernel_path = config
        .kernel_path
        .to_str()
        .ok_or_else(|| ProvisionError::Vmm("Invalid kernel path".into()))?
        .to_string();

    let initramfs_path = config
        .initramfs_path
        .to_str()
        .ok_or_else(|| ProvisionError::Vmm("Invalid initramfs path".into()))?
        .to_string();

    let tap_name = config.tap_name.clone();
    let guest_ip = config.guest_ip;
    let host_ip = config.host_ip;
    let netmask = config.netmask;
    let memory = config.memory;
    let vcpus = config.vcpus;
    let log_path = config.log_path.clone();

    // Channel to get the running flag (or a setup error) back from the thread.
    // Only Arc<AtomicBool> crosses the thread boundary, which is Send.
    let (tx, rx) = std::sync::mpsc::channel::<Result<Arc<AtomicBool>, ProvisionError>>();

    let thread = std::thread::Builder::new()
        .name(format!("vm-{}", tap_name))
        .spawn(move || {
            // /dev/null does not support epoll (EPERM), so we use a pipe instead.
            // The read end supports epoll; the write end is kept open so reads block
            // rather than returning EOF immediately.
            let mut pipe_fds = [-1i32; 2];
            if unsafe { libc::pipe(pipe_fds.as_mut_ptr()) } != 0 {
                tx.send(Err(ProvisionError::Io(std::io::Error::last_os_error()))).ok();
                return;
            }
            let stdin_input = unsafe { std::fs::File::from_raw_fd(pipe_fds[0]) };
            let _write_end = unsafe { std::fs::File::from_raw_fd(pipe_fds[1]) };

            let output: Box<dyn std::io::Write + Send> = match &log_path {
                Some(path) => match std::fs::File::create(path) {
                    Ok(f) => Box::new(f),
                    Err(e) => { tx.send(Err(ProvisionError::Io(e))).ok(); return; }
                },
                None => match std::fs::File::create("/dev/null") {
                    Ok(f) => Box::new(f),
                    Err(e) => { tx.send(Err(ProvisionError::Io(e))).ok(); return; }
                },
            };

            let mut vmm = match VMM::new(Box::new(stdin_input), output, memory) {
                Ok(v) => v,
                Err(e) => { tx.send(Err(ProvisionError::Vmm(format!("{:?}", e)))).ok(); return; }
            };

            if let Err(e) = vmm.add_net_device(
                tap_name.clone(),
                Some(guest_ip),
                Some(host_ip),
                Some(netmask),
            ) {
                tx.send(Err(ProvisionError::Vmm(format!("{:?}", e)))).ok();
                return;
            }

            if let Err(e) = vmm.configure(vcpus, &kernel_path, &initramfs_path, None) {
                tx.send(Err(ProvisionError::Vmm(format!("{:?}", e)))).ok();
                return;
            }

            // Send the running flag back before blocking in run()
            tx.send(Ok(vmm.running_flag())).ok();
            vmm.run();
        })
        .map_err(ProvisionError::Io)?;

    // Wait for setup result from the thread
    let running = match rx.recv() {
        Ok(Ok(flag)) => flag,
        Ok(Err(e)) => {
            let _ = thread.join();
            return Err(e);
        }
        Err(_) => {
            return Err(ProvisionError::ThreadSetup(
                "VM thread exited before sending setup result".into(),
            ));
        }
    };

    Ok(VmHandle {
        guest_ip,
        tap_name: config.tap_name,
        running,
        thread: Some(thread),
    })
}
