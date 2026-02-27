use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct IpManagerState {
    pub allocations: HashMap<String, String>, // vm_id -> ip_address
}

#[derive(Debug)]
pub struct IpManager {
    file_path: PathBuf,
    start_ip: u32,
    end_ip: u32,
    lock: Mutex<()>,
}

#[derive(Debug)]
pub enum IpManagerError {
    Io(std::io::Error),
    Json(serde_json::Error),
    PoolExhausted,
    VmAlreadyHasIp(String),
}

impl std::fmt::Display for IpManagerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IpManagerError::Io(e) => write!(f, "IO error: {}", e),
            IpManagerError::Json(e) => write!(f, "JSON error: {}", e),
            IpManagerError::PoolExhausted => write!(f, "IP pool exhausted"),
            IpManagerError::VmAlreadyHasIp(ip) => write!(f, "VM already has IP: {}", ip),
        }
    }
}

impl std::error::Error for IpManagerError {}

impl From<std::io::Error> for IpManagerError {
    fn from(err: std::io::Error) -> Self {
        IpManagerError::Io(err)
    }
}

impl From<serde_json::Error> for IpManagerError {
    fn from(err: serde_json::Error) -> Self {
        IpManagerError::Json(err)
    }
}

impl IpManager {
    pub fn new<P: AsRef<Path>>(file_path: P, start_ip: Ipv4Addr, end_ip: Ipv4Addr) -> Result<Self, IpManagerError> {
        let manager = Self {
            file_path: file_path.as_ref().to_path_buf(),
            start_ip: u32::from(start_ip),
            end_ip: u32::from(end_ip),
            lock: Mutex::new(()),
        };

        if !manager.file_path.exists() {
            manager.write_state(&IpManagerState::default())?;
        }

        Ok(manager)
    }

    fn read_state(&self) -> Result<IpManagerState, IpManagerError> {
        let mut file = match File::open(&self.file_path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(IpManagerState::default());
            }
            Err(e) => return Err(e.into()),
        };

        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        
        if contents.trim().is_empty() {
            return Ok(IpManagerState::default());
        }

        let state: IpManagerState = serde_json::from_str(&contents)?;
        Ok(state)
    }

    fn write_state(&self, state: &IpManagerState) -> Result<(), IpManagerError> {
        let json = serde_json::to_string_pretty(state)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.file_path)?;
        file.write_all(json.as_bytes())?;
        Ok(())
    }

    pub fn allocate_ip(&self, vm_id: &str) -> Result<String, IpManagerError> {
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

        let ip = selected_ip.ok_or(IpManagerError::PoolExhausted)?;
        state.allocations.insert(vm_id.to_string(), ip.clone());
        
        self.write_state(&state)?;

        Ok(ip)
    }

    pub fn release_ip(&self, vm_id: &str) -> Result<bool, IpManagerError> {
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
        assert!(matches!(res, Err(IpManagerError::PoolExhausted)));
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
