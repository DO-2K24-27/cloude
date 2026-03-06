# backend

Backend API server for the cloude serverless platform.

## Overview

The backend service provides orchestration and management capabilities for the cloude platform. It handles IP address allocation for VMs, service coordination, and health monitoring.

## Features

- **HTTP API**: RESTful endpoints for platform management
- **IP Management**: Thread-safe IP address allocation and deallocation
- **State Persistence**: IP allocations persisted to disk for crash recovery
- **Health Monitoring**: Service health check endpoints

## API Endpoints

### GET /

Welcome endpoint for service verification.

**Response:**
```
Welcome to the Backend server!
```

### GET /health

Health check endpoint for monitoring and load balancers.

**Response:**
```
Backend server is healthy!
```

## IP Manager

The backend includes a sophisticated IP address manager that handles allocation and deallocation of IP addresses for VMs.

### Features

- **Thread-safe**: Concurrent access protected by mutex
- **Persistent**: State saved to JSON file after each operation
- **Pool-based**: Manages a configurable range of IP addresses
- **Crash-resistant**: State restored from disk on restart

### Usage Example

```rust
use backend::ip_manager::IpManager;
use std::net::Ipv4Addr;

// Create IP manager with pool from 192.168.100.10 to 192.168.100.50
let manager = IpManager::new(
    "ip_allocations.json",
    Ipv4Addr::new(192, 168, 100, 10),
    Ipv4Addr::new(192, 168, 100, 50)
)?;

// Allocate IP for a VM
let ip = manager.allocate("vm-123")?;
println!("Allocated: {}", ip);

// Release IP when VM is destroyed
manager.release("vm-123")?;
```

### State Format

IP allocations are stored in JSON format:

```json
{
  "allocations": {
    "vm-123": "192.168.100.10",
    "vm-456": "192.168.100.11"
  }
}
```

## Configuration

### Environment Variables

- **BACKEND_SERVER_ADDR**: Address to bind the server (default: `127.0.0.1:8080`)

See [../docs/environment.md](../docs/environment.md) for complete environment variable documentation.

## Running

### Development

```bash
cargo run -p backend
```

The server will start on `127.0.0.1:8080` by default.

### Production

```bash
export BACKEND_SERVER_ADDR="0.0.0.0:8080"
cargo run -p backend --release
```

### Docker (future)

```bash
docker build -t cloude-backend .
docker run -p 8080:8080 cloude-backend
```

## Testing

Run unit tests:

```bash
cargo test -p backend
```

Test health endpoint:

```bash
curl http://localhost:8080/health
```

## Architecture

The backend is built with:
- **axum**: Web framework for HTTP server
- **tokio**: Async runtime
- **tracing**: Structured logging
- **serde**: Serialization for state persistence

### Request Flow

```
Client
  │
  │ HTTP Request
  ▼
┌─────────────────┐
│  Axum Router    │
│  ├─ /           │
│  ├─ /health     │
│  └─ (future)    │
└────────┬────────┘
         │
         ▼
  ┌──────────────┐
  │   Handler    │
  └──────────────┘
```

## Development

### Adding Endpoints

1. Define handler function:
```rust
async fn new_endpoint() -> &'static str {
    "response"
}
```

2. Add route to router:
```rust
let app = Router::new()
    .route("/new", get(new_endpoint));
```

### Project Structure

```
backend/
├── Cargo.toml
└── src/
    ├── lib.rs           # Public library interface
    ├── ip_manager.rs    # IP allocation logic
    └── bin/
        └── main.rs      # Server entry point
```

## Dependencies

- **axum**: HTTP server framework
- **tokio**: Async runtime
- **tracing**: Logging
- **tracing-subscriber**: Log formatting
- **serde**: Serialization
- **serde_json**: JSON handling

## License

See workspace license for details.