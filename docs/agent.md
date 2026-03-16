# Agent Documentation

## Overview

HTTP service intended to run inside the guest VM. It receives execution requests from the backend and runs code directly in the guest userspace.

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
  - Java
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

### 5. Error Handling
- **Purpose**: Provides robust error handling for execution failures.
- **Details**:
  - Captures runtime errors and returns them in the `stderr` field of the response.
  - Handles invalid requests with appropriate HTTP status codes and error messages.