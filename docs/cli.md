# CLI Documentation

## Overview

The `CLI` provides a command-line interface for interacting with the Cloude system. It allows users to submit jobs, query their status, and manage resources.

## Features

### Job Submission
- Submit code in various programming languages for execution.
- Receive a unique job ID for tracking.

### Status Queries
- Query the status of submitted jobs.
- Retrieve execution results, including `stdout`, `stderr`, and `exit_code`.

### Language Support
The CLI supports all languages available in the backend, including:
- Python
- Node.js
- Rust
- C, C++
- Go
- Java

## Usage Examples

### Submit a Job

```bash
cargo run -p cli -- go --language python --file agent/examples/hello.py
```

### Check Job Status

```bash
cargo run -p cli -- status <job_id>
```

### Using a Remote Backend

```bash
cargo run -p cli -- --backend-url http://<BACKEND_IP>:8080 go --language python --file agent/examples/hello.py
```

## Code Structure

- `src/main.rs`: Entry point for the CLI, handles command parsing and execution.
- `examples/mock_backend.rs`: Example implementation of a mock backend for testing.