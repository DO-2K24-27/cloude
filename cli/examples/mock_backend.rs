//! Mock backend server – use this to test the CLI without a real agent/VM.
//!
//! Run it with:
//!   cargo run --example mock_backend -p cli
//!
//! Then in another terminal try:
//!   cargo run -p cli -- go --language python --file agent/examples/hello.py
//!   cargo run -p cli -- status <id>
//!
//! Special pre-built job IDs you can query directly:
//!   cargo run -p cli -- status pending-job   → always "pending"
//!   cargo run -p cli -- status running-job   → always "running"
//!   cargo run -p cli -- status done-job      → "done" with stdout
//!   cargo run -p cli -- status error-job     → "error" with stderr
//!   cargo run -p cli -- status 404-unknown   → 404 not found

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tokio::time::Instant;

// ── State ────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct Job {
    id: String,
    language: String,
    code_preview: String, // first 80 chars of submitted code
    created_at: Instant,
    /// Simulated delay before the job finishes (seconds)
    finish_after_secs: u64,
}

type Store = Arc<RwLock<HashMap<String, Job>>>;

// ── DTOs ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct RunRequest {
    language: String,
    code: String,
}

#[derive(Serialize)]
struct RunResponse {
    id: String,
}

// ── Static jobs (always available, no submission needed) ─────────────

fn static_status(id: &str) -> Option<serde_json::Value> {
    match id {
        "pending-job" => Some(json!({
            "id": id,
            "status": "pending",
            "exit_code": null,
            "stdout": null,
            "stderr": null,
        })),
        "running-job" => Some(json!({
            "id": id,
            "status": "running",
            "exit_code": null,
            "stdout": null,
            "stderr": null,
        })),
        "done-job" => Some(json!({
            "id": id,
            "status": "done",
            "exit_code": 0,
            "stdout": "Hello from the mock VM!\nAll done.\n",
            "stderr": null,
        })),
        "error-job" => Some(json!({
            "id": id,
            "status": "error",
            "exit_code": 1,
            "stdout": null,
            "stderr": "RuntimeError: something went wrong inside the VM\n",
        })),
        "crash-job" => Some(json!({
            "id": id,
            "status": "error",
            "exit_code": 137,
            "stdout": "partial output before crash\n",
            "stderr": "Killed (signal 9)\n",
        })),
        _ => None,
    }
}

// ── Handlers ─────────────────────────────────────────────────────────

async fn root() -> &'static str {
    "Mock backend is running – see examples/mock_backend.rs for usage."
}

async fn health() -> &'static str {
    "ok"
}

async fn run_job(State(store): State<Store>, Json(payload): Json<RunRequest>) -> impl IntoResponse {
    // Generate a short readable ID
    let id = format!(
        "mock-{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos()
    );

    let preview = payload.code.chars().take(80).collect::<String>();
    let finish_after_secs = 3;

    println!(
        "[mock] POST /run  → id={id}  language={}  finish_in={finish_after_secs}s",
        payload.language
    );
    println!("[mock]   code preview: {preview:?}");

    let job = Job {
        id: id.clone(),
        language: payload.language,
        code_preview: preview,
        created_at: Instant::now(),
        finish_after_secs,
    };

    store.write().await.insert(id.clone(), job);

    (StatusCode::ACCEPTED, Json(RunResponse { id }))
}

async fn get_status(State(store): State<Store>, Path(id): Path<String>) -> impl IntoResponse {
    println!("[mock] GET /status/{id}");

    // Static pre-built scenarios
    if let Some(v) = static_status(&id) {
        return (StatusCode::OK, Json(v));
    }

    // Dynamic jobs submitted via POST /run
    let store = store.read().await;
    match store.get(&id) {
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("Job {id} not found") })),
        ),
        Some(job) => {
            let elapsed = job.created_at.elapsed().as_secs();

            let body = if elapsed < 1 {
                json!({
                    "id": job.id,
                    "status": "pending",
                    "exit_code": null,
                    "stdout": null,
                    "stderr": null,
                })
            } else if elapsed < job.finish_after_secs {
                json!({
                    "id": job.id,
                    "status": "running",
                    "exit_code": null,
                    "stdout": null,
                    "stderr": null,
                })
            } else {
                let stdout = mock_stdout(&job.language, &job.code_preview);
                json!({
                    "id": job.id,
                    "status": "done",
                    "exit_code": 0,
                    "stdout": stdout,
                    "stderr": null,
                })
            };

            (StatusCode::OK, Json(body))
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────

/// Fake stdout based on the submitted code
fn mock_stdout(language: &str, code_preview: &str) -> String {
    let hint = if code_preview.contains("print") || code_preview.contains("console.log") {
        "Hello, World!\n"
    } else {
        "(no output)\n"
    };
    format!("[mock VM – language={language}]\n{hint}--- execution finished ---\n")
}

// ── Entry point ───────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let addr = std::env::var("MOCK_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    let store: Store = Arc::new(RwLock::new(HashMap::new()));

    let app = Router::new()
        .route("/", get(root))
        .route("/health", get(health))
        .route("/run", post(run_job))
        .route("/status/{id}", get(get_status))
        .with_state(store);

    println!("╔══════════════════════════════════════════════╗");
    println!("║         Cloude mock backend running          ║");
    println!("╠══════════════════════════════════════════════╣");
    println!("║  Listening on  http://{addr:<22} ║");
    println!("╠══════════════════════════════════════════════╣");
    println!("║  Static test IDs (no submission needed):     ║");
    println!("║    pending-job  running-job  done-job        ║");
    println!("║    error-job    crash-job                    ║");
    println!("╠══════════════════════════════════════════════╣");
    println!("║  Dynamic jobs simulate:                      ║");
    println!("║    0s → pending                              ║");
    println!("║    1s → running                              ║");
    println!("║    3s → done                                 ║");
    println!("╚══════════════════════════════════════════════╝");
    println!();

    let listener = TcpListener::bind(&addr).await.expect("Cannot bind address");
    axum::serve(listener, app).await.expect("Server error");
}
