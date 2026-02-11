// Usage:
// KERNEL_PATH=/path/to/kernel INITRAMFS_PATH=/path/to/initramfs cargo run --bin test
// SERIAL_OUTPUT=/path/to/output.log - optional, to capture serial output

use std::{env, os::fd::AsRawFd};
use vmm::{VMInput, VMM};
use vmm_sys_util::terminal::Terminal;

#[derive(Debug)]
pub enum Error {
    VmmNew(vmm::Error),

    VmmKernel(env::VarError),

    VmmConfigure(vmm::Error),

    VmmRun(vmm::Error),
}

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

    VMM::new(stdin_box, writer).map_err(Error::VmmNew)
}

fn configure_vmm(
    mut vmm: VMM,
    vcpus: u8,
    memory: u32,
    kernel_path: &str,
    initramfs_path: &str,
) -> Result<VMM, Error> {
    vmm.configure(vcpus, memory, kernel_path, initramfs_path)
        .map_err(Error::VmmConfigure)?;

    Ok(vmm)
}

fn start_vmm(mut vmm: VMM) -> Result<(), Error> {
    vmm.run();

    Ok(())
}
