# syntax=docker/dockerfile:1.7
FROM rust:1-bookworm AS builder

# Install protoc (tonic-build requires it at compile time).
RUN apt-get update \
  && apt-get install -y --no-install-recommends protobuf-compiler ca-certificates \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /workspace

# Copy the workspace into the builder. The .dockerignore should keep target/
# and node_modules out so this is reasonably small.
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY proto ./proto
COPY kirk-stub-realistic ./kirk-stub-realistic
COPY kirk-stub-kirk ./kirk-stub-kirk
COPY kirk-server ./kirk-server

# Build only the release binary.
#
# Default features ONLY. The shipped image must never enable the gated
# secure feature; see docs/SECURE_BUILD.md for the operator workflow.
RUN cargo build --release -p kirk-server

# Distroless runtime — no shell, just the binary.
FROM gcr.io/distroless/cc-debian12 AS runtime
WORKDIR /app
COPY --from=builder /workspace/target/release/kirk-server /usr/local/bin/kirk-server
USER 10001
EXPOSE 50051 8080 9090
ENV RUST_LOG=info
ENTRYPOINT ["/usr/local/bin/kirk-server"]
