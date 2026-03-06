# Backend

HTTP service that receives user code execution requests and coordinates with the agent. The backend manages IP allocation for VM tracking and forwards execution requests to the agent service.


## Quickstart

```bash
# 1) Ensure agent is running first
# See agent/README.md for agent setup

# 2) Run backend (from repository root)
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
- `BACKEND_AGENT_TIMEOUT_SECS` (default: `120`)
- `BACKEND_IP_FILE` (default: `ip_allocations.json`)
- `BACKEND_IP_START` (default: `172.17.0.2`)
- `BACKEND_IP_END` (default: `172.17.0.254`)


## IP Allocation

The backend maintains a pool of IP addresses (default: 172.17.0.2 - 172.17.0.254) and allocates them to VMs. Allocations are persisted to a JSON file for tracking.