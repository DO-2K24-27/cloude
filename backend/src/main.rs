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
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::{error, info};
use tracing_subscriber::{self, EnvFilter};
use virt::network::{setup_bridge, setup_nat};

// ── Shared application state ────────────────────────────────────────

struct AppState {
    jobs: RwLock<HashMap<String, Job>>,
    agent_url: String,
    client: reqwest::Client,
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
    if let Err(e) = setup_bridge(bridge_name, host_ip, ip_mask).await {
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

    let state = Arc::new(AppState {
        jobs: RwLock::new(HashMap::new()),
        agent_url,
        client,
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

    // Spawn a background task that forwards the request to the agent
    let job_id = id.clone();
    let language = payload.language.clone();
    let code = payload.code.clone();
    let state = Arc::clone(&state);

    tokio::spawn(async move {
        // Mark as running
        {
            let mut jobs = state.jobs.write().await;
            if let Some(j) = jobs.get_mut(&job_id) {
                j.status = JobStatus::Running;
            }
        }

        let result = state
            .client
            .post(format!("{}/execute", state.agent_url.trim_end_matches('/')))
            .json(&AgentExecuteRequest { language, code })
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
