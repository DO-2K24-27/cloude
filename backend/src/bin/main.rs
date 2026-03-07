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
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::{Level, error, info};
use tracing_subscriber;

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

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Job {
    id: String,
    status: JobStatus,
    language: String,
    #[serde(skip_serializing)]
    #[allow(dead_code)]
    code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stderr: Option<String>,
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
    // Get the server address from the environment variable or use a default
    let server_addr =
        env::var("BACKEND_SERVER_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    let agent_url = env::var("AGENT_URL").unwrap_or_else(|_| "http://127.0.0.1:3001".to_string());

    // Initialize tracing subscriber for logging
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();

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
        code: payload.code.clone(),
        exit_code: None,
        stdout: None,
        stderr: None,
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
            .post(format!("{}/execute", state.agent_url))
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
