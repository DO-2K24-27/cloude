# Backend

HTTP service that receives user code execution requests and coordinates with the agent. The backend manages IP allocation for VM tracking and forwards execution requests to the agent service.

## Requirements

- Linux x86_64
- `rust` / `cargo`
- Running agent service (see `agent/README.md`)

## Quickstart

```bash
# 1) Ensure agent is running first
# See agent/README.md for agent setup

# 2) Run backend
cd /home/margo/Documents/Git/cloude
cargo run -p backend
```

In another terminal:

```bash
# 3) Health check
curl -sS http://127.0.0.1:8080/health && echo

# 4) Execute code via backend
curl -sS -X POST http://127.0.0.1:8080/execute \
  -H 'content-type: application/json' \
  -d '{"language":"python","code":"print(\"Hello World!\")"}'
```

## Environment variables

- `BACKEND_SERVER_ADDR` (default: `127.0.0.1:8080`)
- `BACKEND_AGENT_ADDR` (default: `http://127.0.0.1:3001`)
- `BACKEND_IP_FILE` (default: `ip_allocations.json`)
- `BACKEND_IP_START` (default: `172.17.0.2`)
- `BACKEND_IP_END` (default: `172.17.0.254`)

## API

### `GET /`

Returns welcome message.

### `GET /health`

Health check endpoint.

Response: `Backend server is healthy!`

### `POST /execute`

Execute user code by forwarding to the agent.

Request body:

```json
{
  "language": "python",
  "code": "print(1+1)"
}
```

Response:

```json
{
  "vm_id": "vm-1709672304",
  "vm_ip": "172.17.0.2"
}
```

Supported languages: `python`, `node`, `rust`

## Architecture

```
Client → Backend (HTTP:8080) → Agent (HTTP:3001) → QEMU/VMM
         ↓
         Allocates unique IP
         Tracks VM ID
```

The backend is a lightweight coordinator that:
1. Receives code execution requests from clients
2. Allocates a unique IP address for tracking
3. Forwards the request to the agent service
4. Returns VM ID and allocated IP to the client

All code execution, initramfs building, and VM management is handled by the agent.

## Example usage

### Python

```bash
curl -X POST http://127.0.0.1:8080/execute \
  -H 'Content-Type: application/json' \
  -d '{
    "language": "python",
    "code": "for i in range(5): print(i)"
  }'
```

### Node.js

```bash
curl -X POST http://127.0.0.1:8080/execute \
  -H 'Content-Type: application/json' \
  -d '{
    "language": "node",
    "code": "console.log(2 + 2);"
  }'
```

### Rust

```bash
curl -X POST http://127.0.0.1:8080/execute \
  -H 'Content-Type: application/json' \
  -d '{
    "language": "rust",
    "code": "fn main() { println!(\"Hello Rust!\"); }"
  }'
```

## IP Allocation

The backend maintains a pool of IP addresses (default: 172.17.0.2 - 172.17.0.254) and allocates them to VMs. Allocations are persisted to a JSON file for tracking.

View current allocations:

```bash
cat ip_allocations.json
```

Example:

```json
{
  "vm-1709672304": "172.17.0.2",
  "vm-1709672315": "172.17.0.3"
}
```

## Dependencies

See `backend/Cargo.toml`:
- `axum` - HTTP server framework
- `tokio` - Async runtime
- `reqwest` - HTTP client for agent communication
- `serde` / `serde_json` - JSON serialization
- `tracing` - Structured logging
