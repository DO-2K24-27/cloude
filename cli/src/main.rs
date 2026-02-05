use clap::Parser;

/// VM Configuration CLI
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Number of CPUs to allocate to the VM
    #[arg(short = 'c', long)]
    cpu: u8,

    /// Amount of RAM in GB to allocate to the VM
    #[arg(short = 'r', long)]
    ram: u32,

    /// Path to the kernel image file
    #[arg(short = 'k', long)]
    kernel: String,

    /// Path to the initramfs image file
    #[arg(short = 'i', long)]
    initramfs: String,

    /// Path to the disk image file
    #[arg(short = 'f', long)]
    file: String,
}

fn main() {
    let args = Args::parse();

    println!("CPU: {}", args.cpu);
    println!("RAM: {} GB", args.ram);
    println!("Kernel: {}", args.kernel);
    println!("Initramfs: {}", args.initramfs);
    println!("File: {}", args.file);
}
