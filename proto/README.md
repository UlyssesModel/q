# proto/

`proto/kirk.proto` is the single source of truth for the gRPC schema.

## Consumers

- **`kirk-server/build.rs`** — invokes `tonic-build::compile_protos("../proto/kirk.proto")` to generate Rust server/client stubs at compile time. Run `cargo build -p kirk-server` to regenerate.
- **`bench-ts/`** — loads `../proto/kirk.proto` at runtime via `@grpc/proto-loader` (no codegen step required).

## Package

`package kirk.v1` — service `KirkService` with 7 RPCs.

## Editing

Changing `kirk.proto` requires:
1. Rebuilding the server (`cargo build -p kirk-server`) so `tonic-build` regenerates the Rust stubs.
2. Restarting the bench (it loads the proto at runtime; no additional step needed).

There is no `buf.gen.yaml` or `buf.work.yaml` in this repo. Proto management is intentionally minimal — `tonic-build` and `proto-loader` are both configured to find the file at `../proto/kirk.proto` relative to their respective crate/package roots.
