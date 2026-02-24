#!/bin/bash
# No-Docker Setup - Deterministic PostgreSQL + Caddy + Full Backend
# Sets up PostgreSQL and Caddy from scratch, then starts API + Worker without Docker
#
# Prerequisites:
#   - Root access (for PostgreSQL cluster initialization)
#   - PostgreSQL 16+ installed
#   - jq installed
#   - OPENAI_API_KEY or ANTHROPIC_API_KEY environment variable
#
# Options:
#   --no-caddy   Skip Caddy reverse proxy (access API directly on :9000)
#
# Usage: sudo -E .claude/skills/no-docker-setup/scripts/start.sh
#        sudo -E .claude/skills/no-docker-setup/scripts/start.sh --no-caddy

set -e

# Add cargo bin to PATH (for just command)
export PATH="$HOME/.cargo/bin:$PATH"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/_utils.sh"
source "$SCRIPT_DIR/_postgres.sh"
source "$SCRIPT_DIR/_caddy.sh"

PROJECT_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"

# Parse args
USE_CADDY=true
for arg in "$@"; do
    case "$arg" in
        --no-caddy) USE_CADDY=false ;;
    esac
done

# Load .env if present
if [ -f "$PROJECT_ROOT/.env" ]; then
    set -a
    source "$PROJECT_ROOT/.env"
    set +a
fi

cleanup() {
    log_info "Cleaning up..."
    pkill -f "everruns-server" 2>/dev/null || true
    pkill -f "everruns-worker" 2>/dev/null || true
    stop_caddy
    stop_postgres
}

trap cleanup EXIT

main() {
    echo "==============================================="
    echo "  No-Docker Setup"
    echo "  Full Backend (PostgreSQL + Caddy + API + Worker)"
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
    echo "--- PostgreSQL Setup ---"
    echo ""
    init_postgres
    start_postgres
    setup_database

    if [ "$USE_CADDY" = true ]; then
        echo ""
        echo "--- Caddy Reverse Proxy ---"
        echo ""
        install_caddy
        start_caddy
        echo ""
        log_info "API accessible at http://localhost:9300/api (via Caddy)"
        log_info "API also directly at http://localhost:9000 (bypassing Caddy)"
    fi

    echo ""
    echo "--- Starting Services ---"
    echo ""

    cd "$PROJECT_ROOT"

    # Use just start-all with flags for no-docker setup
    # API server auto-applies migrations on startup
    export AUTH_MODE="none"
    export RUST_LOG=${RUST_LOG:-info}

    just start-all --no-watch --no-docker --no-ui
}

main "$@"
