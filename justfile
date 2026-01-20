# Everruns development commands
# Wrapper around scripts/dev.sh - see that file for implementation details
#
# Install just: cargo install just
# Usage: just <recipe>   (or: just --list)

# Default recipe: show help
default:
    @./scripts/dev.sh help

# Install all development dependencies (Rust tools + UI + Docs)
init:
    ./scripts/dev.sh init

# Start Docker services (Postgres, Jaeger)
start:
    ./scripts/dev.sh start

# Stop Docker services
stop:
    ./scripts/dev.sh stop

# Start in DEV MODE (in-memory storage, no Docker/PostgreSQL required)
start-dev:
    ./scripts/dev.sh start-dev

# Start everything with auto-reload (Docker, API, Worker, UI)
start-all:
    ./scripts/dev.sh start-all

# Stop all services (API, UI, Docker)
stop-all:
    ./scripts/dev.sh stop-all

# Stop and remove all Docker volumes
reset:
    ./scripts/dev.sh reset

# Run database migrations
migrate:
    ./scripts/dev.sh migrate

# Upload example agents from examples/agents/
upload-agents:
    ./scripts/dev.sh upload-agents

# Patch API keys for providers
seed *args:
    ./scripts/dev.sh seed {{ args }}

# Build all crates
build:
    ./scripts/dev.sh build

# Run tests
test:
    ./scripts/dev.sh test

# Run format, lint, and test checks
check:
    ./scripts/dev.sh check

# Run all pre-PR checks (fmt, clippy, tests, UI, OpenAPI, docs)
pre-pr:
    ./scripts/dev.sh pre-pr

# Start the control-plane server
control-plane:
    ./scripts/dev.sh control-plane

# Alias for control-plane
api: control-plane

# Start the worker
worker:
    ./scripts/dev.sh worker

# Start control-plane with auto-reload
watch-control-plane:
    ./scripts/dev.sh watch-control-plane

# Alias for watch-control-plane
watch-api: watch-control-plane

# Start worker with auto-reload
watch-worker:
    ./scripts/dev.sh watch-worker

# Start the UI development server
ui:
    ./scripts/dev.sh ui

# Build the UI for production
ui-build:
    ./scripts/dev.sh ui-build

# Install UI dependencies
ui-install:
    ./scripts/dev.sh ui-install

# Run UI e2e tests (Playwright)
e2e:
    ./scripts/dev.sh e2e

# Take UI screenshots for visual verification
e2e-screenshots:
    ./scripts/dev.sh e2e-screenshots

# Start the docs development server
docs:
    ./scripts/dev.sh docs

# Build the docs for production
docs-build:
    ./scripts/dev.sh docs-build

# Install docs dependencies
docs-install:
    ./scripts/dev.sh docs-install

# View Docker service logs
logs:
    ./scripts/dev.sh logs

# Clean build artifacts and Docker volumes
clean:
    ./scripts/dev.sh clean

# Run durable benchmarks (in-memory)
durable-bench *args:
    ./scripts/dev.sh durable-bench {{ args }}

# Run durable benchmarks (PostgreSQL)
durable-bench-db *args:
    ./scripts/dev.sh durable-bench-db {{ args }}

# Show help
help:
    @./scripts/dev.sh help
