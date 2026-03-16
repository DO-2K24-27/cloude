# Backend Documentation

## Overview

The `backend` is the central orchestrator of the Cloude system. It provides an HTTP API for submitting code execution requests, manages the lifecycle of virtual machines, and communicates with other components such as the VMM and the agent.

## Architecture

The backend interacts with the following components:
- **CLI**: Receives HTTP requests from the CLI for job submission and status queries.
- **VMM**: Manages the creation and execution of virtual machines.
- **Agent**: Executes the user-submitted code inside the VM and returns the results.
- **Initramfs**: Provides the runtime environment for the VM based on the requested language.

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