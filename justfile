# Everruns development commands
# Install just: cargo install just (or ./scripts/init-cloud-env.sh for pre-built binary)
# Usage: just <recipe>   (or: just --list)

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

# Patch API keys for providers
seed *args:
    ./scripts/lib/setup.sh seed {{ args }}

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

# === UI ===

# Build the UI for production
ui-build:
    ./scripts/lib/ui.sh build

# Run UI e2e tests (Playwright)
e2e:
    ./scripts/lib/ui.sh e2e

# Take UI screenshots for visual verification
e2e-screenshots:
    ./scripts/lib/ui.sh screenshots

# === Docs ===

# Start the docs development server
docs:
    ./scripts/lib/docs.sh dev

# Build the docs for production
docs-build:
    ./scripts/lib/docs.sh build

# === Benchmarks ===

# Run durable benchmarks (in-memory)
durable-bench *args:
    ./scripts/lib/bench.sh memory {{ args }}

# Run durable benchmarks (PostgreSQL)
durable-bench-db *args:
    ./scripts/lib/bench.sh db {{ args }}
