#!/bin/bash
# Smoke Tests - No-Docker Mode
# Sets up PostgreSQL + Temporal locally and runs smoke tests without Docker
#
# Usage: ./.claude/skills/smoke-tests/scripts/run-no-docker.sh
#
# This script:
# 1. Detects or installs PostgreSQL (supports pre-installed versions)
# 2. Installs Temporal CLI from GitHub releases
# 3. Starts local PostgreSQL cluster and Temporal dev server
# 4. Runs database migrations
# 5. Starts API server and Temporal worker
# 6. Ready for running the test checklist

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/_utils.sh"
source "$SCRIPT_DIR/_setup-postgres.sh"
source "$SCRIPT_DIR/_setup-temporal.sh"

# Project root is 4 levels up from scripts folder
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"

# Load .env file if it exists (for API keys, encryption key, etc.)
if [ -f "$PROJECT_ROOT/.env" ]; then
    set -a
    source "$PROJECT_ROOT/.env"
    set +a
fi

API_PID=""
WORKER_PID=""
TEMPORAL_PID=""

cleanup() {
    log_info "Cleaning up..."

    # Stop Worker
    if [ -n "$WORKER_PID" ]; then
        kill "$WORKER_PID" 2>/dev/null || true
        log_info "Stopped Temporal worker"
    fi

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
    export DATABASE_URL="$(get_database_url)"

    # Install sqlx-cli if not present
    if ! command -v sqlx &> /dev/null; then
        log_info "Installing sqlx-cli..."
        cargo install sqlx-cli --no-default-features --features postgres > /dev/null 2>&1
    fi

    sqlx migrate run --source crates/everruns-storage/migrations > /dev/null 2>&1
    check_pass "Migrations - applied successfully"
}

# Build and start API
start_api() {
    log_info "Building and starting API server..."

    cd "$PROJECT_ROOT"
    export DATABASE_URL="$(get_database_url)"
    export TEMPORAL_ADDRESS="localhost:7233"
    export AUTH_MODE="none"

    # Build API (control-plane)
    cargo build -p everruns-control-plane > /dev/null 2>&1

    # Start API in background
    RUST_LOG=info cargo run -p everruns-control-plane > "$API_LOG" 2>&1 &
    API_PID=$!

    # Wait for API to be ready
    log_info "Waiting for API to start..."
    for i in {1..30}; do
        if curl -s http://localhost:9000/health > /dev/null 2>&1; then
            check_pass "API startup - ready on http://localhost:9000"
            return 0
        fi
        sleep 1
    done

    check_fail "API startup" "failed to start (see $API_LOG)"
    cat "$API_LOG"
    exit 1
}

# Build and start Temporal worker
start_worker() {
    log_info "Building and starting Temporal worker..."

    cd "$PROJECT_ROOT"
    export DATABASE_URL="$(get_database_url)"
    export TEMPORAL_ADDRESS="localhost:7233"

    # Build worker
    cargo build -p everruns-worker > /dev/null 2>&1

    # Start worker in background
    RUST_LOG=info cargo run -p everruns-worker > "$WORKER_LOG" 2>&1 &
    WORKER_PID=$!

    # Give worker a moment to start
    sleep 3

    # Check if still running
    if kill -0 "$WORKER_PID" 2>/dev/null; then
        check_pass "Worker startup - Temporal worker started (PID: $WORKER_PID)"
        return 0
    fi

    check_fail "Worker startup" "failed to start (see $WORKER_LOG)"
    cat "$WORKER_LOG"
    exit 1
}

# Main execution
main() {
    echo "==============================================="
    echo "  Smoke Tests (No-Docker Mode)"
    echo "  PostgreSQL + Temporal local setup"
    echo "==============================================="
    echo ""

    # Pre-flight checks
    check_openai_key
    check_encryption_key
    check_root

    echo ""
    echo "--- Dependencies ---"
    echo ""

    # Check and install required tools
    check_protoc
    check_jq

    echo ""
    echo "--- Infrastructure Setup ---"
    echo ""

    # Setup infrastructure
    check_postgres
    install_temporal
    init_postgres
    start_postgres
    setup_database
    TEMPORAL_PID=$(start_temporal)

    echo ""
    echo "--- Application Setup ---"
    echo ""

    # Setup application
    run_migrations
    start_api
    start_worker

    echo ""
    echo "==============================================="
    echo "  Environment Ready"
    echo "==============================================="
    echo ""
    echo "Services running:"
    if [ "$USE_SYSTEM_POSTGRES" = "true" ]; then
        echo "  - PostgreSQL: localhost:5432 (system install via pg_ctlcluster)"
    elif [ "$USE_DIRECT_POSTGRES" = "true" ]; then
        echo "  - PostgreSQL: localhost:5432 (direct pg_ctl, version $PG_VERSION)"
    else
        echo "  - PostgreSQL: $PGDATA (socket)"
    fi
    echo "  - Temporal:   localhost:7233 (PID: $TEMPORAL_PID)"
    echo "  - API:        http://localhost:9000 (PID: $API_PID)"
    echo "  - Worker:     PID $WORKER_PID"
    echo ""
    echo "Logs:"
    echo "  - API:        $API_LOG"
    echo "  - Worker:     $WORKER_LOG"
    echo "  - Temporal:   $TEMPORAL_LOG"
    if [ -f "$PG_LOGFILE" ]; then
        echo "  - PostgreSQL: $PG_LOGFILE"
    fi
    echo ""
    echo "Run smoke tests using the checklist in:"
    echo "  .claude/skills/smoke-test/SKILL.md"
    echo ""
    echo "Press Ctrl+C to stop all services."
    echo ""

    # Keep running until interrupted
    wait
}

main "$@"
