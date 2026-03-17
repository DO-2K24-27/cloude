# Backend

The `backend` is the central orchestrator of the Cloude system. It provides an HTTP API for submitting code execution requests, manages the lifecycle of virtual machines, and communicates with other components such as the VMM and the agent.

## How to Start

### Prerequisites

- Linux host
- `sudo` access (bridge/NAT setup requires root)
- Docker installed (used for initramfs build)
- `nftables` installed

Install `nftables` if needed:
```bash
sudo apt install nftables
```

## Build (backend + VM agent binary)

From root repository, execute the following command in root mode:

```bash
cargo xtask build
```

Why musl: runtime initramfs images are Alpine-based, so a glibc-linked agent (`./target/debug/agent`) fails at boot with `/usr/bin/cloude-agentd: not found`. The musl build is static and runs correctly in Alpine initramfs.

## Run backend

From root repository, execute the following command in root mode:

```bash
cargo xtask run-backend
```

To enable verbose VM guest console logging, add `VM_LOG_GUEST_CONSOLE=true` to the `backend/.env` file.

Expected log:

```text
INFO backend: Starting Backend server on 127.0.0.1:8080
```

## Environment variables

Configuration is read from `backend/.env` (create from `.env.exemple` if needed).

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