use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use backend::ip_manager::IpManager;
use backend::vm_manager::VmManager;
use serde::{Deserialize, Serialize};
use std::env;
use std::net::Ipv4Addr;
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

    // Initialize IP pool (172.17.0.2 to 172.17.0.254)
    let ip_file = env::var("BACKEND_IP_FILE").unwrap_or_else(|_| "ip_allocations.json".to_string());
    let start_ip = Ipv4Addr::new(172, 17, 0, 2);
    let end_ip = Ipv4Addr::new(172, 17, 0, 254);
    let ip_manager = IpManager::new(&ip_file, start_ip, end_ip)
        .expect("Failed to initialize IP manager");
    
    info!("IP pool initialized: {} - {}", start_ip, end_ip);

    // Create VM manager
    let vm_manager = Arc::new(VmManager::new(ip_manager));
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

    // Generate a unique ID for the VM
    let vm_id = format!("vm-{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs());

    // Allocate IP from pool
    let vm_ip = state.vm_manager.allocate_ip(&vm_id)
        .map_err(|e| {
            tracing::error!("Failed to allocate IP: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    
    info!("Allocated IP {} for VM {}", vm_ip, vm_id);

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
