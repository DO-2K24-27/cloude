use crate::ip_manager::{IpManager, IpManagerError};
use sha2::{Digest, Sha256};
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::process::Child;
use tracing::{debug, error, info, warn};

/// Represents an active VM with allocated resources
pub struct VmHandle {
    pub vm_id: String,
    pub ip: Ipv4Addr,
    pub tap_device: String,
    vm_process: Option<Child>,
    ip_manager: Arc<Mutex<IpManager>>,
}

#[derive(Debug)]
pub enum VmError {
    IpAllocation(String),
    NetworkSetup(String),
    InitramfsBuild(String),
    VmSpawn(String),
    AgentTimeout,
    Cleanup(String),
}

impl std::fmt::Display for VmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VmError::IpAllocation(e) => write!(f, "IP allocation failed: {}", e),
            VmError::NetworkSetup(e) => write!(f, "Network setup failed: {}", e),
            VmError::InitramfsBuild(e) => write!(f, "Initramfs build failed: {}", e),
            VmError::VmSpawn(e) => write!(f, "VM spawn failed: {}", e),
            VmError::AgentTimeout => write!(f, "Agent in VM did not respond in time"),
            VmError::Cleanup(e) => write!(f, "Cleanup failed: {}", e),
        }
    }
}

impl std::error::Error for VmError {}

impl From<IpManagerError> for VmError {
    fn from(err: IpManagerError) -> Self {
        VmError::IpAllocation(err.to_string())
    }
}

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
    /// Creates and starts a new VM with the agent embedded
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
        
        let ip_addr: Ipv4Addr = ip.parse()
            .map_err(|e| VmError::IpAllocation(format!("Invalid IP format: {}", e)))?;
        
        info!(vm_id = %vm_id, ip = %ip_addr, "Allocated IP for VM");

        // Step 2: Generate unique tap device name using hash
        let tap_device = generate_tap_device_name(&vm_id);
        debug!(vm_id = %vm_id, tap = %tap_device, "Generated tap device name");
        
        // Step 3: Setup networking for this VM
        if let Err(e) = Self::setup_vm_network(&tap_device, &config.bridge_name, ip_addr).await {
            // Cleanup IP if network setup fails
            let _ = Self::release_ip_internal(&vm_id, &ip_manager);
            return Err(e);
        }

        // Step 4: Build initramfs with agent
        let initramfs_path = match Self::build_initramfs_with_agent(&vm_id, &config, ip_addr).await {
            Ok(path) => path,
            Err(e) => {
                // Cleanup on failure
                let _ = Self::cleanup_tap_device(&tap_device).await;
                let _ = Self::release_ip_internal(&vm_id, &ip_manager);
                return Err(e);
            }
        };

        info!(vm_id = %vm_id, initramfs = %initramfs_path.display(), "Built initramfs with agent");

        // Step 5: Spawn VM process with VMM
        let vm_process = match Self::spawn_vm_process(&config, &initramfs_path, &tap_device, ip_addr).await {
            Ok(process) => process,
            Err(e) => {
                // Cleanup on failure
                let _ = Self::cleanup_tap_device(&tap_device).await;
                let _ = Self::release_ip_internal(&vm_id, &ip_manager);
                return Err(e);
            }
        };

        info!(vm_id = %vm_id, "VM process spawned");

        let mut handle = VmHandle {
            vm_id: vm_id.clone(),
            ip: ip_addr,
            tap_device,
            vm_process: Some(vm_process),
            ip_manager,
        };

        // Step 6: Wait for agent to be ready
        if let Err(e) = handle.wait_for_agent_ready().await {
            // Cleanup on timeout
            handle.destroy().await;
            return Err(e);
        }

        info!(vm_id = %vm_id, ip = %ip_addr, "VM is ready with agent responding");
        Ok(handle)
    }

    /// Setup networking for the VM (tap device + bridge attachment)
    async fn setup_vm_network(
        tap_device: &str,
        bridge_name: &str,
        _guest_ip: Ipv4Addr,
    ) -> Result<(), VmError> {
        debug!(tap = %tap_device, bridge = %bridge_name, "Setting up VM network");

        virt::network::setup_guest_iface(tap_device, bridge_name)
            .await
            .map_err(|e| VmError::NetworkSetup(e.to_string()))?;

        Ok(())
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
        // For now, we'll need to:
        // 1. Copy agent binary (compiled statically)
        // 2. Create init script that starts the agent with proper IP configuration
        // 3. Use cpio to create initramfs
        
        // Placeholder - this needs to be implemented properly
        let initramfs_path = job_dir.join("initramfs.cpio.gz");
        
        // This is a placeholder - actual implementation will build proper initramfs
        warn!(vm_id = %vm_id, "Initramfs building not yet fully implemented");
        
        Ok(initramfs_path)
    }

    /// Spawn the VM process using the run-vm binary from virt crate
    async fn spawn_vm_process(
        config: &VmConfig,
        initramfs_path: &Path,
        tap_device: &str,
        guest_ip: Ipv4Addr,
    ) -> Result<Child, VmError> {
        debug!("Spawning VM process");

        // Calculate host IP in same subnet as guest
        let host_ip: Ipv4Addr = (u32::from(guest_ip) - 1).into();
        let netmask = Ipv4Addr::new(255, 255, 255, 0);

        let child = tokio::process::Command::new("cargo")
            .args(&["run", "--bin", "run-vm", "-p", "virt"])
            .env("KERNEL_PATH", &config.kernel_path)
            .env("INITRAMFS_PATH", initramfs_path)
            .env("TAP_DEVICE", tap_device)
            .env("GUEST_IP", guest_ip.to_string())
            .env("HOST_IP", host_ip.to_string())
            .env("NETMASK", netmask.to_string())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| VmError::VmSpawn(format!("Failed to spawn VM: {}", e)))?;

        Ok(child)
    }

    /// Wait for the agent inside the VM to be ready (health check)
    async fn wait_for_agent_ready(&self) -> Result<(), VmError> {
        info!(vm_id = %self.vm_id, ip = %self.ip, "Waiting for agent to be ready");

        let agent_url = format!("http://{}:3001/health", self.ip);
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

        // Kill VM process
        if let Some(mut process) = self.vm_process.take() {
            if let Err(e) = process.kill().await {
                error!(vm_id = %self.vm_id, error = %e, "Failed to kill VM process");
            }
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

    async fn cleanup_tap_device(tap_device: &str) -> Result<(), VmError> {
        debug!(tap = %tap_device, "Cleaning up tap device");
        
        // TODO: Implement tap device cleanup
        // For now, the tap device will be cleaned up by the system
        
        Ok(())
    }

    fn release_ip_internal(vm_id: &str, ip_manager: &Arc<Mutex<IpManager>>) -> Result<(), VmError> {
        let manager = ip_manager
            .lock()
            .map_err(|e| VmError::Cleanup(format!("Mutex poisoned: {}", e)))?;
        
        manager
            .release_ip(vm_id)
            .map_err(|e| VmError::Cleanup(e.to_string()))?;
        
        Ok(())
    }
}

impl Drop for VmHandle {
    fn drop(&mut self) {
        warn!(vm_id = %self.vm_id, "VmHandle dropped - VM should have been explicitly destroyed");
    }
}
