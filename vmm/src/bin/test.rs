use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
// Usage:
// KERNEL_PATH=/path/to/kernel INITRAMFS_PATH=/path/to/initramfs cargo run --bin test
// SERIAL_OUTPUT=/path/to/output.log - optional, to capture serial output
use std::{u32, u8, env};
use std::path::Path;

use vmm::VMM;

#[derive(Debug)]
pub enum Error {
    VmmNew(vmm::Error),

    VmmKernel(env::VarError),
    
    VmmConfigure(vmm::Error),

    VmmRun(vmm::Error),
}


static COUNT: AtomicUsize = AtomicUsize::new(0);
static LAST_PRESS: Mutex<Option<Instant>> = Mutex::new(None);
extern "C" fn handle_sigint(_: i32) {
    let now = Instant::now();
    let mut last = LAST_PRESS.lock().unwrap();

    // reset counter if last press > 2 sec ago
    if let Some(t) = *last {
        if now.duration_since(t) > Duration::from_secs(1) {
            COUNT.store(0, Ordering::SeqCst);
        }
    }

    *last = Some(now);

    let c = COUNT.fetch_add(1, Ordering::SeqCst) + 1;
    if c >= 3 {
        println!("Force-exiting program after 3 quick Ctrl-C presses");
        unsafe { libc::_exit(0); }
    }
}

fn main() {
    unsafe { libc::signal(libc::SIGINT, handle_sigint as usize); }

    let kernel_path = match env::var("KERNEL_PATH") {
        Ok(val) => val,
        Err(e) => return eprintln!("Error getting KERNEL_PATH: {}", e),
    };

    let initramfs_path = match env::var("INITRAMFS_PATH") {
        Ok(val) => val,
        Err(e) => return eprintln!("Error getting INITRAMFS_PATH: {}", e),
    };

    let vcpus: u8 = 2;
    let memory: u32 = 1024; // in MiB

    let vmm = match create_vmm() {
        Ok(vmm) => vmm,
        Err(e) => {
            eprintln!("Error creating VMM: {:?}", e);
            return;
        }
    };

    let vmm = match configure_vmm(vmm, vcpus, memory, &kernel_path, &initramfs_path) {
        Ok(vmm) => vmm,
        Err(e) => {
            eprintln!("Error configuring VMM: {:?}", e);
            return;
        }
    };

    if let Err(e) = start_vmm(vmm) {
        eprintln!("Error running VMM: {:?}", e);
    }
}

fn create_vmm() -> Result<VMM, Error> {
    // Check if serial output path is provided
    let writer: Box<dyn std::io::Write + Send> = if let Ok(serial_output) = env::var("SERIAL_OUTPUT") {
        println!("Serial output will be written to: {}", serial_output);
        Box::new(std::fs::File::create(&serial_output).expect("Failed to create serial output file"))
    } else {
        Box::new(std::io::stdout())
    };

    VMM::new(writer).map_err(Error::VmmNew)
}

fn configure_vmm(mut vmm: VMM, vcpus: u8, memory: u32, kernel_path: &str, initramfs_path: &str) -> Result<VMM, Error> {
    vmm.configure(vcpus, memory, kernel_path, initramfs_path)
        .map_err(Error::VmmConfigure)?;

    Ok(vmm)
}

fn start_vmm(mut vmm: VMM) -> Result<(), Error> {

    vmm.run();

    Ok(())
}