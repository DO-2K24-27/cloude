# Backend - Start Guide

This file explains only how to build and start the real backend.

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

## Troubleshooting

### Address already in use

Another backend is already running.

```bash
sudo pkill -f '../target/debug/backend'
```

Then run the backend again.

### "unable to execute nft"

Ensure `/usr/sbin` is in `PATH` in the `sudo env` command:

```bash
PATH="/usr/sbin:$PATH"
```
