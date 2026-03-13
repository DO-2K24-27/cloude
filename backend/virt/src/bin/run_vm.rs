// Usage:
// KERNEL_PATH=/path/to/kernel INITRAMFS_PATH=/path/to/initramfs cargo run --bin test
// SERIAL_OUTPUT=/path/to/output.log - optional, to capture serial output
// TAP_DEVICE=<device_name> - optional, to enable networking with a specific tap device
// GUEST_IP=<ip_address> - optional, guest IP address
// HOST_IP=<ip_address> - optional, host IP address
// NETMASK=<mask> - optional, network mask

use std::{env, net::Ipv4Addr};
use tracing_subscriber::EnvFilter;
use vmm::{VMInput, VMM};
use vmm_sys_util::terminal::Terminal;

/// Check if IPv4 are in the same subnet
fn same_subnet(ip1: Ipv4Addr, ip2: Ipv4Addr, prefix_len: u8) -> bool {
    let mask = !0u32 << (32 - prefix_len);
    (u32::from(ip1) & mask) == (u32::from(ip2) & mask)
}

fn get_env_ip(var_name: &str) -> Result<Option<Ipv4Addr>, std::io::Error> {
    match env::var(var_name) {
        Ok(val) => val.parse().map(Some).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("{} env variable is unvalid: {}", var_name, e),
            )
        }),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(e) => Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Error getting {}: {}", var_name, e),
        )),
    }
}
#[tokio::main]
async fn main() {
    // init logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    log::debug!("Debug logging enabled");

    let kernel_path = match env::var("KERNEL_PATH") {
        Ok(val) => val,
        Err(e) => return eprintln!("Error getting KERNEL_PATH: {}", e),
    };

    let initramfs_path = match env::var("INITRAMFS_PATH") {
        Ok(val) => val,
        Err(e) => return eprintln!("Error getting INITRAMFS_PATH: {}", e),
    };

    let vcpus: u8 = 2;
    let memory: usize = 1024 << 20; // convert from 1024 MB to bytes

    // Configure serial output
    let writer: Box<dyn std::io::Write + Send> =
        if let Ok(serial_output) = env::var("SERIAL_OUTPUT") {
            println!("Serial output will be written to: {}", serial_output);
            Box::new(
                std::fs::File::create(&serial_output).expect("Failed to create serial output file"),
            )
        } else {
            Box::new(std::io::stdout())
        };

    // Configure stdin in raw mode
    let stdin = std::io::stdin();
    let stdin_lock: std::io::StdinLock<'_> = stdin.lock();
    stdin_lock
        .set_raw_mode()
        .expect("Failed to set stdin to raw mode");
    let stdin_box: Box<dyn VMInput> = Box::new(stdin_lock);

    // Create VMM
    let mut vmm = match VMM::new(stdin_box, writer, memory) {
        Ok(v) => v,
        Err(e) => return eprintln!("Error creating VMM: {:?}", e),
    };

    // Add network device if enabled
    if let Some(tap_name) = env::var("TAP_DEVICE").ok() {
        let guest_ip = get_env_ip("GUEST_IP").unwrap();
        let host_ip = get_env_ip("HOST_IP").unwrap();
        let netmask = get_env_ip("NETMASK").unwrap(); // in the form 255.255.255.0

        if let Err(e) = vmm.add_net_device(tap_name.clone(), guest_ip, host_ip, netmask) {
            return eprintln!("Error adding net device: {:?}", e);
        }

        // If an host IP is set, setup the bridge for it
        if let (Some(guest_ip), Some(host_ip), Some(netmask)) = (guest_ip, host_ip, netmask) {
            virt::network::setup_bridge("cloudebrtest".to_string(), host_ip, 24)
                .await
                .expect("Failed to set up bridge");

            let prefix = u32::from(netmask).leading_ones() as u8;

            if !same_subnet(guest_ip, host_ip, prefix) {
                return eprintln!("Error: Guest IP and Host IP are not in the same subnet");
            }

            let network = virt::network::network_addr(guest_ip, prefix).expect("network should be setup");
            virt::network::setup_nat(network, prefix).expect("Failed to set up NAT");

            virt::network::setup_guest_iface(&tap_name, "cloudebrtest")
                .await
                .expect("Failed to set up guest network interface");
        }
    }

    let init_path = env::var("INIT_PATH").ok();
    // Configure VMM
    if let Err(e) = vmm.configure(vcpus, &kernel_path, &initramfs_path, init_path.as_deref()) {
        return eprintln!("Error configuring VMM: {:?}", e);
    }

    // Run VMM
    vmm.run();
}
