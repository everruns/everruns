# Everruns development commands
# Install just: cargo install just (or ./scripts/init-cloud-env.sh for pre-built binary)
# Usage: just <recipe>   (or: just --list)

mod ui
mod docs
mod durable
mod example

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

# === Build & Test ===

# Build all crates
build:
    cargo build

# Run all tests (Rust + UI e2e)
test:
    cargo test --all-features
    cd apps/ui && npm run e2e 2>/dev/null || echo "(e2e skipped)"

# Run pure unit tests (no PostgreSQL required) - fast feedback
test-unit:
    cargo test -p everruns-anthropic --lib --all-features
    cargo test -p everruns-openai --lib --all-features
    cargo test -p everruns-internal-protocol --lib --all-features
    cargo test -p everruns-core --lib --all-features

# Run integration tests (requires PostgreSQL via Docker or start-dev)
test-integration: start-docker
    #!/usr/bin/env bash
    set -e
    # Wait for postgres
    for i in {1..30}; do
        if pg_isready -h localhost -p 5432 -U everruns 2>/dev/null; then break; fi
        sleep 1
    done
    # Run migrations
    sqlx migrate run --source crates/server/migrations 2>/dev/null || true
    # Run tests
    cargo test -p everruns-server --lib
    cargo test -p everruns-server --test api_integration_test -- --test-threads=1
    cargo test -p everruns-server --test repository_integration_test -- --test-threads=1
    cargo test -p everruns-durable --test postgres_integration_test --features postgres-tests -- --test-threads=1
    cargo test -p everruns-durable --test postgres_repository_test --features postgres-tests -- --test-threads=1

# Run workflow tests (requires running server + worker)
test-workflow:
    cargo test -p everruns-server --test workflow_test -- --test-threads=1

# Run LLM tests against real APIs (requires ANTHROPIC_API_KEY, OPENAI_API_KEY)
# Skip providers: SKIP_LLM_INTEGRATION_TESTS_PROVIDERS=gemini just test-llm
test-llm:
    cargo test -p everruns-core --test agent_run_basic
    cargo test -p everruns-core --test agent_run_with_thinking

# Run all formatters and linters (auto-fix)
fmt:
    cargo fmt
    cargo clippy --all-targets --fix --allow-dirty --allow-staged 2>/dev/null || true
    cd apps/ui && npm run format 2>/dev/null || true
    cd apps/ui && npm run lint -- --fix 2>/dev/null || true

# Run format, lint, and test checks
check:
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo test

# Run fast pre-push checks (fmt, lint, lockfile — ~30s)
pre-push:
    ./scripts/lib/pre-push.sh

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

# Start in PRODUCTION MODE (release builds, no watchers, production UI)
start-production *args:
    ./scripts/lib/services.sh start-production {{args}}

# Stop all services (API, UI, Docker)
stop-all:
    ./scripts/lib/services.sh stop-all
