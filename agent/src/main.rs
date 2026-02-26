use agent::builder::image::Builder;
use agent::qemu::QemuRunner;
use anyhow::Result;
use std::env;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: {} <kernel_path> <code_file>", args[0]);
        std::process::exit(1);
    }

    let kernel_path = PathBuf::from(&args[1]);
    let code_file = PathBuf::from(&args[2]);

    if !code_file.exists() {
        eprintln!("Code file not found: {:?}", code_file);
        std::process::exit(1);
    }

    let runtime = match agent::runtimes::detect_runtime(&code_file) {
        Some(rt) => rt,
        None => {
            eprintln!("Unsupported file extension or language for {:?}", code_file);
            std::process::exit(1);
        }
    };

    let work_dir = PathBuf::from("build");
    let builder = Builder::new(&work_dir);

    println!("Building initramfs for {}...", runtime.base_image());
    let initramfs_path = builder.build_image(runtime.as_ref(), &code_file).await?;
    println!("Initramfs built at {:?}", initramfs_path);

    println!("Booting QEMU...");
    let runner = QemuRunner::new(kernel_path);
    let result = runner.run_initramfs(&initramfs_path).await?;

    println!("\n=== EXECUTION RESULT ===");
    println!("Exit code: {}", result.exit_code);
    println!("--- STDOUT ---");
    println!("{}", result.stdout);
    if !result.stderr.is_empty() {
        println!("--- STDERR ---");
        println!("{}", result.stderr);
    }
    println!("========================");

    Ok(())
}
