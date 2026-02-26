use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

pub struct ExecutionResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub struct QemuRunner {
    kernel_path: PathBuf,
}

impl QemuRunner {
    pub fn new<P: AsRef<Path>>(kernel_path: P) -> Self {
        Self {
            kernel_path: kernel_path.as_ref().to_path_buf(),
        }
    }

    pub async fn run_initramfs(&self, initramfs_path: &Path) -> Result<ExecutionResult> {
        let mut child = Command::new("qemu-system-x86_64")
            .arg("-kernel")
            .arg(&self.kernel_path)
            .arg("-initrd")
            .arg(initramfs_path)
            .arg("-append")
            .arg("console=ttyS0 quiet panic=-1")
            .arg("-m")
            .arg("512M")
            .arg("-nographic")
            .arg("-no-reboot")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to spawn QEMU. Is it installed?")?;

        let stdout = child.stdout.take().expect("Failed to open QEMU stdout");
        let mut reader = BufReader::new(stdout).lines();

        let mut captured_output = String::new();
        let mut is_capturing = false;
        let mut exit_code = 127;

        while let Some(line) = reader.next_line().await? {
            if line.contains("--- PROGRAM OUTPUT ---") {
                is_capturing = true;
                continue;
            }
            if line.contains("--- END OUTPUT ---") {
                is_capturing = false;
                continue;
            }
            if line.starts_with("Exit code:") {
                let code_str = line.trim_start_matches("Exit code: ").trim();
                if let Ok(code) = code_str.parse::<i32>() {
                    exit_code = code;
                }
                continue;
            }

            if is_capturing {
                captured_output.push_str(&line);
                captured_output.push('\n');
            }
        }

        let wait_future = child.wait();
        let _status = match tokio::time::timeout(std::time::Duration::from_secs(30), wait_future).await {
            Ok(result) => result?,
            Err(_) => {
                let _ = child.kill().await;
                return Err(anyhow::anyhow!("QEMU execution timed out after 30 seconds"));
            }
        };
        Ok(ExecutionResult {
            exit_code,
            stdout: captured_output,
            stderr: String::new(),
        })
    }
}
