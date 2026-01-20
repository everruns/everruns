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

# === Rust Build ===

# Build all crates
build:
    ./scripts/lib/rust.sh build

# Run tests
test:
    ./scripts/lib/rust.sh test

# Run format, lint, and test checks
check:
    ./scripts/lib/rust.sh check

# Run all pre-PR checks (fmt, clippy, tests, UI, OpenAPI, docs)
pre-pr:
    ./scripts/lib/rust.sh pre-pr

# Clean build artifacts
clean:
    ./scripts/lib/rust.sh clean

# === Services ===

# Start in DEV MODE (in-memory storage, no Docker/PostgreSQL required)
start-dev:
    ./scripts/lib/services.sh start-dev

# Start everything with auto-reload (Docker, API, Worker, UI)
start-all:
    ./scripts/lib/services.sh start-all

# Stop all services (API, UI, Docker)
stop-all:
    ./scripts/lib/services.sh stop-all
