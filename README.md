# cloude

Serverless code execution platform built in Rust by DO24-27 from Polytech Montpellier.

## Overview

cloude is a lightweight serverless computing platform that executes arbitrary code in isolated micro-VMs using QEMU and custom Linux kernels. The platform supports multiple programming languages (Python, Node.js, Rust) and provides secure, isolated execution environments for each request.

## Architecture

The project consists of four main components:

- **agent**: HTTP service that builds initramfs images and executes code inside QEMU micro-VMs
- **backend**: Main API server that coordinates the platform services
- **vmm**: Custom virtual machine monitor (VMM) for KVM-based virtualization
- **cli**: Command-line interface for VM configuration

See [docs/architecture.md](docs/architecture.md) for detailed architecture documentation.

## Quick Start

### Prerequisites

- Linux x86_64 system
- Rust toolchain (cargo, rustc)
- QEMU (qemu-system-x86_64)
- KVM support (for vmm component)

### Running the Agent

The agent is the core execution service:

```bash
# Fetch kernel
./agent/scripts/fetch-kernel.sh

# Set kernel path
export AGENT_KERNEL_PATH="$(pwd)/agent/.cache/kernels/vmlinuz-virt-3.23.3"

# Run agent
cargo run -p agent

# Test execution
curl -X POST http://127.0.0.1:3001/execute \
  -H 'content-type: application/json' \
  -d '{"language":"python","code":"print(1+1)"}'
```

### Running the Backend

```bash
cargo run -p backend
```

### Running the CLI

```bash
cargo run -p cli -- --cpu 2 --ram 1024 --kernel path/to/kernel --initramfs path/to/initramfs --file path/to/disk
```

## Components

### agent

HTTP service that executes user code in isolated QEMU VMs. Supports Python, Node.js, and Rust runtimes.

- **Port**: 3001 (default)
- **Documentation**: [agent/README.md](agent/README.md)

### backend

API server for orchestrating the platform. Manages IP allocation and service coordination.

- **Port**: 8080 (default)
- **Documentation**: [backend/readme.md](backend/readme.md)

### vmm

Custom virtual machine monitor implementing KVM-based virtualization with virtio device support.

- **Documentation**: [vmm/readme.md](vmm/readme.md)

### cli

Command-line tool for configuring and managing virtual machines.

- **Documentation**: [cli/readme.md](cli/readme.md)

## Environment Variables

See [docs/environment.md](docs/environment.md) for a complete list of environment variables used across all components.

## Development

Build all components:
```bash
cargo build --workspace
```

Run tests:
```bash
cargo test --workspace
```

## License

See individual component licenses for details.