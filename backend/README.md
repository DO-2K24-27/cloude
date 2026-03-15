# Backend

The `backend` is the central orchestrator of the Cloude system. It provides an HTTP API for submitting code execution requests, manages the lifecycle of virtual machines, and communicates with other components such as the VMM and the agent.

## Overview

The backend is responsible for:
- Receiving and validating user requests for code execution.
- Managing the lifecycle of jobs, including their creation, execution, and cleanup.
- Provisioning virtual machines with the appropriate runtime environments.
- Communicating with the agent inside the VM to execute the code and retrieve results.
- Handling networking and initramfs management for the VMs.

## Architecture

The backend interacts with the following components:
- **CLI**: Receives HTTP requests from the CLI for job submission and status queries.
- **VMM**: Manages the creation and execution of virtual machines.
- **Agent**: Executes the user-submitted code inside the VM and returns the results.
- **Initramfs**: Provides the runtime environment for the VM based on the requested language.

## Prerequisites

- Linux host
- `sudo` access (bridge/NAT setup requires root)
- Docker installed (used for initramfs build)
- `nftables` installed
- A kernel image available for the VMs (default expected path: `backend/vmlinux`)

Install `nftables` if needed:

```bash
sudo apt install -y nftables
```

## Build (backend + VM agent binary)

From repository root:

```bash
rustup target add x86_64-unknown-linux-musl
cargo build -p backend -p agent --target x86_64-unknown-linux-musl
cp ./target/x86_64-unknown-linux-musl/debug/agent ./backend/cloude-agentd
chmod +x ./backend/cloude-agentd
```

Why musl: runtime initramfs images are Alpine-based, so a glibc-linked agent
(`./target/debug/agent`) fails at boot with `/usr/bin/cloude-agentd: not found`.
The musl build is static and runs correctly in Alpine initramfs.

## Run backend

From repository root:

```bash
cd backend
sudo env \
  PATH="/usr/sbin:$PATH" \
  LANGUAGES_CONFIG_PATH=./config/languages.json \
  AGENT_BINARY_PATH=./cloude-agentd \
  INIT_SCRIPT_PATH=./init.sh \
  VM_KERNEL_PATH=./vmlinux \
  VM_INITRAMFS_DIR=./tmp \
  VM_LOG_GUEST_CONSOLE=false \
  ../target/debug/backend
```

Expected log:

```text
INFO backend: Starting Backend server on 127.0.0.1:8080
```

## Environment variables

### Required in practice

- `VM_KERNEL_PATH` (default `./vmlinux`): Linux kernel used to boot each VM.
- `AGENT_BINARY_PATH` (default `./cloude-agentd`): binary injected into initramfs.
- `INIT_SCRIPT_PATH` (default `./init.sh`): init script injected as `/init`.

### Common runtime settings

- `BACKEND_SERVER_ADDR` (default `127.0.0.1:8080`)
- `BRIDGE_NAME` (default `cloudebr0`)
- `IP_RANGE` (default `10.39.1.0`)
- `IP_MASK` (default `24`, must be `<= 30`)
- `LANGUAGES_CONFIG_PATH` (default `./config/languages.json`)
- `VM_INITRAMFS_DIR` (default `./tmp`)
- `IP_ALLOCATIONS_PATH` (default `./tmp/ip_allocations.json`)
- `VM_LOG_GUEST_CONSOLE` (default `false`)
  - `true/1/yes/on`: print guest kernel+init logs in backend terminal
  - `false`: keep backend logs clean

## Quick health check

In another terminal:

```bash
curl -i http://127.0.0.1:8080/health
```

Expected body:

```text
Backend server is healthy!
```

## Troubleshooting

### "unable to execute nft"

Ensure `/usr/sbin` is in `PATH` in the `sudo env` command:

```bash
PATH="/usr/sbin:$PATH"
```

## API

### Endpoints

- `POST /run`
  - Submits a new job for execution.
  - Request body: `{ "language": "python", "code": "print(1+1)" }`
  - Response: `{ "id": "job-1" }`

- `GET /status/{id}`
  - Retrieves the status of a submitted job.
  - Response: `{ "id": "job-1", "status": "done", "stdout": "2\n", "stderr": "", "exit_code": 0 }`

- `GET /health`
  - Returns the health status of the backend.
  - Response: `"Backend server is healthy!"`

## Key Functions

Below are the most important functions implemented in the `backend` and their roles:

### 1. `run_job`
- **Purpose**: Handles the submission of a new job for execution.
- **Parameters**:
  - `State(state): State<Arc<AppState>>`: Shared application state containing job management and configuration.
  - `Json(payload): Json<RunRequest>`: The HTTP request payload containing the `language` and `code` fields.
- **Returns**:
  - `axum::response::Response`: The HTTP response containing the job ID or an error message.
- **Details**:
  - Validates the requested language against the supported languages.
  - Creates a new job entry and stores it in the shared state.
  - Spawns a background task to handle VM provisioning and job execution.

### 2. `get_status`
- **Purpose**: Retrieves the status of a submitted job.
- **Parameters**:
  - `State(state): State<Arc<AppState>>`: Shared application state containing job management.
  - `Path(id): Path<String>`: The job ID to query.
- **Returns**:
  - `impl IntoResponse`: The HTTP response containing the job status or an error message.
- **Details**:
  - Looks up the job ID in the shared state.
  - Returns the job's current status, including `stdout`, `stderr`, and `exit_code` if available.

### 3. `health_check`
- **Purpose**: Provides a health status of the backend server.
- **Parameters**:
  - None.
- **Returns**:
  - `&'static str`: A static string indicating the health status.
- **Details**:
  - Returns a simple "Backend server is healthy!" message to indicate that the server is operational.

### 4. `initialize_network`
- **Purpose**: Sets up the network bridge and NAT rules for VM networking.
- **Parameters**:
  - `bridge_name: &str`: The name of the network bridge to create.
  - `ip_range: Ipv4Addr`: The IP range to use for the bridge.
  - `ip_mask: u8`: The subnet mask for the IP range.
- **Returns**:
  - `Result<(), NetworkError>`: Indicates whether the network was successfully initialized.
- **Details**:
  - Creates a network bridge using `nftables`.
  - Configures NAT rules to enable internet access for VMs.

### 5. `load_initramfs`
- **Purpose**: Loads the initramfs file for the requested language and version.
- **Parameters**:
  - `language: &str`: The programming language for which to load the initramfs.
  - `version: &str`: The version of the language runtime.
- **Returns**:
  - `Result<PathBuf, InitramfsError>`: The path to the initramfs file on success, or an error on failure.
- **Details**:
  - Constructs the expected path to the initramfs file based on the language and version.
  - Verifies that the file exists in the `tmp/` directory.

### 6. `cleanup_jobs`
- **Purpose**: Periodically cleans up expired jobs to prevent unbounded memory growth.
- **Parameters**:
  - None (runs as a background task).
- **Details**:
  - Removes jobs that have been completed or errored for more than 5 minutes.
  - Logs the number of jobs evicted during each cleanup cycle.
### "/usr/bin/cloude-agentd: not found" inside VM

Rebuild and copy the musl binary:

```bash
rustup target add x86_64-unknown-linux-musl
cargo build -p agent --target x86_64-unknown-linux-musl
cp ./target/x86_64-unknown-linux-musl/debug/agent ./backend/cloude-agentd
chmod +x ./backend/cloude-agentd
```

### Agent seems updated but VM still runs old one

Backend caches initramfs images in `backend/tmp/*.cpio.gz`.
If needed, remove them once and restart backend to force a rebuild.
