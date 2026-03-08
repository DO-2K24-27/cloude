// Usage:
// KERNEL_PATH=/path/to/kernel INITRAMFS_PATH=/path/to/initramfs cargo run --bin test
// SERIAL_OUTPUT=/path/to/output.log - optional, to capture serial output
// TAP_DEVICE=<device_name> - optional, to enable networking with a specific tap device
// GUEST_IP=<ip_address> - optional, guest IP address
// HOST_IP=<ip_address> - optional, host IP address
// NETMASK=<mask> - optional, network mask

use std::env;
use tracing_subscriber::EnvFilter;
use vmm::{VMInput, VMM};
use vmm_sys_util::terminal::Terminal;

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
        let guest_ip = env::var("GUEST_IP").ok();
        let host_ip = env::var("HOST_IP").ok();
        let netmask = env::var("NETMASK").ok();

        if let Err(e) = vmm.add_net_device(
            tap_name.clone(),
            guest_ip.as_deref(),
            host_ip.as_deref(),
            netmask.as_deref(),
        ) {
            return eprintln!("Error adding net device: {:?}", e);
        }

        // If an host IP is set, setup the bridge for it
        if let Some(host_ip) = host_ip.as_deref() {
            virt::network::setup_bridge("cloudebrtest".to_string(), host_ip.parse().unwrap(), 24)
                .await
                .expect("Failed to set up bridge");

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
