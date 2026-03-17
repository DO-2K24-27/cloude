use clap::{Parser, Subcommand};
use std::{
    env,
    path::PathBuf,
    process::{Command, Stdio},
};

#[derive(Parser)]
#[command(name = "cargo xtask", about = "Build and run cloude")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build all binaries: backend, cli, and agent (musl target)
    Build,
    /// Start backend with proper environment variables
    RunBackend,
    /// Run CLI commands
    RunCli {
        /// Arguments to pass to the CLI (use -- to separate)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Clean initramfs cache
    ResetInitramfs,
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Commands::Build => build(),
        Commands::RunBackend => run_backend(),
        Commands::RunCli { args } => run_cli(args),
        Commands::ResetInitramfs => reset_initramfs(),
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn build() -> Result<(), Box<dyn std::error::Error>> {
    println!("Building backend and cli...");
    let status = Command::new("cargo")
        .current_dir(project_root())
        .args(["build", "-p", "backend", "-p", "cli"])
        .status()?;
    if !status.success() {
        return Err("Failed to build backend and cli".into());
    }

    println!("Building agent (musl target)...");
    let status = Command::new("cargo")
        .current_dir(project_root())
        .args([
            "build",
            "-p",
            "agent",
            "--target",
            "x86_64-unknown-linux-musl",
        ])
        .status()?;
    if !status.success() {
        return Err("Failed to build agent".into());
    }

    println!("Copying agent binary...");
    let agent_src = project_root().join("target/x86_64-unknown-linux-musl/debug/agent");
    let agent_dst = project_root().join("backend/cloude-agentd");

    std::fs::copy(&agent_src, &agent_dst)?;

    println!("Making agent binary executable...");
    let status = Command::new("chmod").arg("+x").arg(&agent_dst).status()?;
    if !status.success() {
        return Err("Failed to make agent binary executable".into());
    }

    println!("✓ Build complete!");
    Ok(())
}

fn run_backend() -> Result<(), Box<dyn std::error::Error>> {
    println!("Starting backend (requires root privileges)...");

    let backend_path = project_root().join("target/debug/backend");

    if !backend_path.exists() {
        return Err("Backend binary not found. Run `cargo xtask build` first.".into());
    }

    let env_path = project_root().join("backend/.env");
    if env_path.exists() {
        dotenvy::from_path(&env_path).ok();
    }

    let status = Command::new("sudo")
        .arg("env")
        .arg(format!(
            "PATH=/usr/sbin:{}",
            env::var("PATH").unwrap_or_default()
        ))
        .arg(format!(
            "LANGUAGES_CONFIG_PATH={}",
            env::var("LANGUAGES_CONFIG_PATH")
                .unwrap_or_else(|_| "./config/languages.json".to_string())
        ))
        .arg(format!(
            "AGENT_BINARY_PATH={}",
            env::var("AGENT_BINARY_PATH").unwrap_or_else(|_| "./cloude-agentd".to_string())
        ))
        .arg(format!(
            "INIT_SCRIPT_PATH={}",
            env::var("INIT_SCRIPT_PATH").unwrap_or_else(|_| "./init.sh".to_string())
        ))
        .arg(format!(
            "VM_KERNEL_PATH={}",
            env::var("VM_KERNEL_PATH").unwrap_or_else(|_| "./vmlinux".to_string())
        ))
        .arg(format!(
            "VM_INITRAMFS_DIR={}",
            env::var("VM_INITRAMFS_DIR").unwrap_or_else(|_| "./tmp".to_string())
        ))
        .arg(format!(
            "VM_LOG_GUEST_CONSOLE={}",
            env::var("VM_LOG_GUEST_CONSOLE").unwrap_or_else(|_| "false".to_string())
        ))
        .arg(backend_path)
        .current_dir(project_root().join("backend"))
        .status()?;

    if !status.success() {
        return Err("Backend exited with error".into());
    }

    Ok(())
}

fn run_cli(args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    println!("Running CLI with args: {:?}", args);

    let mut cmd = Command::new("cargo")
        .current_dir(project_root())
        .args(["run", "-p", "cli", "--"])
        .args(&args)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?;

    let status = cmd.wait()?;

    if !status.success() {
        return Err("CLI command failed".into());
    }

    Ok(())
}

fn reset_initramfs() -> Result<(), Box<dyn std::error::Error>> {
    println!("Cleaning initramfs cache...");

    let tmp_dir = project_root().join("backend/tmp");
    if tmp_dir.exists() {
        let entries = std::fs::read_dir(&tmp_dir)?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("cpio.gz") {
                println!("Removing: {:?}", path);
                std::fs::remove_file(&path)?;
            }
        }
    } else {
        println!("tmp directory does not exist, nothing to clean.");
    }

    println!("✓ Initramfs cache cleared. Restart backend to regenerate.");
    Ok(())
}
