use agent::runtimes::{LanguageRuntime, runtime_from_language};
use anyhow::{Context, Result};
use axum::{
    Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::get,
    routing::post,
};
use serde::{Deserialize, Serialize};
use std::env;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::{Duration, timeout};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

struct AppState {
    job_counter: AtomicU64,
    run_limit: Arc<Semaphore>,
    work_dir: PathBuf,
    exec_timeout: Duration,
}

#[derive(Debug, Deserialize)]
struct ExecuteRequest {
    language: String,
    code: String,
}

#[derive(Debug, Serialize)]
struct ExecuteResponse {
    job_id: String,
    exit_code: i32,
    stdout: String,
    stderr: String,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

struct ExecutionResult {
    exit_code: i32,
    stdout: String,
    stderr: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let server_addr =
        env::var("AGENT_SERVER_ADDR").unwrap_or_else(|_| "127.0.0.1:3001".to_string());
    let work_dir =
        PathBuf::from(env::var("AGENT_WORK_DIR").unwrap_or_else(|_| "build".to_string()));
    let timeout_secs = env::var("AGENT_EXEC_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(30);

    let state = Arc::new(AppState {
        job_counter: AtomicU64::new(1),
        run_limit: Arc::new(Semaphore::new(1)),
        work_dir,
        exec_timeout: Duration::from_secs(timeout_secs),
    });

    let app = Router::new()
        .route("/health", get(health))
        .route("/execute", post(execute))
        .with_state(state);

    info!("Starting agent server on {}", server_addr);
    let listener = TcpListener::bind(&server_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> &'static str {
    "ok"
}

async fn execute(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ExecuteRequest>,
) -> impl IntoResponse {
    let id = state.job_counter.fetch_add(1, Ordering::Relaxed);
    let job_id = format!("job-{}", id);
    let _permit = match acquire_run_permit(&state, &job_id).await {
        Ok(permit) => permit,
        Err(response) => return response,
    };

    let runtime = match runtime_from_language(&payload.language) {
        Some(runtime) => runtime,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Unsupported language: {}", payload.language),
                }),
            )
                .into_response();
        }
    };

    let job_dir = state.work_dir.join(&job_id);

    if let Err(e) = tokio::fs::create_dir_all(&job_dir).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to create job dir: {}", e),
            }),
        )
            .into_response();
    }

    let source_path = job_dir.join(format!("code.{}", runtime.source_extension()));
    if let Err(e) = tokio::fs::write(&source_path, payload.code).await {
        schedule_job_cleanup(job_dir.clone());
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to write source code: {}", e),
            }),
        )
            .into_response();
    }

    let result =
        match execute_job(runtime.as_ref(), &source_path, &job_dir, state.exec_timeout).await {
            Ok(result) => result,
            Err(e) => {
                schedule_job_cleanup(job_dir.clone());
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: e.to_string(),
                    }),
                )
                    .into_response();
            }
        };

    schedule_job_cleanup(job_dir);

    (
        StatusCode::OK,
        Json(ExecuteResponse {
            job_id,
            exit_code: result.exit_code,
            stdout: result.stdout,
            stderr: result.stderr,
        }),
    )
        .into_response()
}

fn schedule_job_cleanup(job_dir: PathBuf) {
    tokio::spawn(async move {
        if let Err(err) = tokio::fs::remove_dir_all(&job_dir).await {
            warn!(path = %job_dir.display(), error = %err, "Failed to remove job directory");
        }
    });
}

async fn acquire_run_permit(
    state: &Arc<AppState>,
    job_id: &str,
) -> std::result::Result<OwnedSemaphorePermit, axum::response::Response> {
    info!(job_id = %job_id, "Waiting for run permit");
    let run_limit = Arc::clone(&state.run_limit);
    match run_limit.acquire_owned().await {
        Ok(permit) => {
            info!(job_id = %job_id, "Acquired run permit");
            Ok(permit)
        }
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Execution lock error: {}", e),
            }),
        )
            .into_response()),
    }
}

async fn execute_job(
    runtime: &dyn LanguageRuntime,
    source_path: &Path,
    work_dir: &Path,
    exec_timeout: Duration,
) -> Result<ExecutionResult> {
    if let Some(commands) = runtime.compile_candidates(source_path, work_dir) {
        let compile_result = run_process_candidates(&commands, work_dir, exec_timeout).await?;
        if compile_result.exit_code != 0 {
            return Ok(compile_result);
        }
    }

    run_process_candidates(
        &runtime.run_candidates(source_path, work_dir),
        work_dir,
        exec_timeout,
    )
    .await
}

async fn run_process_candidates(
    commands: &[(String, Vec<String>)],
    work_dir: &Path,
    exec_timeout: Duration,
) -> Result<ExecutionResult> {
    let mut last_error = None;

    for (program, args) in commands {
        let mut cmd = Command::new(program);
        cmd.args(args)
            .current_dir(work_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        match cmd.spawn() {
            Ok(child) => {
                let output = timeout(exec_timeout, child.wait_with_output())
                    .await
                    .with_context(|| {
                        format!(
                            "Process timed out after {}s: {}",
                            exec_timeout.as_secs(),
                            program
                        )
                    })?
                    .with_context(|| {
                        format!("Process failed while waiting for output: {}", program)
                    })?;

                return Ok(ExecutionResult {
                    exit_code: output.status.code().unwrap_or(1),
                    stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                });
            }
            Err(err) => last_error = Some((program.clone(), err)),
        }
    }

    let (program, err) = last_error.context("No execution command candidate provided")?;
    Err(err).with_context(|| format!("Failed to spawn process: {}", program))
}
