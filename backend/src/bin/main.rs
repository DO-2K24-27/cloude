use axum::{Router, routing::get};
use backend::network::{setup_bridge, setup_nat};
use std::env;
use tokio::net::TcpListener;
use tracing::{Level, info};
use tracing_subscriber;

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    // Get the server address from the environment variable or use a default
    let server_addr =
        env::var("BACKEND_SERVER_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());

    // Initialize tracing subscriber for logging
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();

    // NOTE, DO NOT MERGE UNTIL REMOVAL OF THIS COMMENT:
    // I think using TWO WHOLE crates only to create the interface and tell it to do postrouting/ip forwarding may be a lot.
    // An alternative would be to use ioctl (?) or just run a command.
    // Please give me feedback, this is making me go crazy.
    
    // Set up the bridge and NAT rules
    if let Err(e) = setup_bridge().await {
        eprintln!("Failed to set up bridge: {}", e);
        return Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()));
    }
    if let Err(e) = setup_nat() {
        eprintln!("Failed to set up NAT: {}", e);
        return Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()));
    }
    
    // Create a simple router with a health check endpoint
    let app = Router::new()
        .route("/", get(root))
        .route("/health", get(health_check));

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
