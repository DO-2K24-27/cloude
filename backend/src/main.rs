use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use backend::initramfs_manager::get_languages_config;
use backend::ip_manager::IpManager;
use backend::vm_lifecycle::{VmConfig, VmHandle};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::{error, info};
use tracing_subscriber::{self, EnvFilter};
use virt::network::{setup_bridge, setup_nat};

// ── Shared application state ────────────────────────────────────────

struct AppState {
    jobs: RwLock<HashMap<String, Job>>,
    client: reqwest::Client,
    supported_languages: Vec<backend::initramfs_manager::InitramfsLanguage>,
    vm_config: VmConfig,
    ip_manager: Arc<Mutex<IpManager>>,
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
    let bridge_name = env::var("BRIDGE_NAME").unwrap_or_else(|_| "cloudebr0".to_string());

    let languages_config_path =
        env::var("LANGUAGES_CONFIG_PATH").unwrap_or_else(|_| "./config/languages.json".to_string());

    let agent_binary =
        env::var("AGENT_BINARY_PATH").unwrap_or_else(|_| "./cloude-agentd".to_string());

    let init_script = env::var("INIT_SCRIPT_PATH").unwrap_or_else(|_| "./init.sh".to_string());
    let vm_initramfs_dir =
        env::var("VM_INITRAMFS_DIR").unwrap_or_else(|_| "./tmp".to_string());

    let available_languages: Vec<backend::initramfs_manager::InitramfsLanguage> =
        get_languages_config(&languages_config_path)?;

    for language in available_languages.clone() {
        log::debug!("Available language: {}", language.name);
        log::debug!("  version: {}", language.version);
        log::debug!("  base_image: {}", language.base_image);

        let lang_name = language.name.clone();
        language
            .setup_initramfs(&agent_binary, &init_script, &vm_initramfs_dir)
            .await
            .map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Failed to setup initramfs for {}: {}", lang_name, e),
                )
            })?;
    }

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

    if !(1..=30).contains(&ip_mask) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "IP_MASK must be in range 1..=30 to reserve gateway and guest addresses, got {}",
                ip_mask
            ),
        ));
    }

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

    // Build a shared HTTP client with a timeout for agent calls
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .expect("Failed to build HTTP client");

    let vm_kernel_path =
        env::var("VM_KERNEL_PATH").unwrap_or_else(|_| "./vmlinux".to_string());
    let vm_log_guest_console = env::var("VM_LOG_GUEST_CONSOLE")
        .map(|v| {
            let normalized = v.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(false);
    tokio::fs::create_dir_all(&vm_initramfs_dir).await?;

    let ip_allocations_path =
        env::var("IP_ALLOCATIONS_PATH").unwrap_or_else(|_| "./tmp/ip_allocations.json".to_string());
    if let Some(parent) = PathBuf::from(&ip_allocations_path).parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let host_bits = 32_u32.checked_sub(u32::from(ip_mask)).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("Failed to compute host bits from IP_MASK={}", ip_mask),
        )
    })?;
    let host_space = 1_u32.checked_shl(host_bits).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("Failed to compute host address space from IP_MASK={}", ip_mask),
        )
    })?;
    let broadcast_offset = host_space.checked_sub(1).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("Failed to compute broadcast offset from IP_MASK={}", ip_mask),
        )
    })?;
    let ip_range_u32 = u32::from(ip_range);
    let pool_start_u32 = ip_range_u32.checked_add(2).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("IP_RANGE {} overflows when computing pool start", ip_range),
        )
    })?;
    let pool_end_u32 = ip_range_u32
        .checked_add(broadcast_offset)
        .and_then(|v| v.checked_sub(1))
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("IP_RANGE {} overflows when computing pool end", ip_range),
            )
        })?;
    if pool_start_u32 > pool_end_u32 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "Invalid pool bounds for IP_RANGE={} and IP_MASK={}",
                ip_range, ip_mask
            ),
        ));
    }
    let pool_start: Ipv4Addr = pool_start_u32.into();
    let pool_end: Ipv4Addr = pool_end_u32.into();
    let ip_manager = Arc::new(Mutex::new(
        IpManager::new(&ip_allocations_path, pool_start, pool_end).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to initialize IP manager: {}", e),
            )
        })?,
    ));

    let state = Arc::new(AppState {
        jobs: RwLock::new(HashMap::new()),
        client,
        supported_languages: available_languages.clone(),
        vm_config: VmConfig {
            kernel_path: PathBuf::from(vm_kernel_path),
            initramfs_dir: PathBuf::from(vm_initramfs_dir),
            bridge_name: bridge_name.clone(),
            vcpus: 1,
            memory_mb: 512,
            log_guest_console: vm_log_guest_console,
        },
        ip_manager,
    });

    // Background task: evict terminal jobs older than 5 mins to prevent unbounded memory growth.
    const JOB_TTL: std::time::Duration = std::time::Duration::from_secs(300);
    let cleanup_state = Arc::clone(&state);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            let mut jobs = cleanup_state.jobs.write().await;
            let before = jobs.len();
            jobs.retain(|_, j| {
                !matches!(j.status, JobStatus::Done | JobStatus::Error)
                    || j.created_at.elapsed() < JOB_TTL
            });
            let removed = before - jobs.len();
            if removed > 0 {
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
) -> axum::response::Response {
    let requested_language = payload.language.trim().to_ascii_lowercase();
    let language = normalize_language_alias(&requested_language);

    let mut supported_languages = state
        .supported_languages
        .iter()
        .map(|lang| lang.name.to_ascii_lowercase())
        .collect::<Vec<_>>();
    supported_languages.sort();
    supported_languages.dedup();
    if payload.code.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Code cannot be empty"
            })),
        )
            .into_response();
    }

    let code = payload.code.clone();

    if !supported_languages.iter().any(|name| name == &language) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!(
                    "Unsupported language: {}. Supported languages: {}",
                    payload.language,
                    supported_languages.join(", ")
                )
            })),
        )
            .into_response();
    }

    let id = uuid::Uuid::new_v4().to_string();

    let job = Job {
        id: id.clone(),
        status: JobStatus::Pending,
        language: language.clone(),
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

    info!("Job {} created – language={}", id, language);

    // Spawn a background task that creates a VM and forwards the request to its agent
    let job_id = id.clone();
    let language = language.clone();
    let code = code.clone();
    let state = Arc::clone(&state);

    tokio::spawn(async move {
        // Mark as running
        {
            let mut jobs = state.jobs.write().await;
            if let Some(j) = jobs.get_mut(&job_id) {
                j.status = JobStatus::Running;
            }
        }

        let mut vm = match VmHandle::create(
            job_id.clone(),
            &language,
            &state.vm_config,
            Arc::clone(&state.ip_manager),
        )
        .await
        {
            Ok(vm) => vm,
            Err(e) => {
                let mut jobs = state.jobs.write().await;
                if let Some(j) = jobs.get_mut(&job_id) {
                    j.status = JobStatus::Error;
                    j.stderr = Some(format!("Failed to create VM: {e}"));
                }
                error!("Job {} – failed to create VM: {}", job_id, e);
                return;
            }
        };

        let execute_url = format!("{}/execute", vm.agent_url().trim_end_matches('/'));
        let request_payload = AgentExecuteRequest { language, code };

        let mut execution_result: Result<AgentExecuteResponse, String> =
            Err("VM agent execute request did not run".to_string());

        for attempt in 1..=5 {
            let result = state
                .client
                .post(&execute_url)
                .json(&request_payload)
                .send()
                .await;

            match result {
                Ok(resp) if resp.status().is_success() => {
                    execution_result = resp
                        .json::<AgentExecuteResponse>()
                        .await
                        .map_err(|e| format!("Failed to parse agent response: {e}"));
                    break;
                }
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    execution_result = Err(format!("Agent returned HTTP {status}: {body}"));
                    break;
                }
                Err(e) => {
                    if attempt == 5 {
                        execution_result = Err(format!("Cannot reach VM agent: {e}"));
                        break;
                    }

                    info!(
                        "Job {} – execute call failed on attempt {}/5, retrying: {}",
                        job_id, attempt, e
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                }
            }
        }

        let mut jobs = state.jobs.write().await;
        match execution_result {
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
                    j.stderr = Some(e.clone());
                }
                error!("Job {} – execution failed: {}", job_id, e);
            }
        }

        // Teardown after job state is finalized so polling clients are never stuck in "running"
        // if VM shutdown blocks longer than expected.
        drop(jobs);
        vm.destroy().await;
    });

    (StatusCode::ACCEPTED, Json(RunResponse { id })).into_response()
}

fn normalize_language_alias(input: &str) -> String {
    match input {
        "py" => "python".to_string(),
        "js" | "javascript" => "node".to_string(),
        "rs" => "rust".to_string(),
        "golang" => "go".to_string(),
        "c++" => "cpp".to_string(),
        _ => input.to_string(),
    }
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
