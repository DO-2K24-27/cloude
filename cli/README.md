# CLI

## How to use the CLI ?

Run a job with the CLI:
```
cargo run -p cli -- go --language python --file agent/examples/hello.py
```

Check job status and result:
```
cargo run -p cli -- status <JOB_ID>
```

## Using distant backend

```
cargo run -p cli -- --backend-url http://<BACKEND_IP>:8080 go --language python --file agent/examples/hello.py
```