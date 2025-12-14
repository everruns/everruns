#!/bin/bash
# Cloud Agent Smoke Test - Main Entry Point
# Sets up PostgreSQL + Temporal locally and runs smoke tests without Docker
#
# Usage: ./.claude/skills/cloud-agent-smoke-tests/scripts/run-smoke-tests.sh

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/common.sh"
source "$SCRIPT_DIR/setup-postgres.sh"
source "$SCRIPT_DIR/setup-temporal.sh"

# Project root is 4 levels up from scripts folder
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"
API_PID=""
TEMPORAL_PID=""

cleanup() {
    log_info "Cleaning up..."

    # Stop API server
    if [ -n "$API_PID" ]; then
        kill "$API_PID" 2>/dev/null || true
        log_info "Stopped API server"
    fi

    # Stop Temporal server
    stop_temporal "$TEMPORAL_PID"

    # Stop PostgreSQL
    stop_postgres
}

trap cleanup EXIT

# Run database migrations
run_migrations() {
    log_info "Running database migrations..."

    cd "$PROJECT_ROOT"
    export DATABASE_URL="postgres://everruns:everruns@%2Ftmp%2Fpgdata/everruns"

    # Install sqlx-cli if not present
    if ! command -v sqlx &> /dev/null; then
        log_info "Installing sqlx-cli..."
        cargo install sqlx-cli --no-default-features --features postgres > /dev/null 2>&1
    fi

    sqlx migrate run --source crates/everruns-storage/migrations > /dev/null 2>&1
    log_info "Migrations applied"
}

# Build and start API
start_api() {
    log_info "Building and starting API server..."

    cd "$PROJECT_ROOT"
    export DATABASE_URL="postgres://everruns:everruns@%2Ftmp%2Fpgdata/everruns"
    export TEMPORAL_ADDRESS="localhost:7233"

    # Build API
    cargo build -p everruns-api > /dev/null 2>&1

    # Start API in background
    RUST_LOG=info cargo run -p everruns-api > "$API_LOG" 2>&1 &
    API_PID=$!

    # Wait for API to be ready
    log_info "Waiting for API to start..."
    for i in {1..30}; do
        if curl -s http://localhost:9000/health > /dev/null 2>&1; then
            log_info "API is ready on http://localhost:9000"
            return 0
        fi
        sleep 1
    done

    log_error "API failed to start"
    cat "$API_LOG"
    exit 1
}

# Run smoke tests
run_smoke_tests() {
    log_info "Running smoke tests..."

    cd "$PROJECT_ROOT"
    bash scripts/smoke-test.sh
}

# Main execution
main() {
    echo "==============================================="
    echo "  Cloud Agent Smoke Test"
    echo "  (PostgreSQL + Temporal, no Docker)"
    echo "==============================================="
    echo ""

    # Pre-flight checks
    check_openai_key
    check_root

    # Setup infrastructure
    check_postgres
    install_temporal
    TEMPORAL_PID=$(start_temporal)
    init_postgres
    start_postgres
    setup_database

    # Setup application
    run_migrations
    start_api

    echo ""
    echo "==============================================="
    echo "  Running Smoke Tests"
    echo "==============================================="
    echo ""

    run_smoke_tests

    echo ""
    log_info "All smoke tests completed successfully!"
    echo ""
    echo "Services running:"
    echo "  - PostgreSQL: $PGDATA (socket)"
    echo "  - Temporal:   localhost:7233"
    echo "  - API:        http://localhost:9000"
    echo ""
}

main "$@"
