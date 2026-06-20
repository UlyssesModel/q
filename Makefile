SHELL := /bin/bash

# Targets:
#   make build          — cargo build --release for the workspace
#   make test           — cargo test --workspace (lib + bins + integration)
#   make run            — run kirk-server locally on default ports
#   make fmt            — cargo fmt --all (format in place)
#   make lint           — cargo clippy --workspace --all-targets -- -D warnings
#   make ci             — full local CI: fmt-check, lint, test, typecheck
#   make image          — docker build the server image
#   make up             — docker compose up -d kirk-server (with healthcheck)
#   make down           — docker compose down
#   make bench-rest     — TRANSPORT=rest docker compose run --rm bench
#   make bench-grpc     — TRANSPORT=grpc docker compose run --rm bench
#   make bench-tcp      — TRANSPORT=tcp  docker compose run --rm bench
#   make bench-all      — run all three transports back-to-back
#   make bench-compare  — run all three transports sequentially and print compare
#   make compare        — bun src/cli.ts compare results/*.json
#   make proto-sync     — copy proto/kirk.proto into bench-ts/proto/
#   make clean                 — cargo clean + remove bench-ts/results + node_modules
#   make check-secure-isolation — assert default build does not resolve secret-kirk-edge
#   make build-secure          — print operator workflow for the secret-kirk-edge feature

WORKERS     ?= 0
USERS       ?= 100
DURATION    ?= 30s
MATRIX_SIZE ?= 32
OP          ?= forward

.PHONY: help build test run fmt lint ci image up down \
        bench-rest bench-grpc bench-tcp bench-all bench-compare \
        compare proto-sync clean \
        check-secure-isolation build-secure

help:  ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*## ' $(MAKEFILE_LIST) \
	  | awk 'BEGIN {FS = ":.*## "}; {printf "  %-18s %s\n", $$1, $$2}'

build:  ## Build the workspace (release)
	cargo build --release --workspace

test:  ## Run all tests (lib + bins + integration, single-threaded to avoid TIME_WAIT)
	cargo test --workspace --lib --bins
	cargo test --workspace --tests -- --test-threads=1

run:  ## Run kirk-server locally on default ports
	cargo run --release -p kirk-server -- --workers $(WORKERS)

fmt:  ## Format all Rust code in place
	cargo fmt --all

lint:  ## Run clippy (deny warnings)
	cargo clippy --workspace --all-targets -- -D warnings

ci:  ## Full local CI: fmt-check, lint, lib/bin tests, integration tests, typecheck
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets -- -D warnings
	cargo test --workspace --lib --bins
	cargo test --workspace --tests -- --test-threads=1
	@if command -v bun >/dev/null 2>&1; then \
	  cd bench-ts && bun install && bun run typecheck; \
	else \
	  echo "[WARN] bun not found — skipping typecheck (install bun to run locally)"; \
	fi

image:  ## Build the server Docker image
	docker build -f docker/Dockerfile -t kirk-server:dev .

up:  ## Start kirk-server in the background via docker compose
	docker compose up -d kirk-server

down:  ## Stop and remove docker compose services
	docker compose down

bench-rest:  ## Run REST benchmark via docker compose
	TRANSPORT=rest USERS=$(USERS) DURATION=$(DURATION) MATRIX_SIZE=$(MATRIX_SIZE) OP=$(OP) docker compose run --rm bench

bench-grpc:  ## Run gRPC benchmark via docker compose
	TRANSPORT=grpc USERS=$(USERS) DURATION=$(DURATION) MATRIX_SIZE=$(MATRIX_SIZE) OP=$(OP) docker compose run --rm bench

bench-tcp:  ## Run TCP benchmark via docker compose
	TRANSPORT=tcp USERS=$(USERS) DURATION=$(DURATION) MATRIX_SIZE=$(MATRIX_SIZE) OP=$(OP) docker compose run --rm bench

bench-all: bench-rest bench-grpc bench-tcp  ## Run all three transport benchmarks

bench-compare:  ## Run all three transports sequentially and print a comparison table
	@echo "=== REST ===" && $(MAKE) bench-rest
	@echo "=== gRPC ===" && $(MAKE) bench-grpc
	@echo "=== TCP  ===" && $(MAKE) bench-tcp
	@echo ""
	@echo "=== Comparison ===" && $(MAKE) compare

compare:  ## Compare result files with bun bench-ts compare
	cd bench-ts && bun src/cli.ts compare results/*.json

proto-sync:  ## Copy proto/kirk.proto into bench-ts/proto/ (keeps client proto in sync)
	mkdir -p bench-ts/proto
	cp proto/kirk.proto bench-ts/proto/kirk.proto

check-secure-isolation:  ## Assert the default build does not resolve the secret-kirk-edge dep
	bash scripts/check-default-build-clean.sh

build-secure:  ## Print operator instructions for building with the secret-kirk-edge feature
	@echo ""
	@echo "=== Secure Build (secret-kirk-edge feature) ==="
	@echo ""
	@echo "The 'secret-kirk-edge' feature requires Tailnet access and a local Cargo.toml"
	@echo "patch that declares the private git dependency. CI and Docker MUST NOT use"
	@echo "this target. See docs/SECURE_BUILD.md for the full operator workflow."
	@echo ""
	@echo "Summary of steps:"
	@echo "  1. Ensure you have active Tailnet access to the private git host."
	@echo "  2. Follow the patch-dep instructions in docs/SECURE_BUILD.md."
	@echo "  3. Run: cargo build --release -p kirk-server --features secret-kirk-edge"
	@echo "  4. Run: bash scripts/check-default-build-clean.sh  (must still pass)"
	@echo ""
	@echo "NEVER commit Cargo.toml changes that include the git URL."
	@echo ""

clean:  ## Remove build artifacts, bench results, and node_modules
	cargo clean
	rm -rf bench-ts/results/*.json
	rm -rf bench-ts/node_modules
