use crate::ip_manager::IpManager;
use sha2::{Digest, Sha256};
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tracing::{debug, error, info, warn};

/// Represents an active VM with allocated resources
pub struct VmHandle {
    pub vm_id: String,
    pub ip: Ipv4Addr,
    pub tap_device: String,
    vm_thread: Option<thread::JoinHandle<()>>,
    vm_running: Arc<std::sync::atomic::AtomicBool>,
    ip_manager: Arc<Mutex<IpManager>>,
}

#[derive(Debug)]
pub enum VmError {
    IpAllocation(String),
    NetworkSetup(String),
    InitramfsBuild(String),
    VmmCreation(String),
    VmmConfiguration(String),
    AgentTimeout,
    Cleanup(String),
}

impl std::fmt::Display for VmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VmError::IpAllocation(e) => write!(f, "IP allocation failed: {}", e),
            VmError::NetworkSetup(e) => write!(f, "Network setup failed: {}", e),
            VmError::InitramfsBuild(e) => write!(f, "Initramfs build failed: {}", e),
            VmError::VmmCreation(e) => write!(f, "VMM creation failed: {}", e),
            VmError::VmmConfiguration(e) => write!(f, "VMM configuration failed: {}", e),
            VmError::AgentTimeout => write!(f, "Agent in VM did not respond in time"),
            VmError::Cleanup(e) => write!(f, "Cleanup failed: {}", e),
        }
    }
}

impl std::error::Error for VmError {}

/// Configuration for launching a VM
pub struct VmConfig {
    pub kernel_path: PathBuf,
    pub work_dir: PathBuf,
    pub bridge_name: String,
    pub vcpus: u8,
    pub memory_mb: usize,
}

/// Generate a unique tap device name from VM ID using a hash
/// Linux interface names are limited to 15 characters (IFNAMSIZ - 1)
/// Format: tap-{11_hex_chars} (total 15 chars)
fn generate_tap_device_name(vm_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(vm_id.as_bytes());
    let hash = hasher.finalize();

    // Take first 11 hex characters from hash
    let hex = format!("{:x}", hash);
    format!("tap-{}", &hex[..11])
}

impl VmHandle {
    /// Creates and starts a new VM using VMM library
    pub async fn create(
        vm_id: String,
        config: &VmConfig,
        ip_manager: Arc<Mutex<IpManager>>,
    ) -> Result<Self, VmError> {
        info!(vm_id = %vm_id, "Creating new VM");

        // Step 1: Allocate IP from pool
        let ip = {
            let manager = ip_manager
                .lock()
                .map_err(|e| VmError::IpAllocation(format!("Mutex poisoned: {}", e)))?;
            manager
                .allocate_ip(&vm_id)
                .map_err(|e| VmError::IpAllocation(e.to_string()))?
        };

        let ip_addr: Ipv4Addr = ip
            .parse()
            .map_err(|e| VmError::IpAllocation(format!("Invalid IP format: {}", e)))?;

        info!(vm_id = %vm_id, ip = %ip_addr, "Allocated IP for VM");

        // Step 2: Generate unique tap device name
        let tap_device = generate_tap_device_name(&vm_id);
        debug!(vm_id = %vm_id, tap = %tap_device, "Generated tap device name");

        // Step 3: Build initramfs with agent
        let initramfs_path = match Self::build_initramfs_with_agent(&vm_id, &config, ip_addr).await
        {
            Ok(path) => path,
            Err(e) => {
                let _ = Self::release_ip_internal(&vm_id, &ip_manager);
                return Err(e);
            }
        };

        info!(vm_id = %vm_id, initramfs = %initramfs_path.display(), "Built initramfs");

        // Step 4: Spawn VMM in a dedicated thread
        let vm_running = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let vm_running_clone = Arc::clone(&vm_running);

        let kernel_path = config.kernel_path.clone();
        let tap_device_clone = tap_device.clone();
        let vcpus = config.vcpus;
        let memory_mb = config.memory_mb;
        let host_ip: Ipv4Addr = (u32::from(ip_addr) - 1).into();
        let netmask = Ipv4Addr::new(255, 255, 255, 0);

        let vm_thread = thread::spawn(move || {
            // Create dummy stdin/stdout for VMM
            let stdin = Box::new(
                std::fs::File::open("/dev/null").expect("Failed to open /dev/null for stdin"),
            );
            let stdout = Box::new(std::io::sink());
            let memory_size = (memory_mb as usize) << 20; // Convert MB to bytes

            // Create VMM
            let mut vmm = match vmm::VMM::new(stdin, stdout, memory_size) {
                Ok(v) => v,
                Err(e) => {
                    error!("Failed to create VMM: {:?}", e);
                    return;
                }
            };

            // Add network device (this creates the tap device)
            if let Err(e) = vmm.add_net_device(
                tap_device_clone.clone(),
                Some(ip_addr),
                Some(host_ip),
                Some(netmask),
            ) {
                error!("Failed to add network device: {:?}", e);
                return;
            }

            info!("Network device added, tap created");

            // Configure VMM with kernel and initramfs
            if let Err(e) = vmm.configure(
                vcpus,
                kernel_path.to_str().unwrap(),
                initramfs_path.to_str().unwrap(),
                None,
            ) {
                error!("Failed to configure VMM: {:?}", e);
                return;
            }

            info!("VMM configured, starting vCPUs");

            // Run VMM (this blocks until VM stops)
            vmm.run();

            info!("VMM stopped");
            vm_running_clone.store(false, std::sync::atomic::Ordering::SeqCst);
        });

        // Step 5: Wait for tap device to be created
        info!(vm_id = %vm_id, "Waiting for tap device to be created");
        let tap_path = format!("/sys/class/net/{}", tap_device);
        let max_wait = Duration::from_secs(10);
        let start = std::time::Instant::now();

        loop {
            if tokio::fs::metadata(&tap_path).await.is_ok() {
                info!(vm_id = %vm_id, tap = %tap_device, "Tap device created");
                break;
            }

            if start.elapsed() >= max_wait {
                error!(vm_id = %vm_id, tap = %tap_device, "Tap device not created within timeout");
                vm_running.store(false, std::sync::atomic::Ordering::SeqCst);
                let _ = Self::release_ip_internal(&vm_id, &ip_manager);
                return Err(VmError::NetworkSetup(format!(
                    "Tap device {} not created within {:?}",
                    tap_device, max_wait
                )));
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        // Step 6: Attach tap to bridge
        if let Err(e) = virt::network::setup_guest_iface(&tap_device, &config.bridge_name).await {
            error!(vm_id = %vm_id, "Failed to attach tap to bridge: {}", e);
            vm_running.store(false, std::sync::atomic::Ordering::SeqCst);
            let _ = Self::release_ip_internal(&vm_id, &ip_manager);
            return Err(VmError::NetworkSetup(e.to_string()));
        }

        info!(vm_id = %vm_id, "Network setup complete");

        let mut handle = VmHandle {
            vm_id: vm_id.clone(),
            ip: ip_addr,
            tap_device,
            vm_thread: Some(vm_thread),
            vm_running,
            ip_manager,
        };

        // Step 7: Wait for agent to be ready
        if let Err(e) = handle.wait_for_agent_ready().await {
            handle.destroy().await;
            return Err(e);
        }

        info!(vm_id = %vm_id, ip = %ip_addr, "VM is ready with agent responding");
        Ok(handle)
    }

    /// Build initramfs with embedded agent binary
    async fn build_initramfs_with_agent(
        vm_id: &str,
        config: &VmConfig,
        _guest_ip: Ipv4Addr,
    ) -> Result<PathBuf, VmError> {
        debug!(vm_id = %vm_id, "Building initramfs with agent");

        let job_dir = config.work_dir.join(vm_id);
        tokio::fs::create_dir_all(&job_dir)
            .await
            .map_err(|e| VmError::InitramfsBuild(format!("Failed to create job dir: {}", e)))?;

        // TODO: Create initramfs with agent binary embedded
        let initramfs_path = job_dir.join("initramfs.cpio.gz");

        // Create empty initramfs for now
        if !initramfs_path.exists() {
            tokio::fs::write(&initramfs_path, b"").await.map_err(|e| {
                VmError::InitramfsBuild(format!("Failed to create placeholder: {}", e))
            })?;
        }

        warn!(vm_id = %vm_id, "Initramfs building not yet implemented - using empty file");

        Ok(initramfs_path)
    }

    /// Wait for the agent inside the VM to be ready (health check)
    async fn wait_for_agent_ready(&self) -> Result<(), VmError> {
        info!(vm_id = %self.vm_id, ip = %self.ip, "Waiting for agent to be ready");

        let agent_url = self.agent_url();
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|_e| VmError::AgentTimeout)?;

        // Try for up to 30 seconds
        for attempt in 1..=15 {
            debug!(vm_id = %self.vm_id, attempt = attempt, "Checking agent health");

            match client.get(&agent_url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    info!(vm_id = %self.vm_id, "Agent is ready");
                    return Ok(());
                }
                Ok(resp) => {
                    debug!(vm_id = %self.vm_id, status = %resp.status(), "Agent returned non-success");
                }
                Err(e) => {
                    debug!(vm_id = %self.vm_id, error = %e, "Agent not yet reachable");
                }
            }

            tokio::time::sleep(Duration::from_secs(2)).await;
        }

        error!(vm_id = %self.vm_id, "Agent did not become ready in time");
        Err(VmError::AgentTimeout)
    }

    /// Get the agent URL for this VM
    pub fn agent_url(&self) -> String {
        format!("http://{}:3001", self.ip)
    }

    /// Destroy the VM and cleanup all resources
    pub async fn destroy(&mut self) {
        info!(vm_id = %self.vm_id, "Destroying VM");

        // Signal VMM to stop
        self.vm_running
            .store(false, std::sync::atomic::Ordering::SeqCst);

        // Wait for VMM thread to finish
        if let Some(thread) = self.vm_thread.take() {
            // Don't wait forever
            let _ = thread.join();
        }

        // Cleanup tap device
        if let Err(e) = Self::cleanup_tap_device(&self.tap_device).await {
            error!(vm_id = %self.vm_id, error = %e, "Failed to cleanup tap device");
        }

        // Release IP
        if let Err(e) = Self::release_ip_internal(&self.vm_id, &self.ip_manager) {
            error!(vm_id = %self.vm_id, error = %e, "Failed to release IP");
        }

        info!(vm_id = %self.vm_id, "VM destroyed");
    }

    /// Cleanup tap device
    async fn cleanup_tap_device(tap_device: &str) -> Result<(), String> {
        // The tap device is automatically destroyed when the VMM process exits
        // We just log it here
        debug!(tap = %tap_device, "Tap device cleanup (handled by kernel)");
        Ok(())
    }

    /// Release IP address (internal helper)
    fn release_ip_internal(vm_id: &str, ip_manager: &Arc<Mutex<IpManager>>) -> Result<(), String> {
        let manager = ip_manager
            .lock()
            .map_err(|e| format!("Mutex poisoned: {}", e))?;
        manager.release_ip(vm_id).map_err(|e| e.to_string())?;
        Ok(())
    }
}

impl Drop for VmHandle {
    fn drop(&mut self) {
        // Ensure VM is stopped when handle is dropped
        self.vm_running
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }
}
