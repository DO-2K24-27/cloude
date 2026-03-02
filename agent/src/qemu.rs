use anyhow::{Context, Result};
use std::env;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::{Duration, timeout};

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
        let timeout_secs = env::var("AGENT_QEMU_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(120);
        let timeout_duration = Duration::from_secs(timeout_secs);

        let mut cmd = Command::new("qemu-system-x86_64");
        cmd.arg("-kernel")
            .arg(&self.kernel_path)
            .arg("-initrd")
            .arg(initramfs_path)
            .arg("-append")
            .arg("console=ttyS0 panic=1 reboot=t")
            .arg("-m")
            .arg("512M")
            .arg("-nographic")
            .arg("-no-reboot")
            .arg("-device")
            .arg("virtio-rng-pci")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        let mut child = cmd
            .spawn()
            .context("Failed to spawn QEMU. Is it installed?")?;

        let stdout = child.stdout.take().expect("Failed to open QEMU stdout");
        let mut reader = BufReader::new(stdout).lines();

        let mut captured_output = String::new();
        let mut is_capturing = false;
        let mut exit_code = 127;
        let mut seen_exit_code = false;

        let read_result = timeout(timeout_duration, async {
            loop {
                let Some(line) = reader.next_line().await? else {
                    break;
                };

                if line.contains("--- PROGRAM OUTPUT ---") {
                    is_capturing = true;
                    continue;
                }
                if line.contains("--- END OUTPUT ---") {
                    is_capturing = false;
                    continue;
                }
                if line.starts_with("Exit code:") {
                    let code_str = line.trim_start_matches("Exit code:").trim();
                    if let Ok(code) = code_str.parse::<i32>() {
                        exit_code = code;
                        seen_exit_code = true;
                    }
                    break;
                }

                if is_capturing {
                    if is_kernel_log_line(&line) {
                        continue;
                    }
                    captured_output.push_str(&line);
                    captured_output.push('\n');
                }
            }
            Ok::<(), anyhow::Error>(())
        })
        .await;

        match read_result {
            Ok(result) => result?,
            Err(_) => {
                let _ = child.kill().await;
                return Err(anyhow::anyhow!(
                    "QEMU execution timed out after {} seconds",
                    timeout_duration.as_secs()
                ));
            }
        }

        if seen_exit_code {
            let _ = child.kill().await;
        } else {
            let _ = timeout(timeout_duration, child.wait()).await;
        }

        Ok(ExecutionResult {
            exit_code,
            stdout: captured_output,
            stderr: String::new(),
        })
    }
}

fn is_kernel_log_line(line: &str) -> bool {
    let s = line.trim_start();
    if !s.starts_with('[') {
        return false;
    }
    match s.find(']') {
        Some(i) => i > 2,
        None => false,
    }
}
