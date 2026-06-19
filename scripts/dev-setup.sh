#!/usr/bin/env bash
# dev-setup.sh — idempotent developer environment bootstrap
#
# Usage:
#   bash scripts/dev-setup.sh
#
# What it does:
#   1. Installs protoc (required by tonic-build / kirk-server at compile time)
#   2. Prints instructions for installing Bun (required for bench-ts)
#   3. Runs `cargo fetch` to pre-populate the registry cache
#
# Platforms: macOS (arm64 / x86_64) and Debian/Ubuntu Linux.
# This script is intentionally non-interactive and safe to re-run.

set -euo pipefail

BOLD=$'\e[1m'
GREEN=$'\e[32m'
YELLOW=$'\e[33m'
RESET=$'\e[0m'

info()  { printf "${GREEN}[setup]${RESET} %s\n" "$*"; }
warn()  { printf "${YELLOW}[warn]${RESET}  %s\n" "$*"; }

# --------------------------------------------------------------------------- #
# 1. Install protoc
# --------------------------------------------------------------------------- #

if command -v protoc >/dev/null 2>&1; then
  PROTOC_VER=$(protoc --version 2>&1 | head -1)
  info "protoc already installed: ${PROTOC_VER}"
else
  info "Installing protoc..."

  OS="$(uname -s)"
  case "$OS" in
    Darwin)
      if command -v brew >/dev/null 2>&1; then
        brew install protobuf
      else
        warn "Homebrew not found. Install Homebrew first: https://brew.sh"
        warn "Then re-run this script or run: brew install protobuf"
        exit 1
      fi
      ;;
    Linux)
      if command -v apt-get >/dev/null 2>&1; then
        sudo apt-get update -q
        sudo apt-get install -y --no-install-recommends protobuf-compiler
      elif command -v dnf >/dev/null 2>&1; then
        sudo dnf install -y protobuf-compiler
      elif command -v yum >/dev/null 2>&1; then
        sudo yum install -y protobuf-compiler
      else
        warn "Could not detect a supported package manager (apt/dnf/yum)."
        warn "Please install protoc manually: https://grpc.io/docs/protoc-installation/"
        exit 1
      fi
      ;;
    *)
      warn "Unsupported OS: $OS. Please install protoc manually."
      exit 1
      ;;
  esac

  PROTOC_VER=$(protoc --version 2>&1 | head -1)
  info "protoc installed: ${PROTOC_VER}"
fi

# --------------------------------------------------------------------------- #
# 2. Bun installation hint
# --------------------------------------------------------------------------- #

if command -v bun >/dev/null 2>&1; then
  BUN_VER=$(bun --version 2>&1 | head -1)
  info "Bun already installed: ${BUN_VER}"
else
  warn "Bun is not installed. It is required for bench-ts (typecheck + runtime)."
  warn "Install it with:"
  warn "  curl -fsSL https://bun.sh/install | bash"
  warn "Then restart your shell and re-run this script to verify."
fi

# --------------------------------------------------------------------------- #
# 3. cargo fetch — pre-populate registry cache
# --------------------------------------------------------------------------- #

WORKTREE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
info "Running cargo fetch in ${WORKTREE_ROOT}..."
(cd "$WORKTREE_ROOT" && cargo fetch)
info "cargo fetch complete."

# --------------------------------------------------------------------------- #
# Done
# --------------------------------------------------------------------------- #

printf "\n${BOLD}Dev setup complete.${RESET}\n"
printf "  Build server:  make build\n"
printf "  Run locally:   make run\n"
printf "  Full CI:       make ci\n"
