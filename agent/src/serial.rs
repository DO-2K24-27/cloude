use anyhow::{Context, Result};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, debug};

#[derive(Debug, Clone)]
pub struct SerialConfig {
    pub ip: String,
    pub port: u16,
}

impl SerialConfig {
    pub fn new(ip: String, port: u16) -> Self {
        Self { ip, port }
    }
}

pub async fn read_serial_config() -> Result<SerialConfig> {
    // Temporary test solution - remove in production
    const TEST_FILE: &str = "/tmp/agent_serial_test";
    if std::path::Path::new(TEST_FILE).exists() {
        info!("Using test serial file: {}", TEST_FILE);
        return try_read_serial_config(TEST_FILE);
    }
    
    const SERIAL_DEVICE: &str = "/dev/ttyS0";
    const MAX_RETRIES: u32 = 30;
    const RETRY_DELAY: Duration = Duration::from_secs(1);

    info!("Attempting to read configuration from serial port {}", SERIAL_DEVICE);

    for attempt in 1..=MAX_RETRIES {
        match try_read_serial_config(SERIAL_DEVICE) {
            Ok(config) => {
                info!("Successfully read serial config: IP={}, Port={}", config.ip, config.port);
                return Ok(config);
            }
            Err(e) => {
                debug!("Attempt {}/{} failed: {}", attempt, MAX_RETRIES, e);
                if attempt < MAX_RETRIES {
                    sleep(RETRY_DELAY).await;
                }
            }
        }
    }

    Err(anyhow::anyhow!(
        "Failed to read serial configuration after {} attempts", 
        MAX_RETRIES
    ))
}

fn try_read_serial_config(device_path: &str) -> Result<SerialConfig> {
    let file = File::open(device_path)
        .with_context(|| format!("Failed to open serial device {}", device_path))?;
    
    let mut reader = BufReader::new(file);
    let mut buffer = String::new();

    loop {
        buffer.clear();
        let bytes_read = reader.read_line(&mut buffer)
            .context("Failed to read from serial device")?;
        
        if bytes_read == 0 {
            return Err(anyhow::anyhow!("Reached EOF without finding configuration"));
        }

        let line = buffer.trim();
        debug!("Serial line received: {}", line);

        // Expected format: IP:192.168.100.10:3001
        if line.starts_with("IP:") {
            return parse_ip_config(line);
        }
    }
}

fn parse_ip_config(line: &str) -> Result<SerialConfig> {
    let parts: Vec<&str> = line.split(':').collect();
    
    if parts.len() != 3 {
        return Err(anyhow::anyhow!(
            "Invalid IP configuration format. Expected 'IP:address:port', got '{}'", 
            line
        ));
    }

    if parts[0] != "IP" {
        return Err(anyhow::anyhow!(
            "Configuration line must start with 'IP:', got '{}'", 
            parts[0]
        ));
    }

    let ip = parts[1].to_string();
    let port = parts[2].parse::<u16>()
        .with_context(|| format!("Invalid port number: {}", parts[2]))?;

    // Validation basique de l'IP
    if ip.is_empty() {
        return Err(anyhow::anyhow!("IP address cannot be empty"));
    }

    Ok(SerialConfig::new(ip, port))
}

#[cfg(test)]
pub async fn read_serial_config_from_file(file_path: &str) -> Result<SerialConfig> {
    try_read_serial_config(file_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_parse_ip_config_valid() {
        let config = parse_ip_config("IP:192.168.100.10:3001").unwrap();
        assert_eq!(config.ip, "192.168.100.10");
        assert_eq!(config.port, 3001);
    }

    #[test]
    fn test_parse_ip_config_invalid_format() {
        assert!(parse_ip_config("IP:192.168.100.10").is_err());
        assert!(parse_ip_config("INVALID:192.168.100.10:3001").is_err());
        assert!(parse_ip_config("IP::3001").is_err());
        assert!(parse_ip_config("IP:192.168.100.10:invalid").is_err());
    }

    #[tokio::test]
    async fn test_read_serial_config_from_file() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "Some kernel log").unwrap();
        writeln!(temp_file, "IP:192.168.100.20:3002").unwrap();
        temp_file.flush().unwrap();

        let config = read_serial_config_from_file(temp_file.path().to_str().unwrap()).await.unwrap();
        assert_eq!(config.ip, "192.168.100.20");
        assert_eq!(config.port, 3002);
    }

    #[tokio::test]
    async fn test_read_serial_config_no_ip_found() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "Some log line").unwrap();
        writeln!(temp_file, "Another log line").unwrap();
        temp_file.flush().unwrap();

        drop(temp_file);
        
        let result = read_serial_config_from_file("/tmp/empty_test_file").await;
        assert!(result.is_err());
    }
}