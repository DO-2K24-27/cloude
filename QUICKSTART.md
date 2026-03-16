# QUICKSTART

Simple end-to-end startup for local development.

## 0) Prerequisites (one-time)

- Linux OS
- Rust toolchain with Musl (rustup target add x86_64-unknown-linux-musl)
- nftables (to setup network)
- docker (for initramfs generation)

You also need a VM kernel file at `backend/vmlinux`.

## 1) Build everything (one command)

From repository root:

```bash
cargo build -p backend -p cli && \
cargo build -p agent --target x86_64-unknown-linux-musl && \
cp ./target/x86_64-unknown-linux-musl/debug/agent ./backend/cloude-agentd && \
chmod +x ./backend/cloude-agentd
```

## 2) Start backend (one command, terminal A)

From repository root:

```bash
cd backend && \
  PATH="/usr/sbin:$PATH" \
  LANGUAGES_CONFIG_PATH=./config/languages.json \
  AGENT_BINARY_PATH=./cloude-agentd \
  INIT_SCRIPT_PATH=./init.sh \
  VM_KERNEL_PATH=./vmlinux \
  VM_INITRAMFS_DIR=./tmp \
  VM_LOG_GUEST_CONSOLE=false \
  ../target/debug/backend
```

Tip: set `VM_LOG_GUEST_CONSOLE=true` only when debugging VM boot/agent startup.

## 3) Run code with CLI (terminal B)

Submit a job:

```bash
cargo run -p cli -- go --language python --file agent/examples/hello.py
```

Then check status/result:

```bash
cargo run -p cli -- status <JOB_ID>
```

## 4) Optional reset if initramfs cache is stale

If agent changes are not reflected inside VM:

```bash
rm -f backend/tmp/*.cpio.gz
```

Then restart backend (step 2).
