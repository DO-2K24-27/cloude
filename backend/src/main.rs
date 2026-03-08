use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
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
use tracing::{Level, error, info};
use tracing_subscriber;

// ── Shared application state ────────────────────────────────────────

struct AppState {
    jobs: RwLock<HashMap<String, Job>>,
    client: reqwest::Client,
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
    // Initialize tracing subscriber for logging
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();

    // Get configuration from environment variables
    let server_addr =
        env::var("BACKEND_SERVER_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    
    let kernel_path = PathBuf::from(
        env::var("BACKEND_KERNEL_PATH")
            .expect("BACKEND_KERNEL_PATH environment variable must be set")
    );
    
    if !kernel_path.exists() {
        eprintln!("Kernel not found at {:?}", kernel_path);
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Kernel file not found"
        ));
    }

    let work_dir = PathBuf::from(
        env::var("BACKEND_WORK_DIR").unwrap_or_else(|_| "backend_vms".to_string())
    );
    tokio::fs::create_dir_all(&work_dir).await?;

    let bridge_name = env::var("BRIDGE_NAME").unwrap_or_else(|_| "cloudebr0".to_string());
    
    let ip_start: Ipv4Addr = env::var("IP_RANGE_START")
        .unwrap_or_else(|_| "10.39.1.10".to_string())
        .parse()
        .map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Invalid IP_RANGE_START: {}", e),
            )
        })?;
    
    let ip_end: Ipv4Addr = env::var("IP_RANGE_END")
        .unwrap_or_else(|_| "10.39.1.250".to_string())
        .parse()
        .map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Invalid IP_RANGE_END: {}", e),
            )
        })?;

    let ip_file = env::var("IP_ALLOCATIONS_FILE")
        .unwrap_or_else(|_| "ip_allocations.json".to_string());

    // Initialize IP manager
    let ip_manager = IpManager::new(&ip_file, ip_start, ip_end)
        .map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to initialize IP manager: {}", e),
            )
        })?;

    info!(
        "Initialized IP manager with range {}-{}",
        ip_start, ip_end
    );

    // Build a shared HTTP client with a timeout for agent calls
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .expect("Failed to build HTTP client");

    let vm_config = VmConfig {
        kernel_path,
        work_dir,
        bridge_name,
        vcpus: 2,
        memory_mb: 1024,
    };

    let state = Arc::new(AppState {
        jobs: RwLock::new(HashMap::new()),
        client,
        vm_config,
        ip_manager: Arc::new(Mutex::new(ip_manager)),
    });

    // Background task: evict terminal jobs older than 5mins
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

    // Start the server
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

    // Spawn a background task that creates a VM, executes code, and cleans up
    let job_id = id.clone();
    let language = payload.language.clone();
    let code = payload.code.clone();
    let state_clone = Arc::clone(&state);

    tokio::spawn(async move {
        // Mark as running
        {
            let mut jobs = state_clone.jobs.write().await;
            if let Some(j) = jobs.get_mut(&job_id) {
                j.status = JobStatus::Running;
            }
        }

        // Create VM for this job
        info!("Job {} – Creating VM", job_id);
        let mut vm = match VmHandle::create(
            job_id.clone(),
            &state_clone.vm_config,
            Arc::clone(&state_clone.ip_manager),
        )
        .await
        {
            Ok(vm) => {
                info!("Job {} – VM created with IP {}", job_id, vm.ip);
                vm
            }
            Err(e) => {
                error!("Job {} – Failed to create VM: {}", job_id, e);
                let mut jobs = state_clone.jobs.write().await;
                if let Some(j) = jobs.get_mut(&job_id) {
                    j.status = JobStatus::Error;
                    j.stderr = Some(format!("Failed to create VM: {}", e));
                }
                return;
            }
        };

        // Send code execution request to agent in the VM
        let agent_url = format!("{}/execute", vm.agent_url());
        info!("Job {} – Sending code to agent at {}", job_id, agent_url);
        
        let result = state_clone
            .client
            .post(&agent_url)
            .json(&AgentExecuteRequest {
                language: language.clone(),
                code: code.clone(),
            })
            .send()
            .await;

        // Process result and update job status
        let mut jobs = state_clone.jobs.write().await;
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
                        info!("Job {} completed successfully", job_id);
                    }
                    Err(e) => {
                        if let Some(j) = jobs.get_mut(&job_id) {
                            j.status = JobStatus::Error;
                            j.stderr = Some(format!("Failed to parse agent response: {}", e));
                        }
                        error!("Job {} – agent response parse error: {}", job_id, e);
                    }
                }
            }
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                if let Some(j) = jobs.get_mut(&job_id) {
                    j.status = JobStatus::Error;
                    j.stderr = Some(format!("Agent returned HTTP {}: {}", status, body));
                }
                error!("Job {} – agent error HTTP {}", job_id, status);
            }
            Err(e) => {
                if let Some(j) = jobs.get_mut(&job_id) {
                    j.status = JobStatus::Error;
                    j.stderr = Some(format!("Cannot reach agent in VM: {}", e));
                }
                error!("Job {} – cannot reach agent: {}", job_id, e);
            }
        }
        
        // Release the lock before destroying VM (which can take time)
        drop(jobs);

        // Cleanup: destroy the VM
        info!("Job {} – Destroying VM", job_id);
        vm.destroy().await;
        info!("Job {} – VM destroyed", job_id);
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
