use anyhow::Context;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use backend::ip_manager::IpManager;
use backend::vm_manager::VmManager;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::RandomState;
use std::env;
use std::hash::{BuildHasher, Hasher};
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

    // Initialize IP pool with configurable range
    let ip_file = env::var("BACKEND_IP_FILE").unwrap_or_else(|_| "ip_allocations.json".to_string());
    
    let start_ip = env::var("BACKEND_IP_START")
        .ok()
        .and_then(|s| s.parse::<Ipv4Addr>().ok())
        .unwrap_or_else(|| {
            info!("Using default start IP: 172.17.0.2");
            Ipv4Addr::new(172, 17, 0, 2)
        });
    
    let end_ip = env::var("BACKEND_IP_END")
        .ok()
        .and_then(|s| s.parse::<Ipv4Addr>().ok())
        .unwrap_or_else(|| {
            info!("Using default end IP: 172.17.0.254");
            Ipv4Addr::new(172, 17, 0, 254)
        });
    
    let ip_manager = IpManager::new(&ip_file, start_ip, end_ip)
        .context("Failed to initialize IP manager")
        .expect("Critical error during startup");
    
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

/// Generates a unique VM identifier using timestamp and random suffix
/// 
/// Format: vm-{timestamp_hex}-{random_4hex}
/// Example: vm-65e7a3f1-a4b2
fn generate_vm_id() -> String {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    
    let random_state = RandomState::new();
    let mut hasher = random_state.build_hasher();
    hasher.write_u64(timestamp);
    let random_suffix = hasher.finish() & 0xFFFF; // 4 hex digits
    
    format!("vm-{:x}-{:04x}", timestamp, random_suffix)
}

async fn execute(
    State(state): State<AppState>,
    Json(payload): Json<ExecuteRequest>,
) -> Result<Json<ExecuteResponse>, StatusCode> {
    let vm_id = generate_vm_id();

    info!(
        "Received execute request - VM ID: {}, Language: {}, Code size: {} bytes",
        vm_id,
        payload.language,
        payload.code.len()
    );

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
            // Release the allocated IP on failure
            match state.vm_manager.release_ip(&vm_id) {
                Ok(true) => tracing::info!("Released IP {} for VM {} after failure", vm_ip, vm_id),
                Ok(false) => tracing::warn!("No IP to release for VM {} (not allocated)", vm_id),
                Err(release_err) => tracing::error!("Failed to release IP {} for VM {}: {}", vm_ip, vm_id, release_err),
            }
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Release the IP after successful execution
    match state.vm_manager.release_ip(&vm_id) {
        Ok(true) => tracing::info!("Released IP {} for VM {} after successful execution", vm_ip, vm_id),
        Ok(false) => tracing::warn!("No IP to release for VM {} after execution (not allocated)", vm_id),
        Err(release_err) => {
            tracing::error!("Failed to release IP {} for VM {} after successful execution: {}", vm_ip, vm_id, release_err);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    Ok(Json(ExecuteResponse {
        vm_id,
        vm_ip,
    }))
}
