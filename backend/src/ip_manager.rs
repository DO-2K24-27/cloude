use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Represents the serializable state of IP allocations.
/// This structure is mapped directly to the JSON file on disk.
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct IpManagerState {
    pub allocations: HashMap<String, String>, // vm_id -> ip_address
}

/// A thread-safe manager for allocating and releasing IP addresses for VMs.
/// State is persisted synchronously to a JSON file to prevent data loss.
#[derive(Debug)]
pub struct IpManager {
    file_path: PathBuf,
    start_ip: u32,
    end_ip: u32,
    lock: Mutex<()>,
}

impl IpManager {
    /// Creates a new `IpManager` or loads an existing state from the given file path.
    /// 
    /// # Arguments
    /// * `file_path` - Path to the JSON file used for persistence.
    /// * `start_ip` - The first IP address available in the allocation pool.
    /// * `end_ip` - The last IP address available in the allocation pool.
    pub fn new<P: AsRef<Path>>(file_path: P, start_ip: Ipv4Addr, end_ip: Ipv4Addr) -> Result<Self> {
        let manager = Self {
            file_path: file_path.as_ref().to_path_buf(),
            start_ip: u32::from(start_ip),
            end_ip: u32::from(end_ip),
            lock: Mutex::new(()),
        };

        if !manager.file_path.exists() {
            manager.write_state(&IpManagerState::default())
                .context("Failed to create initial state file")?;
        }

        Ok(manager)
    }

    /// Reads the current IP allocation state from the JSON file.
    /// If the file does not exist or is empty, it returns a new default state.
    fn read_state(&self) -> Result<IpManagerState> {
        let mut file = match File::open(&self.file_path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(IpManagerState::default());
            }
            Err(e) => return Err(e).context("Failed to open state file"),
        };

        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        
        if contents.trim().is_empty() {
            return Ok(IpManagerState::default());
        }

        let state: IpManagerState = serde_json::from_str(&contents)?;
        Ok(state)
    }

    /// Serializes the given `IpManagerState` and writes it atomically to the JSON file.
    /// Uses a temporary file + fsync + atomic rename to prevent corruption on crash.
    /// 
    /// # Arguments
    /// * `state` - The current allocation state to save over the previous one.
    fn write_state(&self, state: &IpManagerState) -> Result<()> {
        let json = serde_json::to_string_pretty(state)
            .context("Failed to serialize state")?;
        
        // Create temp file in the same directory as the target file
        let temp_path = self.file_path.with_extension("tmp");
        
        // Write to temp file
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temp_path)
            .context("Failed to create temporary state file")?;
        
        file.write_all(json.as_bytes())
            .context("Failed to write state to temporary file")?;
        
        // Fsync temp file to ensure data is on disk
        file.sync_all()
            .context("Failed to sync temporary file")?;
        
        // Drop file handle before rename
        drop(file);
        
        // Atomically replace the old file with the new one
        std::fs::rename(&temp_path, &self.file_path)
            .context("Failed to rename temporary file to state file")?;
        
        // Fsync parent directory to ensure the rename is durable
        if let Some(parent) = self.file_path.parent() {
            let parent_dir = OpenOptions::new()
                .read(true)
                .open(parent)
                .context("Failed to open parent directory for sync")?;
            parent_dir.sync_all()
                .context("Failed to sync parent directory")?;
        }
        
        Ok(())
    }

    /// Allocates an available IP address for the specified VM.
    /// If the VM already has an allocated IP, the existing IP is returned idempotently.
    /// 
    /// # Arguments
    /// * `vm_id` - A unique identifier for the Virtual Machine.
    pub fn allocate_ip(&self, vm_id: &str) -> Result<String> {
        let _guard = self.lock.lock().unwrap();
        let mut state = self.read_state()?;

        if let Some(existing_ip) = state.allocations.get(vm_id) {
            return Ok(existing_ip.clone());
        }

        let allocated_ips: HashSet<&String> = state.allocations.values().collect();

        let mut current_ip_val = self.start_ip;
        let mut selected_ip = None;

        while current_ip_val <= self.end_ip {
            let ip_addr = Ipv4Addr::from(current_ip_val).to_string();
            if !allocated_ips.contains(&ip_addr) {
                selected_ip = Some(ip_addr);
                break;
            }
            current_ip_val += 1;
        }

        let ip = selected_ip
            .ok_or_else(|| anyhow::anyhow!("IP pool exhausted"))?;
        state.allocations.insert(vm_id.to_string(), ip.clone());
        
        self.write_state(&state)?;

        Ok(ip)
    }

    /// Releases the IP address associated with the given VM, making it available again.
    /// 
    /// # Arguments
    /// * `vm_id` - The unique identifier of the Virtual Machine.
    /// 
    /// Returns `true` if an IP was successfully released, `false` if the VM had no IP allocated.
    pub fn release_ip(&self, vm_id: &str) -> Result<bool> {
        let _guard = self.lock.lock().unwrap();
        let mut state = self.read_state()?;

        if state.allocations.remove(vm_id).is_some() {
            self.write_state(&state)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn test_manager() -> (IpManager, NamedTempFile) {
        let file = NamedTempFile::new().unwrap();
        let start = Ipv4Addr::new(192, 168, 1, 10);
        let end = Ipv4Addr::new(192, 168, 1, 12); // Pool of 3 IPs
        let manager = IpManager::new(file.path(), start, end).unwrap();
        (manager, file)
    }

    #[test]
    fn test_allocate_and_release() {
        let (manager, _file) = test_manager();

        // Allocate 1
        let ip1 = manager.allocate_ip("vm-1").unwrap();
        assert_eq!(ip1, "192.168.1.10");

        // Release 1
        let released = manager.release_ip("vm-1").unwrap();
        assert!(released);

        // Re-allocate should get the same first available IP
        let ip1_again = manager.allocate_ip("vm-1").unwrap();
        assert_eq!(ip1_again, "192.168.1.10");
    }

    #[test]
    fn test_pool_exhaustion() {
        let (manager, _file) = test_manager();

        assert!(manager.allocate_ip("vm-1").is_ok());
        assert!(manager.allocate_ip("vm-2").is_ok());
        assert!(manager.allocate_ip("vm-3").is_ok());

        // 4th allocation should fail
        let res = manager.allocate_ip("vm-4");
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("IP pool exhausted"));
    }

    #[test]
    fn test_idempotent_allocation() {
        let (manager, _file) = test_manager();

        let ip1 = manager.allocate_ip("vm-1").unwrap();
        let ip1_again = manager.allocate_ip("vm-1").unwrap(); // Ask again for same VM

        assert_eq!(ip1, ip1_again);
    }

    #[test]
    fn test_persistence() {
        let file = NamedTempFile::new().unwrap();
        let start = Ipv4Addr::new(10, 0, 0, 1);
        let end = Ipv4Addr::new(10, 0, 0, 10);

        {
            let manager1 = IpManager::new(file.path(), start, end).unwrap();
            manager1.allocate_ip("vm-1").unwrap();
        } // manager1 dropped, file flushed

        {
            let manager2 = IpManager::new(file.path(), start, end).unwrap();
            // vm-1 should still have 10.0.0.1
            let state = manager2.read_state().unwrap();
            assert_eq!(state.allocations.get("vm-1").unwrap(), "10.0.0.1");

            // next allocation should be 10.0.0.2
            let ip2 = manager2.allocate_ip("vm-2").unwrap();
            assert_eq!(ip2, "10.0.0.2");
        }
    }
}
