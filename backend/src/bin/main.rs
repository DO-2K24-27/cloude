use axum::{Router, routing::get};
use log::info;
use std::env;
use std::net::Ipv4Addr;
use tokio::net::TcpListener;
use tracing_subscriber::{self, EnvFilter};
use virt::network::{setup_bridge, setup_nat};

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    // init logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    log::debug!("Debug logging enabled");

    // Get the server address from the environment variable or use a default
    let server_addr =
        env::var("BACKEND_SERVER_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    let bridge_name = env::var("BRIDGE_NAME").unwrap_or_else(|_| "cloudebr0".to_string());
    // 39 is miku
    let ip_range: Ipv4Addr = env::var("IP_RANGE")
        .as_deref()
        .unwrap_or_else(|_| "10.39.1.0")
        .parse()
        .map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("IP_RANGE env variable is invalid: {}", e),
            )
        })?;
    let ip_mask: u8 = env::var("IP_MASK")
        .unwrap_or_else(|_| "24".to_string())
        .parse()
        .map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("IP_MASK env variable is invalid: {}", e),
            )
        })?;

    // NOTE, DO NOT MERGE UNTIL REMOVAL OF THIS COMMENT:
    // I think using TWO WHOLE crates only to create the interface and tell it to do postrouting/ip forwarding may be a lot.
    // An alternative would be to use ioctl (?) or just run a command.
    // Please give me feedback, this is making me go crazy.

    // Set up the bridge and NAT rules
    let host_ip: Ipv4Addr = (ip_range.to_bits() + 1).into();
    if let Err(e) = setup_bridge(bridge_name, host_ip, ip_mask).await {
        eprintln!("Failed to set up bridge: {}", e);
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            e.to_string(),
        ));
    }

    if let Err(e) = setup_nat(ip_range, ip_mask) {
        eprintln!("Failed to set up NAT: {}", e);
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            e.to_string(),
        ));
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
