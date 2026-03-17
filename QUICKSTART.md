# QUICKSTART

Simple end-to-end startup for local development.

## 0) Prerequisites (one-time)

- Linux OS
- Rust toolchain with Musl: `rustup target add x86_64-unknown-linux-musl`
- nftables (to setup network)
- docker (for initramfs generation)
- Build tools (curl, tar, make, gcc)

You need a VM kernel file at `backend/vmlinux`. You must build it separately (instructions to come)

## 1) Build everything

From repository root:

```bash
cargo xtask build
```

This will:
- Build backend and CLI binaries
- Build agent with musl target
- Copy agent binary to `backend/cloude-agentd`

## 2) Start backend (terminal A)

From repository root:

```bash
cargo xtask run-backend
```

**Note:** The backend requires root privileges (sudo) to setup network interfaces and run VMs. The `run-backend` command will automatically use `sudo` to start the backend.

Tip: Set `VM_LOG_GUEST_CONSOLE=true` in `backend/.env` only when debugging VM boot/agent startup.

## 3) Run code with CLI (terminal B)

### Run sample Python code

Submit a Python job using the example file:

```bash
cargo xtask run-cli -- go --language python --file agent/examples/hello.py
```

This will:
1. Upload the Python code to the backend
2. Backend creates a VM with Python runtime
3. Agent executes the code inside the VM
4. Returns the job ID (e.g., `job-abc123`)

### Check job status

Use the job ID from the previous command:

```bash
cargo xtask run-cli -- status job-abc123
```

### Create and run your own Python script

1. Create a Python file (e.g., `my_script.py`):

```bash
cat > my_script.py << 'EOF'
# Simple Python script
import random

print("Generating 5 random numbers between 1 and 100:")
for _ in range(5):
    print(random.randint(1, 100))
EOF
```

2. Submit the job:

```bash
cargo xtask run-cli -- go --language python --file my_script.py
```

### Run other languages

The system supports multiple languages. Example with Node.js:

```bash
cargo xtask run-cli -- go --language node --file agent/examples/hello.js
```

Check the `backend/config/languages.json` file for available languages and versions.

## Troubleshooting

### Reset stale initramfs cache

If agent changes are not reflected inside VM:

```bash
cargo xtask reset-initramfs
```

Then restart backend (step 2).
