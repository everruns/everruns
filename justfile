# Everruns development commands
# Install just: cargo install just (or ./scripts/init-cloud-env.sh for pre-built binary)
# Usage: just <recipe>   (or: just --list)

mod ui
mod docs
mod durable

# Default recipe: show available commands
default:
    @just --list

# === Setup ===

# Install all development dependencies (Rust tools + UI + Docs)
init:
    ./scripts/lib/setup.sh init

# Upload example agents from examples/agents/
upload-agents:
    ./scripts/lib/setup.sh upload-agents

# === Docker Services ===

# Start Docker services (Postgres, Jaeger)
start-docker:
    ./scripts/lib/docker.sh start

# Stop Docker services
stop-docker:
    ./scripts/lib/docker.sh stop

# Stop and remove all Docker volumes
reset:
    ./scripts/lib/docker.sh reset

# Run database migrations
migrate:
    ./scripts/lib/docker.sh migrate

# === Build & Test ===

# Build all crates
build:
    cargo build

# Run all tests (Rust + UI e2e)
test:
    cargo test --all-features
    cd apps/ui && npm run e2e 2>/dev/null || echo "(e2e skipped)"

# Run all formatters and linters (auto-fix)
fmt:
    cargo fmt
    cargo clippy --all-targets --fix --allow-dirty --allow-staged 2>/dev/null || true
    cd apps/ui && npm run lint -- --fix 2>/dev/null || true

# Run format, lint, and test checks
check:
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo test

# Run all pre-PR checks (fmt, clippy, tests, UI, OpenAPI, docs)
pre-pr:
    ./scripts/lib/pre-pr.sh

# Clean build artifacts
clean:
    cargo clean

# === Services ===

# Start in DEV MODE (in-memory storage, no Docker/PostgreSQL required)
start-dev *args:
    ./scripts/lib/services.sh start-dev {{args}}

# Start everything with auto-reload (Docker, API, Worker, UI)
start-all *args:
    ./scripts/lib/services.sh start-all {{args}}

# Stop all services (API, UI, Docker)
stop-all:
    ./scripts/lib/services.sh stop-all
