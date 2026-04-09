# contracts

Shared API contracts between services.

## Protobuf (gRPC)

- `proto/optima/v1/kernel.proto` — `Kernel` service: `CheckConflicts`, `Health`

### Regenerate Go stubs

From repo root (requires `protoc`, `protoc-gen-go`, `protoc-gen-go-grpc` on `PATH`):

```bash
export PATH="$PATH:$(go env GOPATH)/bin"
cd go-orchestrator
protoc --go_out=. --go_opt=module=optima/go-orchestrator \
  --go-grpc_out=. --go-grpc_opt=module=optima/go-orchestrator \
  -I ../contracts/proto ../contracts/proto/optima/v1/kernel.proto
```

Or use `make proto-go` from the repo root.

Rust stubs are generated at build time via `rust-kernel/build.rs`.

## Other

- JSON Schema / Avro (optional, for data lake exports)
