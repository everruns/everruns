#!/bin/bash
# No-Docker Setup - Deterministic PostgreSQL + Full Backend
# Sets up PostgreSQL from scratch and starts API + Worker without Docker
#
# Prerequisites:
#   - Root access (for PostgreSQL cluster initialization)
#   - PostgreSQL 16+ installed
#   - jq installed
#   - OPENAI_API_KEY or ANTHROPIC_API_KEY environment variable
#
# Usage: sudo -E .claude/skills/no-docker-setup/scripts/start.sh

set -e

# Add cargo bin to PATH (for just command)
export PATH="$HOME/.cargo/bin:$PATH"

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

cleanup() {
    log_info "Cleaning up..."
    pkill -f "everruns-control-plane" 2>/dev/null || true
    pkill -f "everruns-worker" 2>/dev/null || true
    stop_postgres
}

trap cleanup EXIT

main() {
    echo "==============================================="
    echo "  No-Docker Setup"
    echo "  Full Backend (PostgreSQL + API + Worker)"
    echo "==============================================="
    echo ""

    echo "--- Prerequisites ---"
    echo ""
    check_root
    check_api_key
    set_encryption_key
    check_jq
    check_postgres_binaries
    ensure_sqlx

    echo ""
    echo "--- PostgreSQL Setup ---"
    echo ""
    init_postgres
    start_postgres
    setup_database

    echo ""
    echo "--- Starting Services ---"
    echo ""

    cd "$PROJECT_ROOT"

    # Use just start-all with flags for no-docker setup
    # This runs migrations, API, and Worker
    export AUTH_MODE="none"
    export RUST_LOG=${RUST_LOG:-info}

    just start-all --no-watch --no-docker --no-ui
}

main "$@"
