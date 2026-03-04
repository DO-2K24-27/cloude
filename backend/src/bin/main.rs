use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use backend::vm_manager::VmManager;
use serde::{Deserialize, Serialize};
use std::env;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{Level, info};
use tracing_subscriber;

#[derive(Debug, Deserialize)]
struct ExecuteRequest {
    language: String,
    code: String,
}

#[derive(Debug, Serialize)]
struct ExecuteResponse {
    vm_id: String,
    vm_ip: String,
}

#[derive(Clone)]
struct AppState {
    vm_manager: Arc<VmManager>,
}

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    // Get the server address from the environment variable or use a default
    let server_addr =
        env::var("BACKEND_SERVER_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());

    // Initialize tracing subscriber for logging
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();

    // Create VM manager
    let vm_manager = Arc::new(VmManager::new());
    let state = AppState { vm_manager };

    // Create a simple router with a health check endpoint
    let app = Router::new()
        .route("/", get(root))
        .route("/health", get(health_check))
        .route("/execute", post(execute))
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

async fn execute(
    State(state): State<AppState>,
    Json(payload): Json<ExecuteRequest>,
) -> Result<Json<ExecuteResponse>, StatusCode> {
    info!("Received execute request for language: {}", payload.language);
    info!("Code: {}", payload.code);

    // Placeholder
    let vm_id = format!("vm-{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs());

    // Mocked IP for now (will be retrieved from IP pool later)
    let vm_ip = "172.17.0.2".to_string();

    // Send code to VMM
    state.vm_manager
        .send_code_to_vm(&vm_id, &vm_ip, &payload.language, &payload.code)
        .await
        .map_err(|e| {
            tracing::error!("Failed to send code to VM: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(ExecuteResponse {
        vm_id,
        vm_ip,
    }))
}
