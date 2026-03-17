# CLI

The `CLI` provides a command-line interface for interacting with the Cloude system. It allows users to submit jobs, query their status, and manage resources.

## How to Start

### Run a Job

```bash
cargo run -p cli -- go --language python --file agent/examples/hello.py
```

### Check Job Status

```bash
cargo run -p cli -- status <JOB_ID>
```

### Using Remote Backend

```bash
cargo run -p cli -- --backend-url http://<BACKEND_IP>:8080 go --language python --file agent/examples/hello.py
```

For detailed documentation, refer to [docs/cli.md](../docs/cli.md).