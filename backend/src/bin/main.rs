use axum::{Json, Router, http::StatusCode, routing::{get, post}};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::env;
use std::time::Duration;
use tokio::net::TcpListener;
use tracing::{Level, error, info};

#[derive(Deserialize)]
struct ExecuteRequest {
    language: String,
    code: String,
}

#[derive(Deserialize, Serialize)]
struct ExecuteResponse {
    job_id: String,
    exit_code: i32,
    stdout: String,
    stderr: String,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Get the server address from the environment variable or use a default
    let server_addr =
        env::var("BACKEND_SERVER_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());

    // Initialize tracing subscriber for logging
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();    
    let app = Router::new()
        .route("/", get(root))
        .route("/health", get(health_check))
        .route("/execute", post(execute_code));

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

async fn execute_code(
    Json(payload): Json<ExecuteRequest>,
) -> Result<Json<ExecuteResponse>, (StatusCode, Json<ErrorResponse>)> {
    let agent_url = env::var("AGENT_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:3001".to_string());

    let client = Client::builder()
        .timeout(Duration::from_secs(180))
        .build()
        .unwrap();

    let resp = client
        .post(format!("{}/execute", agent_url))
        .json(&serde_json::json!({
            "language": payload.language,
            "code": payload.code,
        }))
        .send()
        .await
        .map_err(|e| {
            let msg = format!("Failed to reach agent: {}", e);
            error!("{}", msg);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse { error: msg }))
        })?;

    let result: ExecuteResponse = resp
        .json()
        .await
        .map_err(|e| {
            let msg = format!("Invalid agent response: {}", e);
            error!("{}", msg);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse { error: msg }))
        })?;

    info!(job_id = %result.job_id, exit_code = result.exit_code, "Result received");
    Ok(Json(result))
}
