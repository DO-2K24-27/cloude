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

# 2) Fetch pinned kernel artifact
./agent/scripts/fetch-kernel.sh

# 3) Run agent
export AGENT_KERNEL_PATH="$(pwd)/agent/.cache/kernels/vmlinuz-virt-3.23.3"
test -f "$AGENT_KERNEL_PATH" || (echo "Kernel not found: $AGENT_KERNEL_PATH" && exit 1)

cargo run -p agent
```

In another terminal:

```bash
# 4) Health check
curl -sS http://127.0.0.1:3001/health && echo

# 5) Execute code
curl -sS -X POST http://127.0.0.1:3001/execute \
  -H 'content-type: application/json' \
  -d '{"language":"python","code":"print(1+1)"}'
```

## Environment variables

- `AGENT_KERNEL_PATH` (required)
- `AGENT_SERVER_ADDR` (default: `127.0.0.1:3001`)
- `AGENT_WORK_DIR` (default: `build`)
- `AGENT_QEMU_TIMEOUT_SECS` (default: `120`)

## API

`POST /execute`

Request body:

```json
{ "language": "python", "code": "print(1+1)" }
```

Response:

```json
{ "job_id": "job-1", "exit_code": 0, "stdout": "2\n", "stderr": "" }
```
