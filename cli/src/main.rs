use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Get health status from backend
    Health,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let backend_url =
        std::env::var("BACKEND_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());

    match &cli.command {
        Some(Commands::Health) => {
            get_backend_health(&backend_url).await;
        }
        None => {
            println!("No command provided. Use --help for more information.");
        }
    }
}

async fn get_backend_health(backend_url: &str) {
    let health_url = format!("{}/health", backend_url);
    match reqwest::get(&health_url).await {
        Ok(response) => {
            if response.status().is_success() {
                match response.text().await {
                    Ok(text) => println!("Backend Health: {}", text),
                    Err(e) => eprintln!("Failed to read response: {}", e),
                }
            } else {
                eprintln!("Backend returned an error status: {}", response.status());
            }
        }
        Err(e) => eprintln!("Failed to connect to backend: {}", e),
    }
}
