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
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::{Duration, timeout};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

const MAX_OUTPUT_BYTES: usize = 1024 * 1024;

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

struct PreparedJob {
    job_dir: PathBuf,
    source_path: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let server_addr =
        env::var("AGENT_SERVER_ADDR").unwrap_or_else(|_| "0.0.0.0:3001".to_string());
    let work_dir = resolve_work_dir(PathBuf::from(
        env::var("AGENT_WORK_DIR").unwrap_or_else(|_| "build".to_string()),
    ))?;
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
            return error_response(
                StatusCode::BAD_REQUEST,
                format!("Unsupported language: {}", payload.language),
            );
        }
    };

    let prepared_job = match prepare_job(
        &state.work_dir,
        &job_id,
        runtime.source_extension(),
        payload.code,
    )
    .await
    {
        Ok(prepared_job) => prepared_job,
        Err((job_dir, error)) => {
            if let Some(job_dir) = job_dir {
                schedule_job_cleanup(job_dir);
            }
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, error);
        }
    };

    let result = match execute_job(
        runtime.as_ref(),
        &prepared_job.source_path,
        &prepared_job.job_dir,
        state.exec_timeout,
    )
    .await
    {
        Ok(result) => result,
        Err(e) => {
            schedule_job_cleanup(prepared_job.job_dir.clone());
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
        }
    };

    schedule_job_cleanup(prepared_job.job_dir);

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

fn error_response(status: StatusCode, error: String) -> axum::response::Response {
    (status, Json(ErrorResponse { error })).into_response()
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
        Err(e) => Err(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Execution lock error: {}", e),
        )),
    }
}

async fn prepare_job(
    work_dir: &Path,
    job_id: &str,
    source_extension: &str,
    code: String,
) -> std::result::Result<PreparedJob, (Option<PathBuf>, String)> {
    let job_dir = work_dir.join(job_id);

    tokio::fs::create_dir_all(&job_dir)
        .await
        .map_err(|e| (None, format!("Failed to create job dir: {}", e)))?;

    let source_path = job_dir.join(format!("code.{}", source_extension));
    tokio::fs::write(&source_path, code).await.map_err(|e| {
        (
            Some(job_dir.clone()),
            format!("Failed to write source code: {}", e),
        )
    })?;

    Ok(PreparedJob {
        job_dir,
        source_path,
    })
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
        match run_process(program, args, work_dir, exec_timeout).await {
            Ok(result) => return Ok(result),
            Err(err) if err.downcast_ref::<std::io::Error>().is_some() => {
                last_error = Some((program.clone(), err))
            }
            Err(err) => return Err(err),
        }
    }

    let (program, err) = last_error.context("No execution command candidate provided")?;
    Err(err).with_context(|| format!("Failed to spawn process: {}", program))
}

async fn run_process(
    program: &str,
    args: &[String],
    work_dir: &Path,
    exec_timeout: Duration,
) -> Result<ExecutionResult> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(work_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd
        .spawn()
        .with_context(|| format!("Failed to spawn process: {}", program))?;
    let stdout = child.stdout.take().context("Child stdout was not piped")?;
    let stderr = child.stderr.take().context("Child stderr was not piped")?;
    let (tx, mut rx) = mpsc::channel(2);

    let stdout_task = tokio::spawn(read_stream_limited(stdout, StreamKind::Stdout, tx.clone()));
    let stderr_task = tokio::spawn(read_stream_limited(stderr, StreamKind::Stderr, tx));
    let mut recv_closed = false;

    let status = timeout(exec_timeout, async {
        loop {
            tokio::select! {
                stream_result = rx.recv(), if !recv_closed => {
                    match stream_result {
                        Some(StreamResult::Exceeded(kind)) => {
                            child.kill().await.with_context(|| {
                                format!("Failed to kill process after exceeding {} output limit: {}", kind.label(), program)
                            })?;
                        }
                        // Reader tasks finished (EOF): this is expected for short-lived commands.
                        // Stop polling the channel to avoid busy-looping on repeated `None`.
                        None => {
                            recv_closed = true;
                        }
                    }
                }
                status = child.wait() => {
                    break status.with_context(|| {
                        format!("Process failed while waiting for output: {}", program)
                    });
                }
            }
        }
    })
    .await
    .with_context(|| {
        format!(
            "Process timed out after {}s: {}",
            exec_timeout.as_secs(),
            program
        )
    })??;

    let stdout = stdout_task
        .await
        .context("Failed to join stdout reader task")?
        .with_context(|| format!("Failed to read stdout for: {}", program))?;
    let stderr = stderr_task
        .await
        .context("Failed to join stderr reader task")?
        .with_context(|| format!("Failed to read stderr for: {}", program))?;

    Ok(ExecutionResult {
        exit_code: status.code().unwrap_or(1),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    })
}

fn resolve_work_dir(path: PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path);
    }

    Ok(env::current_dir()?.join(path))
}

#[derive(Clone, Copy)]
enum StreamKind {
    Stdout,
    Stderr,
}

impl StreamKind {
    fn label(self) -> &'static str {
        match self {
            StreamKind::Stdout => "stdout",
            StreamKind::Stderr => "stderr",
        }
    }
}

enum StreamResult {
    Exceeded(StreamKind),
}

async fn read_stream_limited<R>(
    mut reader: R,
    kind: StreamKind,
    tx: mpsc::Sender<StreamResult>,
) -> Result<Vec<u8>>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut output = Vec::new();
    let mut chunk = [0_u8; 8192];

    loop {
        let read = reader
            .read(&mut chunk)
            .await
            .with_context(|| format!("Failed reading {}", kind.label()))?;

        if read == 0 {
            return Ok(output);
        }

        let remaining = MAX_OUTPUT_BYTES.saturating_sub(output.len());
        let to_copy = remaining.min(read);
        output.extend_from_slice(&chunk[..to_copy]);

        if read > remaining {
            let _ = tx.send(StreamResult::Exceeded(kind)).await;
            return Ok(output);
        }
    }
}
