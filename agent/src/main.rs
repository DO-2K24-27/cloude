use agent::builder::image::Builder;
use agent::qemu::QemuRunner;
use agent::runtimes::{LanguageRuntime, runtime_from_language};
use agent::serial::read_serial_config;
use anyhow::Result;
use axum::{
    Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::get,
    routing::post,
};
use serde::{Deserialize, Serialize};
use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::net::TcpListener;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::{Level, info};

struct AppState {
    job_counter: AtomicU64,
    run_limit: Arc<Semaphore>,
    work_dir: PathBuf,
    kernel_path: PathBuf,
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

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();

    info!("Agent starting - reading IP configuration from serial port");
    let serial_config = read_serial_config().await?;
    let server_addr = format!("{}:{}", serial_config.ip, serial_config.port);

    let work_dir =
        PathBuf::from(env::var("AGENT_WORK_DIR").unwrap_or_else(|_| "build".to_string()));
    let kernel_path = resolve_kernel_path()?;
    if !kernel_path.exists() {
        anyhow::bail!(
            "Configured kernel path does not exist: {:?}. Set AGENT_KERNEL_PATH correctly.",
            kernel_path
        );
    }

    let state = Arc::new(AppState {
        job_counter: AtomicU64::new(1),
        run_limit: Arc::new(Semaphore::new(1)),
        work_dir,
        kernel_path,
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
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to write source code: {}", e),
            }),
        )
            .into_response();
    }

    let kernel_path = state.kernel_path.clone();
    let result = match execute_job(&kernel_path, runtime.as_ref(), &source_path, &job_dir).await {
        Ok(result) => result,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
                .into_response();
        }
    };

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

fn resolve_kernel_path() -> Result<PathBuf> {
    if let Ok(path) = env::var("AGENT_KERNEL_PATH") {
        return Ok(PathBuf::from(path));
    }

    anyhow::bail!("Missing AGENT_KERNEL_PATH. Configure a server-side kernel path before start.")
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
    kernel_path: &Path,
    runtime: &dyn LanguageRuntime,
    code_file: &Path,
    work_dir: &Path,
) -> Result<agent::qemu::ExecutionResult> {
    if !kernel_path.exists() {
        anyhow::bail!("Kernel not found: {:?}", kernel_path);
    }

    let builder = Builder::new(work_dir);

    info!(
        "Building initramfs for runtime {}",
        runtime.source_extension()
    );
    let initramfs_path = builder.build_image(runtime, code_file).await?;

    info!(
        "Booting QEMU for {:?} with kernel {:?} and initramfs {:?}",
        code_file, kernel_path, initramfs_path
    );
    let runner = QemuRunner::new(kernel_path);
    runner.run_initramfs(&initramfs_path).await
}
