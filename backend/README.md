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

Install `nftables` if needed:

```bash
sudo apt install nftables
```

## Build

From repository root:

```bash
cargo build -p backend -p agent
cp ./target/debug/agent ./backend/cloude-agentd
chmod +x ./backend/cloude-agentd
```

## Run backend

From repository root:

```bash
cd backend
sudo env \
  LANGUAGES_CONFIG_PATH=./config/languages.json \
  AGENT_BINARY_PATH=./cloude-agentd \
  INIT_SCRIPT_PATH=./init.sh \
  PATH="/usr/sbin:$PATH" \
  ../target/debug/backend
```

Expected log:

```text
INFO backend: Starting Backend server on 127.0.0.1:8080
```

## Quick health check

In another terminal:

```bash
curl -i http://127.0.0.1:8080/health
```

Expected body:

```text
Backend server is healthy!
```

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
