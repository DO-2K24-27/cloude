use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::{error, info, warn};
use tracing_subscriber::{self, EnvFilter};
use virt::network::{setup_bridge, setup_nat};

use backend::ip_manager::IpManager;

// ── Shared application state ────────────────────────────────────────

struct AppState {
    jobs: RwLock<HashMap<String, Job>>,
    /// Live VM handles keyed by job_id.
    vm_handles: RwLock<HashMap<String, virt::provision::VmHandle>>,
    agent_url: String,
    client: reqwest::Client,
    kernel_path: String,
    /// language → pre-built initramfs path
    supported_languages: HashMap<String, PathBuf>,
    ip_manager: Arc<IpManager>,
    bridge_name: String,
    /// Bridge / gateway IP seen by guest VMs
    host_ip: Ipv4Addr,
    ip_mask: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum JobStatus {
    Pending,
    Running,
    Done,
    Error,
}

#[derive(Clone, Debug, Serialize)]
struct Job {
    id: String,
    status: JobStatus,
    language: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stderr: Option<String>,
    #[serde(skip)]
    created_at: std::time::Instant,
}

// ── Request / Response DTOs ─────────────────────────────────────────

#[derive(Deserialize)]
struct RunRequest {
    language: String,
    code: String,
}

#[derive(Serialize)]
struct RunResponse {
    id: String,
}

// ── Agent DTOs (for forwarding to the agent) ────────────────────────

#[derive(Serialize)]
struct AgentExecuteRequest {
    language: String,
    code: String,
}

#[derive(Deserialize)]
struct AgentExecuteResponse {
    #[allow(dead_code)]
    job_id: String,
    exit_code: i32,
    stdout: String,
    stderr: String,
}

// ── Main ────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    // init logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    log::debug!("Debug logging enabled");

    // Get the server address from the environment variable or use a default
    let server_addr =
        env::var("BACKEND_SERVER_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    let agent_url = env::var("AGENT_URL").unwrap_or_else(|_| "http://127.0.0.1:3001".to_string());
    let bridge_name = env::var("BRIDGE_NAME").unwrap_or_else(|_| "cloudebr0".to_string());

    let kernel_path = env::var("KERNEL_PATH").map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "KERNEL_PATH env variable is required",
        )
    })?;

    // SUPPORTED_LANGUAGES=python:./tmp/python-3.12.cpio.gz,node:./tmp/node-20.cpio.gz
    let supported_languages =
        parse_supported_languages(&env::var("SUPPORTED_LANGUAGES").unwrap_or_default());

    // 39 is miku
    let ip_range: Ipv4Addr = env::var("IP_RANGE")
        .as_deref()
        .unwrap_or_else(|_| "10.39.1.0")
        .parse()
        .map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("IP_RANGE env variable is invalid: {}", e),
            )
        })?;
    let ip_mask: u8 = env::var("IP_MASK")
        .unwrap_or_else(|_| "24".to_string())
        .parse()
        .map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("IP_MASK env variable is invalid: {}", e),
            )
        })?;

    // NOTE, DO NOT MERGE UNTIL REMOVAL OF THIS COMMENT:
    // I think using TWO WHOLE crates only to create the interface and tell it to do postrouting/ip forwarding may be a lot.
    // An alternative would be to use ioctl (?) or just run a command.
    // Please give me feedback, this is making me go crazy.

    // Set up the bridge and NAT rules
    let host_ip: Ipv4Addr = (ip_range.to_bits() + 1).into();
    if let Err(e) = setup_bridge(bridge_name.clone(), host_ip, ip_mask).await {
        eprintln!("Failed to set up bridge: {}", e);
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            e.to_string(),
        ));
    }

    if let Err(e) = setup_nat(ip_range, ip_mask) {
        eprintln!("Failed to set up NAT: {}", e);
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            e.to_string(),
        ));
    }

    // Guest VMs are allocated from ip_range + 2 onwards (ip_range + 1 is the bridge)
    let guest_start: Ipv4Addr = (ip_range.to_bits() + 2).into();
    let guest_end: Ipv4Addr = (ip_range.to_bits() + 254).into();

    let ip_manager_file =
        env::var("IP_MANAGER_FILE").unwrap_or_else(|_| "/tmp/cloude-ip-manager.json".to_string());
    let ip_manager = Arc::new(
        IpManager::new(&ip_manager_file, guest_start, guest_end).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to create IP manager: {}", e),
            )
        })?,
    );

    // Build a shared HTTP client with a timeout for agent calls
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .expect("Failed to build HTTP client");

    let state = Arc::new(AppState {
        jobs: RwLock::new(HashMap::new()),
        vm_handles: RwLock::new(HashMap::new()),
        agent_url,
        client,
        kernel_path,
        supported_languages,
        ip_manager,
        bridge_name,
        host_ip,
        ip_mask,
    });

    // Background task: evict terminal jobs older than 5 mins to prevent unbounded memory growth.
    const JOB_TTL: std::time::Duration = std::time::Duration::from_secs(300);
    let cleanup_state = Arc::clone(&state);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            let expired_ids: Vec<String> = {
                let jobs = cleanup_state.jobs.read().await;
                jobs.iter()
                    .filter(|(_, j)| {
                        matches!(j.status, JobStatus::Done | JobStatus::Error)
                            && j.created_at.elapsed() >= JOB_TTL
                    })
                    .map(|(id, _)| id.clone())
                    .collect()
            };
            if expired_ids.is_empty() {
                continue;
            }
            {
                let mut handles = cleanup_state.vm_handles.write().await;
                for id in &expired_ids {
                    if let Some(mut h) = handles.remove(id) {
                        h.stop();
                    }
                    let _ = cleanup_state.ip_manager.release_ip(id);
                }
            }
            {
                let mut jobs = cleanup_state.jobs.write().await;
                let removed = expired_ids.len();
                for id in &expired_ids {
                    jobs.remove(id);
                }
                info!("Evicted {} expired jobs", removed);
            }
        }
    });

    let app = Router::new()
        .route("/", get(root))
        .route("/health", get(health_check))
        .route("/run", post(run_job))
        .route("/status/{id}", get(get_status))
        .with_state(state);

    info!("Starting Backend server on {}", &server_addr);
    let listener = TcpListener::bind(&server_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn root() -> &'static str {
    "Welcome to the Backend server!"
}

async fn health_check() -> &'static str {
    "Backend server is healthy!"
}

// ── POST /run  –  submit a new job ──────────────────────────────────

async fn run_job(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<RunRequest>,
) -> impl IntoResponse {
    let id = uuid::Uuid::new_v4().to_string();

    let job = Job {
        id: id.clone(),
        status: JobStatus::Pending,
        language: payload.language.clone(),
        exit_code: None,
        stdout: None,
        stderr: None,
        created_at: std::time::Instant::now(),
    };

    // Store the job
    {
        let mut jobs = state.jobs.write().await;
        jobs.insert(id.clone(), job);
    }

    info!("Job {} created – language={}", id, payload.language);

    // Spawn a background task that provisions a VM and forwards the request to the agent
    let job_id = id.clone();
    let language = payload.language.clone();
    let code = payload.code.clone();
    let state = Arc::clone(&state);

    tokio::spawn(async move {
        // ── 1. Look up the initramfs for the requested language ──────
        let initramfs_path = match state.supported_languages.get(&language) {
            Some(p) => p.clone(),
            None => {
                error!("Job {} – unsupported language: {}", job_id, language);
                let mut jobs = state.jobs.write().await;
                if let Some(j) = jobs.get_mut(&job_id) {
                    j.status = JobStatus::Error;
                    j.stderr = Some(format!("Unsupported language: {}", language));
                }
                return;
            }
        };

        // ── 2. Load initramfs (verify it exists) ─────────────────────
        if !initramfs_path.exists() {
            error!("Job {} – initramfs not found: {:?}", job_id, initramfs_path);
            let mut jobs = state.jobs.write().await;
            if let Some(j) = jobs.get_mut(&job_id) {
                j.status = JobStatus::Error;
                j.stderr = Some(format!("Initramfs not found: {:?}", initramfs_path));
            }
            return;
        }

        // Allocate a guest IP and build a unique TAP name (max 15 chars)
        let guest_ip: Ipv4Addr = match state.ip_manager.allocate_ip(&job_id) {
            Ok(s) => match s.parse::<Ipv4Addr>() {
                Ok(ip) => ip,
                Err(e) => {
                    error!("Job {} – invalid allocated IP: {}", job_id, e);
                    let mut jobs = state.jobs.write().await;
                    if let Some(j) = jobs.get_mut(&job_id) {
                        j.status = JobStatus::Error;
                        j.stderr = Some(format!("Invalid allocated IP: {}", e));
                    }
                    return;
                }
            },
            Err(e) => {
                error!("Job {} – IP allocation failed: {}", job_id, e);
                let mut jobs = state.jobs.write().await;
                if let Some(j) = jobs.get_mut(&job_id) {
                    j.status = JobStatus::Error;
                    j.stderr = Some(format!("IP allocation failed: {}", e));
                }
                return;
            }
        };

        let tap_name = format!(
            "ctap{}",
            job_id.replace('-', "").chars().take(11).collect::<String>()
        );

        let netmask: Ipv4Addr = if state.ip_mask == 0 {
            Ipv4Addr::new(0, 0, 0, 0)
        } else {
            (u32::MAX << (32 - state.ip_mask as u32)).into()
        };

        // ── 3. Launch VM with the correct initramfs ──────────────────
        let vm_handle = match virt::provision::spawn_vm(virt::provision::VmConfig {
            kernel_path: PathBuf::from(&state.kernel_path),
            initramfs_path,
            tap_name: tap_name.clone(),
            guest_ip,
            host_ip: state.host_ip,
            netmask,
            memory: 512 << 20,
            vcpus: 1,
            log_path: Some(PathBuf::from(format!(
                "/tmp/cloude-vm-{}.log",
                &job_id[..8]
            ))),
        }) {
            Ok(h) => h,
            Err(e) => {
                let _ = state.ip_manager.release_ip(&job_id);
                error!("Job {} – VM spawn failed: {}", job_id, e);
                let mut jobs = state.jobs.write().await;
                if let Some(j) = jobs.get_mut(&job_id) {
                    j.status = JobStatus::Error;
                    j.stderr = Some(format!("VM spawn failed: {}", e));
                }
                return;
            }
        };

        // Attach the TAP device to the bridge
        let net_err = virt::network::setup_guest_iface(&tap_name, &state.bridge_name)
            .await
            .err()
            .map(|e| e.to_string());
        if let Some(msg) = net_err {
            let _ = state.ip_manager.release_ip(&job_id);
            // vm_handle dropped here stops the VM
            error!("Job {} – network setup failed: {}", job_id, msg);
            let mut jobs = state.jobs.write().await;
            if let Some(j) = jobs.get_mut(&job_id) {
                j.status = JobStatus::Error;
                j.stderr = Some(format!("Network setup failed: {}", msg));
            }
            return;
        }

        // ── 4. Mark as running ───────────────────────────────────────
        {
            let mut jobs = state.jobs.write().await;
            if let Some(j) = jobs.get_mut(&job_id) {
                j.status = JobStatus::Running;
            }
        }

        // ── 5. Store VM handle for cleanup ───────────────────────────
        {
            let mut handles = state.vm_handles.write().await;
            handles.insert(job_id.clone(), vm_handle);
        }

        info!(
            "Job {} – VM provisioned (guest_ip={}, tap={})",
            job_id, guest_ip, tap_name
        );

        // Forward the request to the agent running inside the VM
        let agent_url = format!("http://{}:3001", guest_ip);
        let result = state
            .client
            .post(format!("{}/execute", agent_url.trim_end_matches('/')))
            .json(&AgentExecuteRequest {
                language: language.clone(),
                code,
            })
            .send()
            .await;

        let mut jobs = state.jobs.write().await;
        match result {
            Ok(resp) if resp.status().is_success() => {
                match resp.json::<AgentExecuteResponse>().await {
                    Ok(agent_resp) => {
                        if let Some(j) = jobs.get_mut(&job_id) {
                            j.status = JobStatus::Done;
                            j.exit_code = Some(agent_resp.exit_code);
                            j.stdout = Some(agent_resp.stdout);
                            j.stderr = Some(agent_resp.stderr);
                        }
                        info!("Job {} completed", job_id);
                    }
                    Err(e) => {
                        if let Some(j) = jobs.get_mut(&job_id) {
                            j.status = JobStatus::Error;
                            j.stderr = Some(format!("Failed to parse agent response: {e}"));
                        }
                        error!("Job {} – agent response parse error: {e}", job_id);
                    }
                }
            }
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                if let Some(j) = jobs.get_mut(&job_id) {
                    j.status = JobStatus::Error;
                    j.stderr = Some(format!("Agent returned HTTP {status}: {body}"));
                }
                error!("Job {} – agent error HTTP {status}", job_id);
            }
            Err(e) => {
                if let Some(j) = jobs.get_mut(&job_id) {
                    j.status = JobStatus::Error;
                    j.stderr = Some(format!("Cannot reach agent: {e}"));
                }
                error!("Job {} – cannot reach agent: {e}", job_id);
            }
        }
        drop(jobs);

        // Stop the VM and release its IP
        {
            let mut handles = state.vm_handles.write().await;
            if let Some(mut h) = handles.remove(&job_id) {
                h.stop();
            }
        }
        if let Err(e) = state.ip_manager.release_ip(&job_id) {
            warn!("Job {} – failed to release IP: {}", job_id, e);
        }
    });

    (StatusCode::ACCEPTED, Json(RunResponse { id }))
}

// ── GET /status/:id  –  query job result ────────────────────────────

async fn get_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let jobs = state.jobs.read().await;

    match jobs.get(&id) {
        Some(job) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": job.id,
                "status": job.status,
                "exit_code": job.exit_code,
                "stdout": job.stdout,
                "stderr": job.stderr,
            })),
        ),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": format!("Job {id} not found"),
            })),
        ),
    }
}

// ── Helpers ──────────────────────────────────────────────────────────

/// Parse "lang:path,lang2:path2,..." into a HashMap.
fn parse_supported_languages(raw: &str) -> HashMap<String, PathBuf> {
    raw.split(',')
        .filter_map(|entry| {
            let mut parts = entry.splitn(2, ':');
            let lang = parts.next()?.trim();
            let path = parts.next()?.trim();
            if lang.is_empty() || path.is_empty() {
                return None;
            }
            Some((lang.to_string(), PathBuf::from(path)))
        })
        .collect()
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_supported_languages_valid() {
        let map = parse_supported_languages(
            "python:./tmp/python-3.12.cpio.gz,node:./tmp/node-20.cpio.gz",
        );
        assert_eq!(
            map.get("python"),
            Some(&PathBuf::from("./tmp/python-3.12.cpio.gz"))
        );
        assert_eq!(
            map.get("node"),
            Some(&PathBuf::from("./tmp/node-20.cpio.gz"))
        );
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn test_parse_supported_languages_empty() {
        let map = parse_supported_languages("");
        assert!(map.is_empty());
    }

    #[test]
    fn test_parse_supported_languages_ignores_malformed() {
        let map = parse_supported_languages(
            "python:./tmp/python.cpio.gz,broken,node:./tmp/node.cpio.gz",
        );
        assert_eq!(map.len(), 2);
        assert!(map.contains_key("python"));
        assert!(map.contains_key("node"));
    }

    #[test]
    fn test_tap_name_length() {
        // TAP device names must be <= 15 chars
        let job_id = uuid::Uuid::new_v4().to_string();
        let tap_name = format!(
            "ctap{}",
            job_id.replace('-', "").chars().take(11).collect::<String>()
        );
        assert!(
            tap_name.len() <= 15,
            "tap name '{}' is {} chars",
            tap_name,
            tap_name.len()
        );
    }

    #[test]
    fn test_netmask_from_prefix() {
        let mask: Ipv4Addr = (u32::MAX << (32 - 24u32)).into();
        assert_eq!(mask, Ipv4Addr::new(255, 255, 255, 0));

        let mask16: Ipv4Addr = (u32::MAX << (32 - 16u32)).into();
        assert_eq!(mask16, Ipv4Addr::new(255, 255, 0, 0));
    }
}
