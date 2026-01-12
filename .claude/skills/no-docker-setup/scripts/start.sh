#!/bin/bash
# No-Docker Smoke Tests - Deterministic Setup
# Sets up PostgreSQL and runs services without Docker
#
# Prerequisites:
#   - Root access
#   - PostgreSQL 17 installed (apt-get install postgresql-17)
#   - jq installed (apt-get install jq)
#   - OPENAI_API_KEY or ANTHROPIC_API_KEY environment variable
#
# Usage: sudo -E ./run.sh

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/_utils.sh"
source "$SCRIPT_DIR/_postgres.sh"

PROJECT_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"

# Load .env if present
if [ -f "$PROJECT_ROOT/.env" ]; then
    set -a
    source "$PROJECT_ROOT/.env"
    set +a
fi

API_PID=""
WORKER_PID=""

cleanup() {
    log_info "Cleaning up..."
    [ -n "$WORKER_PID" ] && kill "$WORKER_PID" 2>/dev/null || true
    [ -n "$API_PID" ] && kill "$API_PID" 2>/dev/null || true
    stop_postgres
}

trap cleanup EXIT

# Run database migrations
run_migrations() {
    log_info "Running migrations..."

    cd "$PROJECT_ROOT"

    # Install sqlx-cli if needed
    if ! command -v sqlx &> /dev/null; then
        log_info "Installing sqlx-cli..."
        cargo install sqlx-cli --no-default-features --features postgres > /dev/null 2>&1
    fi

    sqlx migrate run --source crates/control-plane/migrations > /dev/null 2>&1
    check_pass "Migrations - applied"
}

# Build and start API
start_api() {
    log_info "Building and starting API..."

    cd "$PROJECT_ROOT"
    export AUTH_MODE="none"

    cargo build -p everruns-control-plane > /dev/null 2>&1
    RUST_LOG=info cargo run -p everruns-control-plane > "$API_LOG" 2>&1 &
    API_PID=$!

    # Wait for API
    for i in {1..30}; do
        if curl -s http://localhost:9000/health > /dev/null 2>&1; then
            check_pass "API - ready on http://localhost:9000"
            return 0
        fi
        sleep 1
    done

    check_fail "API startup" "see $API_LOG"
    cat "$API_LOG"
    exit 1
}

# Build and start worker
start_worker() {
    log_info "Building and starting worker..."

    cd "$PROJECT_ROOT"
    export GRPC_ADDRESS="127.0.0.1:9001"

    cargo build -p everruns-worker > /dev/null 2>&1
    RUST_LOG=info cargo run -p everruns-worker > "$WORKER_LOG" 2>&1 &
    WORKER_PID=$!

    sleep 3

    if kill -0 "$WORKER_PID" 2>/dev/null; then
        check_pass "Worker - started (PID: $WORKER_PID)"
        return 0
    fi

    check_fail "Worker startup" "see $WORKER_LOG"
    cat "$WORKER_LOG"
    exit 1
}

main() {
    echo "==============================================="
    echo "  No-Docker Smoke Tests"
    echo "  Deterministic Environment Setup"
    echo "==============================================="
    echo ""

    echo "--- Prerequisites ---"
    echo ""
    check_root
    check_api_key
    set_encryption_key
    check_jq
    check_postgres_binaries

    echo ""
    echo "--- Infrastructure ---"
    echo ""
    init_postgres
    start_postgres
    setup_database

    echo ""
    echo "--- Application ---"
    echo ""
    run_migrations
    start_api
    start_worker

    echo ""
    echo "==============================================="
    echo "  Environment Ready"
    echo "==============================================="
    echo ""
    echo "Services:"
    echo "  PostgreSQL: localhost:5432 (data: $PGDATA)"
    echo "  API:        http://localhost:9000 (PID: $API_PID)"
    echo "  Worker:     PID $WORKER_PID"
    echo ""
    echo "Logs:"
    echo "  API:        $API_LOG"
    echo "  Worker:     $WORKER_LOG"
    echo "  PostgreSQL: $PG_LOGFILE"
    echo ""
    echo "Run tool-calling tests:"
    echo "  .claude/skills/smoke-test/scripts/tool-calling-tests.sh"
    echo ""
    echo "Press Ctrl+C to stop all services."
    echo ""

    wait
}

main "$@"
