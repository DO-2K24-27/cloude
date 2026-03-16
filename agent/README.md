# Agent

HTTP service intended to run inside the guest VM.
It receives execution requests from the backend and runs code directly in the guest userspace.

## Requirements

- Linux x86_64
- `rust` / `cargo`
- `curl`
- language runtimes available in guest (`python3`, `node`, `rustc` as needed)

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

## Implemented Features

### 1. HTTP API
- **Purpose**: Provides an HTTP interface for executing code inside the guest VM.
- **Endpoints**:
  - `POST /execute`: Accepts code execution requests.
  - `GET /health`: Returns the health status of the agent.
- **Details**:
  - Validates incoming requests for supported languages and code format.
  - Returns execution results, including `stdout`, `stderr`, and `exit_code`.

### 2. Language Runtimes
- **Purpose**: Supports multiple programming languages for code execution.
- **Supported Languages**:
  - Python
  - Node.js
  - Rust
  - C, C++
  - Go
- **Details**:
  - Dynamically selects the appropriate runtime based on the `language` field in the request.
  - Executes code in a secure and isolated environment.

### 3. Execution Timeout
- **Purpose**: Prevents long-running or stuck processes from consuming resources indefinitely.
- **Details**:
  - Configurable via the `AGENT_EXEC_TIMEOUT_SECS` environment variable.
  - Terminates execution if the timeout is exceeded.

### 4. Working Directory Management
- **Purpose**: Manages temporary files and directories for code execution.
- **Details**:
  - Uses the `AGENT_WORK_DIR` environment variable to specify the working directory.
  - Cleans up temporary files after execution to prevent resource leaks.

## Key Functions

Below are the most important functions implemented in the `cloude-agent` and their roles:

### 1. `execute_code`
- **Purpose**: Executes the user-submitted code in the specified language runtime.
- **Parameters**:
  - `language: &str`: The programming language in which the code is written (e.g., `python`, `node`).
  - `code: &str`: The code to be executed.
- **Returns**:
  - `Result<ExecutionResult, ExecutionError>`: The result of the execution, containing `stdout`, `stderr`, and `exit_code` on success, or an error on failure.
- **Details**:
  - Dynamically selects the appropriate runtime based on the `language` parameter.
  - Executes the code in a secure and isolated environment.
  - Captures and returns the execution output and errors.

### 2. `initialize_runtime`
- **Purpose**: Prepares the runtime environment for a specific programming language.
- **Parameters**:
  - `language: &str`: The programming language to initialize (e.g., `python`, `node`).
- **Returns**:
  - `Result<(), RuntimeError>`: Indicates whether the runtime was successfully initialized.
- **Details**:
  - Ensures that the required runtime dependencies are available.
  - Sets up any necessary environment variables or configurations.

### 3. `handle_execute_request`
- **Purpose**: Handles HTTP requests to the `/execute` endpoint.
- **Parameters**:
  - `request: ExecuteRequest`: The HTTP request containing the `language` and `code` fields.
- **Returns**:
  - `Result<ExecuteResponse, HttpError>`: The HTTP response containing the execution result or an error.
- **Details**:
  - Parses and validates the incoming request.
  - Calls `execute_code` to run the submitted code.
  - Formats and returns the execution result as an HTTP response.

### 4. `cleanup_working_directory`
- **Purpose**: Cleans up temporary files and directories used during code execution.
- **Parameters**:
  - `work_dir: &Path`: The path to the working directory to clean up.
- **Returns**:
  - `Result<(), CleanupError>`: Indicates whether the cleanup was successful.
- **Details**:
  - Deletes temporary files and directories to prevent resource leaks.
  - Ensures that the working directory is ready for the next execution request.

### 5. `health_check`
- **Purpose**: Provides a health status of the agent.
- **Parameters**:
  - None.
- **Returns**:
  - `Result<(), HealthError>`: Indicates whether the agent is healthy.
- **Details**:
  - Checks the availability of required resources and dependencies.
  - Returns an HTTP 200 status if the agent is operational.