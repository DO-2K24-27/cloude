// Usage:
// KERNEL_PATH=/path/to/kernel INITRAMFS_PATH=/path/to/initramfs cargo run --bin test
// SERIAL_OUTPUT=/path/to/output.log - optional, to capture serial output

use std::env;
use vmm::{VMInput, VMM};
use vmm_sys_util::terminal::Terminal;

fn main() {
    let kernel_path = match env::var("KERNEL_PATH") {
        Ok(val) => val,
        Err(e) => return eprintln!("Error getting KERNEL_PATH: {}", e),
    };

    let initramfs_path = match env::var("INITRAMFS_PATH") {
        Ok(val) => val,
        Err(e) => return eprintln!("Error getting INITRAMFS_PATH: {}", e),
    };

    let vcpus: u8 = 2;
    let memory: u32 = 1024;

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
    let mut vmm = match VMM::new(stdin_box, writer) {
        Ok(v) => v,
        Err(e) => return eprintln!("Error creating VMM: {:?}", e),
    };

    // Add network device if enabled
    if env::var("ENABLE_VIRTIO_NET").is_ok() {
        let tap_name = env::var("TAP_DEVICE").ok();
        if let Err(e) = vmm.add_net_device(tap_name.as_deref()) {
            return eprintln!("Error adding net device: {:?}", e);
        }
    }

    // Configure VMM
    if let Err(e) = vmm.configure(vcpus, memory, &kernel_path, &initramfs_path) {
        return eprintln!("Error configuring VMM: {:?}", e);
    }

    // Run VMM
    vmm.run();
}
