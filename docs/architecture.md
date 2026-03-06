# cloude Architecture

This document describes the architecture and design of the cloude serverless platform.

## System Overview

cloude is a serverless code execution platform that runs user code in isolated micro-VMs. The system is built around lightweight virtualization using QEMU and custom Linux kernels, providing strong isolation guarantees while maintaining low overhead.

## Component Architecture

```
┌─────────────┐
│   Client    │
└──────┬──────┘
       │
       │ HTTP POST /execute
       ▼
┌─────────────────────────────────────────┐
│             agent                       │
│  ┌────────────────────────────────┐    │
│  │  HTTP Server (axum)            │    │
│  │  - /health                     │    │
│  │  - /execute                    │    │
│  └──────────┬─────────────────────┘    │
│             │                           │
│             ▼                           │
│  ┌────────────────────────────────┐    │
│  │  Builder                       │    │
│  │  - Runtime detection           │    │
│  │  - Initramfs generation        │    │
│  │  - Code injection              │    │
│  └──────────┬─────────────────────┘    │
│             │                           │
│             ▼                           │
│  ┌────────────────────────────────┐    │
│  │  QEMU Runner                   │    │
│  │  - Process spawning            │    │
│  │  - Output capture              │    │
│  │  - Timeout management          │    │
│  └────────────────────────────────┘    │
└─────────────────────────────────────────┘
       │
       │ qemu-system-x86_64
       ▼
┌─────────────────────────────────────────┐
│        Micro-VM                         │
│  ┌────────────────────────────────┐    │
│  │  Linux Kernel                  │    │
│  │  - Alpine Linux (minimal)      │    │
│  └────────────────────────────────┘    │
│  ┌────────────────────────────────┐    │
│  │  Initramfs                     │    │
│  │  - Runtime (python/node/rust)  │    │
│  │  - User code                   │    │
│  │  - Init script                 │    │
│  └────────────────────────────────┘    │
└─────────────────────────────────────────┘
```

## Components

### agent

The agent is the primary execution service. It receives code execution requests via HTTP and orchestrates the entire execution pipeline.

**Responsibilities:**
- Accept HTTP requests with language and code
- Determine appropriate runtime (Python, Node.js, Rust)
- Build custom initramfs containing runtime and user code
- Spawn QEMU process with kernel and initramfs
- Capture and parse execution output
- Return results to client

**Key modules:**
- `main.rs`: HTTP server and request handling
- `builder/`: Initramfs construction logic
- `runtimes/`: Language-specific runtime configurations
- `qemu.rs`: QEMU process management

**Execution flow:**
1. Client sends POST to /execute with {language, code}
2. Agent validates language support
3. Builder creates initramfs with runtime + code
4. QEMU spawns VM with kernel and initramfs
5. Init script runs code, captures output
6. Agent parses output and exit code
7. Response returned to client

### backend

The backend service provides orchestration and management functionality for the platform.

**Responsibilities:**
- IP address allocation for VMs
- Service health monitoring
- Coordination between platform components

**Key modules:**
- `main.rs`: HTTP server
- `ip_manager.rs`: Thread-safe IP allocation with persistence

The IP manager maintains a pool of available IP addresses and persists allocations to disk, ensuring consistency across restarts.

### vmm

Custom virtual machine monitor implementing KVM-based virtualization. This component provides low-level VM management capabilities.

**Responsibilities:**
- Direct KVM interaction via ioctl
- Memory management for guest VMs
- vCPU configuration and execution
- Device emulation (serial, virtio-net)
- Interrupt handling

**Key modules:**
- `lib.rs`: Core VMM implementation
- `cpu/`: vCPU management, CPUID, MP tables
- `devices/`: Serial console, stdin handler, virtio devices
- `kernel.rs`: Kernel loading
- `irq_allocator.rs`: IRQ allocation for devices

### cli

Command-line interface for VM execution. Provides a user-friendly way to use the backend and execute their code.

## Data Flow

### Code Execution Request

1. **HTTP Request**: Client sends execution request to agent
   ```json
   {
     "language": "python",
     "code": "print('hello')"
   }
   ```

2. **Runtime Selection**: Agent selects appropriate runtime configuration based on language

3. **Image Building**: Builder module:
   - Creates temporary directory in work_dir
   - Copies runtime binaries from base image
   - Injects user code as source file
   - Generates init script for execution
   - Builds initramfs archive

4. **VM Execution**: QEMU runner:
   - Spawns qemu-system-x86_64 process
   - Loads kernel (Alpine Linux)
   - Mounts initramfs as root filesystem
   - Configures serial console for output
   - Sets execution timeout

5. **Code Execution**: Inside VM:
   - Init script runs automatically
   - Sets up minimal environment
   - Executes code with appropriate interpreter
   - Captures stdout/stderr
   - Prints exit code

6. **Output Capture**: Agent:
   - Reads QEMU serial output
   - Parses markers (--- PROGRAM OUTPUT ---, etc.)
   - Extracts exit code
   - Filters kernel log lines

7. **Response**: Returns JSON with exit code, stdout, stderr

### Concurrency Control

The agent uses a semaphore to limit concurrent VM executions, preventing resource exhaustion. By default, only one VM runs at a time (configurable via semaphore size).

## Runtime System

Each supported language has a runtime configuration defining:
- **base_image**: Container or directory with runtime binaries
- **source_extension**: File extension for code (.py, .js, .rs)
- **run_command**: Command to execute code
- **compile_command**: Optional compilation step (Rust only)

Supported runtimes:
- **Python**: Executes .py files directly
- **Node.js**: Executes .js files with node
- **Rust**: Compiles .rs files, executes binary

## Security Model

**Isolation**: Each execution runs in a fresh QEMU VM with no persistent state. VMs are completely isolated from the host and each other.

**Timeouts**: Configurable execution timeout (default 120s) prevents infinite loops and resource exhaustion.

**Resource limits**: QEMU VMs have fixed memory allocation (512MB) and limited CPU access.

**No network**: VMs run with no network access by default (can be enabled via virtio-net).

## State Management

### Agent State
- Job counter: Atomic counter for unique job IDs
- Work directory: Temporary build artifacts (initramfs images)
- Kernel path: Location of Linux kernel image

### Backend State
- IP allocations: Persisted to JSON file
- Thread-safe access via mutex

### VM State
- Stateless: Each VM is ephemeral and isolated
- No persistence between executions