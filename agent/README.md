# Agent

HTTP service intended to run inside the guest VM. It receives execution requests from the backend and runs code directly in the guest userspace.

## Requirements

- Linux x86_64
- `rust` / `cargo`
- `curl`
- Language runtimes available in guest (`python3`, `node`, `rustc` as needed)

## Quickstart

```bash
# 1) Check dependencies
cargo --version
curl --version

# 2) Run agent
cargo run -p agent
```

In another terminal:

```bash
# 3) Health check
curl -sS http://127.0.0.1:3001/health && echo

# 4) Execute code
curl -sS -X POST http://127.0.0.1:3001/execute \
  -H 'content-type: application/json' \
  -d '{"language":"python","code":"print(1+1)"}'
```

## Environment variables

- `AGENT_SERVER_ADDR` (default: `127.0.0.1:3001`)
- `AGENT_WORK_DIR` (default: `build`)
- `AGENT_EXEC_TIMEOUT_SECS` (default: `30`)

For detailed documentation, refer to [docs/agent.md](../docs/agent.md).