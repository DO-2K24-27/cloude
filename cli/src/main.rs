use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Cloude CLI – run code in micro-VMs
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(short, long, default_value = "http://127.0.0.1:8080")]
    backend: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Send a source file
    Go {
        /// Programming language (python, javascript, rust, …)
        #[arg(short, long)]
        language: String,
        /// Source file to run
        #[arg(short, long)]
        file: PathBuf,
    },

    /// Query the status / result of a job
    Status {
        /// Job ID
        id: String,
    },
}

// ── Shared DTOs (mirror backend) ────────────────────────────────────

#[derive(Serialize)]
struct RunRequest {
    language: String,
    code: String,
}

#[derive(Deserialize)]
struct RunResponse {
    id: String,
}

#[derive(Deserialize)]
struct StatusResponse {
    id: String,
    status: String,
    #[serde(default)]
    exit_code: Option<i32>,
    #[serde(default)]
    stdout: Option<String>,
    #[serde(default)]
    stderr: Option<String>,
}

#[derive(Deserialize, Debug)]
struct ErrorBody {
    error: String,
}

// ── Main ────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let backend = cli.backend.trim_end_matches('/').to_string();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("Failed to build HTTP client");

    match cli.command {
        Commands::Go { language, file } => {
            if let Err(e) = cmd_go(&client, &backend, &language, &file).await {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }
        Commands::Status { id } => {
            if let Err(e) = cmd_status(&client, &backend, &id).await {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }
    }
}

// ── go: send code to backend ────────────────────────────────────────

async fn cmd_go(
    client: &reqwest::Client,
    backend: &str,
    language: &str,
    file: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let code = std::fs::read_to_string(file)
        .map_err(|e| format!("Cannot read file {}: {e}", file.display()))?;

    let url = format!("{backend}/run");
    let body = RunRequest {
        language: language.to_string(),
        code,
    };

    let resp = client.post(&url).json(&body).send().await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err: ErrorBody = resp.json().await.unwrap_or(ErrorBody {
            error: format!("HTTP {status}"),
        });
        return Err(format!("Backend error (HTTP {status}): {}", err.error).into());
    }

    let run: RunResponse = resp.json().await?;
    let job_id = run.id.clone();

    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;

        let status_url = format!("{backend}/status/{job_id}");
        let status_resp = client.get(&status_url).send().await?;

        if !status_resp.status().is_success() {
            let status = status_resp.status();
            let err: ErrorBody = status_resp
                .json()
                .await
                .unwrap_or(ErrorBody {
                    error: format!("HTTP {status}"),
                });
            return Err(format!("Backend error (HTTP {status}): {}", err.error).into());
        }

        let st: StatusResponse = status_resp.json().await?;

        if st.status == "done" || st.status == "error" {
            println!("Status: {}", st.status);
            if let Some(code) = st.exit_code {
                println!("Exit code: {code}");
            }
            if let Some(ref out) = st.stdout {
                if !out.is_empty() {
                    println!("{out}");
                }
            }
            if let Some(ref err) = st.stderr {
                if !err.is_empty() {
                    println!("{err}");
                }
            }
            return Ok(());
        }
    }
}

// ── status: query job result ────────────────────────────────────────

async fn cmd_status(
    client: &reqwest::Client,
    backend: &str,
    id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let url = format!("{backend}/status/{id}");
    let resp = client.get(&url).send().await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err: ErrorBody = resp.json().await.unwrap_or(ErrorBody {
            error: format!("HTTP {status}"),
        });
        return Err(format!("Backend error (HTTP {status}): {}", err.error).into());
    }

    let st: StatusResponse = resp.json().await?;

    println!("Job ID: {}", st.id);
    println!("Status: {}", st.status);
    if let Some(code) = st.exit_code {
        println!("Exit code: {code}");
    }
    Ok(())
}
