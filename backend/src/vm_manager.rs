use crate::ip_manager::IpManager;
use std::sync::Mutex;
use tracing::info;

/// VM manager that uses the VMM library
pub struct VmManager {
    // IP pool manager for allocating IPs to VMs
    ip_manager: Mutex<IpManager>,
}

impl VmManager {
    pub fn new(ip_manager: IpManager) -> Self {
        Self {
            ip_manager: Mutex::new(ip_manager),
        }
    }

    /// Allocates an IP address for a new VM
    /// 
    /// # Arguments
    /// * `vm_id` - Unique identifier for the VM
    /// 
    /// # Returns
    /// The allocated IP address as a String
    pub fn allocate_ip(&self, vm_id: &str) -> Result<String, String> {
        self.ip_manager
            .lock()
            .unwrap()
            .allocate_ip(vm_id)
            .map_err(|e| format!("Failed to allocate IP: {}", e))
    }

    /// Releases the IP address associated with a VM
    /// 
    /// # Arguments
    /// * `vm_id` - Unique identifier for the VM
    pub fn release_ip(&self, vm_id: &str) -> Result<(), String> {
        self.ip_manager
            .lock()
            .unwrap()
            .release_ip(vm_id)
            .map_err(|e| format!("Failed to release IP: {}", e))?;
        Ok(())
    }

    /// Creates a VM and sends code to execute
    /// 
    /// # Arguments
    /// * `vm_id` - Unique identifier for the VM
    /// * `vm_ip` - IP address to assign to the VM
    /// * `language` - Code language (python, node, rust)
    /// * `code` - Source code to execute
    pub async fn send_code_to_vm(
        &self,
        vm_id: &str,
        vm_ip: &str,
        language: &str,
        code: &str,
    ) -> Result<(), String> {
        info!(
            "Sending code to VMM - VM ID: {}, IP: {}, Language: {}",
            vm_id, vm_ip, language
        );
        
        // TODO: Here we will:
        // 1. Create a VMM::new() instance
        // 2. Configure network with vm.add_net_device(vm_ip)
        // 3. Configure kernel and initramfs with the code
        // 4. Start the VM with vm.run()
        
        info!("Code received (length: {} bytes)", code.len());
        info!("Code preview: {}", &code[..code.len().min(100)]);
        
        // For now, we simulate sending
        Ok(())
    }
}
