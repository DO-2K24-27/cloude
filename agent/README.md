# Agent

HTTP service that executes user code inside a QEMU VM using an initramfs built on demand.

## Requirements

- Linux x86_64
- `rust` / `cargo`
- `qemu-system-x86_64`
- `curl`

## Quickstart

```bash
# 1) Check dependencies
qemu-system-x86_64 --version
cargo --version
curl --version

# 2) Run unit tests to ensure everything works
cargo test -p agent

# 3) Fetch pinned kernel artifact
./agent/scripts/fetch-kernel.sh

# 4) Run agent (requires serial port communication)
AGENT_KERNEL_PATH="$(pwd)/agent/.cache/kernels/vmlinuz-virt-3.23.3" cargo run -p agent
```

The agent will wait for IP configuration from the serial port before starting the HTTP server.

**For testing purposes**, in another terminal, provide the IP configuration using a test file:

```bash
# Create test file that the agent will check first
echo "IP=127.0.0.1:3001" > /tmp/agent_serial_test

# The agent will automatically use this file if it exists
```

**Note**: This test file solution is temporary and will be replaced by real serial port communication in production.

Once the agent receives the IP configuration:

```bash
# 5) Health check (use the IP from serial configuration)
curl -sS http://127.0.0.1:3001/health && echo

# 6) Test Python execution
curl -sS -X POST http://127.0.0.1:3001/execute \
  -H 'content-type: application/json' \
  -d '{"language":"python","code":"print(1+1)"}'
```

Expected response format:
```json
{"job_id":"job-1","exit_code":0,"stdout":"2\n","stderr":""}
```

## Environment variables

- `AGENT_KERNEL_PATH` (required)
- `AGENT_WORK_DIR` (default: `build`)
- `AGENT_QEMU_TIMEOUT_SECS` (default: `120`)

## API

`GET /health`

Response: `ok`

`POST /execute`

Request body:

```json
{ "language": "python", "code": "print(1+1)" }
```

Response:

```json
{ "job_id": "job-1", "exit_code": 0, "stdout": "2\n", "stderr": "" }
```
