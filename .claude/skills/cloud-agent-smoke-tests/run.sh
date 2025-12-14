#!/bin/bash
# Cloud Agent Smoke Test Script
# This script sets up PostgreSQL and Temporal locally and runs smoke tests
# without requiring Docker. Designed for Cloud Agent environments.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
PGDATA="/tmp/pgdata"
LOGFILE="$PGDATA/pg.log"
PG_BIN="/usr/lib/postgresql/16/bin"
TEMPORAL_BIN="/usr/local/bin/temporal"
TEMPORAL_LOG="/tmp/temporal.log"
API_PID=""
TEMPORAL_PID=""

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

cleanup() {
    log_info "Cleaning up..."

    # Stop API server
    if [ -n "$API_PID" ]; then
        kill "$API_PID" 2>/dev/null || true
        log_info "Stopped API server"
    fi

    # Stop Temporal server
    if [ -n "$TEMPORAL_PID" ]; then
        kill "$TEMPORAL_PID" 2>/dev/null || true
        log_info "Stopped Temporal server"
    fi

    # Stop PostgreSQL
    if [ -d "$PGDATA" ]; then
        su - postgres -c "export PATH=$PG_BIN:\$PATH && pg_ctl -D $PGDATA stop -m fast" 2>/dev/null || true
        log_info "Stopped PostgreSQL"
    fi
}

trap cleanup EXIT

# Check if running as root
check_root() {
    if [ "$(id -u)" -ne 0 ]; then
        log_error "This script must be run as root to initialize PostgreSQL"
        exit 1
    fi
}

# Check if PostgreSQL is installed
check_postgres() {
    if [ ! -f "$PG_BIN/initdb" ]; then
        log_error "PostgreSQL binaries not found at $PG_BIN"
        log_info "Install PostgreSQL: apt install postgresql-16"
        exit 1
    fi
    log_info "PostgreSQL found at $PG_BIN"
}

# Install Temporal CLI if not present
install_temporal() {
    if [ -f "$TEMPORAL_BIN" ]; then
        log_info "Temporal CLI already installed"
        return 0
    fi

    log_info "Installing Temporal CLI..."

    # Download Temporal CLI binary for Linux x86_64
    curl -sL "https://temporal.download/cli/archive/latest?platform=linux&arch=amd64" -o /tmp/temporal.tar.gz

    # Extract and install
    tar -xzf /tmp/temporal.tar.gz -C /tmp
    mv /tmp/temporal "$TEMPORAL_BIN"
    chmod +x "$TEMPORAL_BIN"
    rm -f /tmp/temporal.tar.gz

    log_info "Temporal CLI installed: $($TEMPORAL_BIN --version)"
}

# Start Temporal dev server
start_temporal() {
    log_info "Starting Temporal dev server..."

    # Start Temporal dev server in background (uses in-memory SQLite)
    "$TEMPORAL_BIN" server start-dev --headless > "$TEMPORAL_LOG" 2>&1 &
    TEMPORAL_PID=$!

    # Wait for Temporal to be ready
    for i in {1..30}; do
        if nc -z localhost 7233 2>/dev/null; then
            log_info "Temporal is ready on localhost:7233"
            return 0
        fi
        sleep 1
    done

    log_error "Temporal failed to start"
    cat "$TEMPORAL_LOG"
    exit 1
}

# Initialize PostgreSQL cluster
init_postgres() {
    log_info "Initializing PostgreSQL cluster..."

    # Clean up previous data
    rm -rf "$PGDATA"
    mkdir -p "$PGDATA"
    chown postgres:postgres "$PGDATA"

    # Initialize cluster
    su - postgres -c "export PATH=$PG_BIN:\$PATH && initdb -D $PGDATA --auth=trust" > /dev/null 2>&1

    # Configure socket directory
    su - postgres -c "echo \"unix_socket_directories = '$PGDATA'\" >> $PGDATA/postgresql.conf"

    log_info "PostgreSQL cluster initialized"
}

# Start PostgreSQL
start_postgres() {
    log_info "Starting PostgreSQL..."

    # Create log file with correct permissions
    touch "$LOGFILE"
    chown postgres:postgres "$LOGFILE"

    # Start server
    su - postgres -c "export PATH=$PG_BIN:\$PATH && pg_ctl -D $PGDATA -l $LOGFILE start" > /dev/null 2>&1

    # Wait for startup
    for i in {1..10}; do
        if pg_isready -h "$PGDATA" > /dev/null 2>&1; then
            log_info "PostgreSQL is ready"
            return 0
        fi
        sleep 1
    done

    log_error "PostgreSQL failed to start"
    cat "$LOGFILE"
    exit 1
}

# Create database and user
setup_database() {
    log_info "Setting up database..."

    # Create user and database
    su - postgres -c "export PATH=$PG_BIN:\$PATH && psql -h $PGDATA -c \"CREATE USER everruns WITH PASSWORD 'everruns';\"" > /dev/null 2>&1
    su - postgres -c "export PATH=$PG_BIN:\$PATH && psql -h $PGDATA -c \"CREATE DATABASE everruns OWNER everruns;\"" > /dev/null 2>&1
    su - postgres -c "export PATH=$PG_BIN:\$PATH && psql -h $PGDATA -c \"GRANT ALL PRIVILEGES ON DATABASE everruns TO everruns;\"" > /dev/null 2>&1

    log_info "Database 'everruns' created"
}

# Install UUIDv7 polyfill for PostgreSQL < 18
install_uuidv7() {
    log_info "Installing UUIDv7 polyfill..."

    su - postgres -c "export PATH=$PG_BIN:\$PATH && psql -h $PGDATA -d everruns" << 'EOF' > /dev/null 2>&1
CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE OR REPLACE FUNCTION uuidv7() RETURNS uuid AS $$
DECLARE
  unix_ts_ms BIGINT;
  uuid_bytes BYTEA;
BEGIN
  unix_ts_ms := (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::BIGINT;

  uuid_bytes :=
    set_byte(set_byte(set_byte(set_byte(set_byte(set_byte(
      gen_random_bytes(16),
      0, ((unix_ts_ms >> 40) & 255)::INT),
      1, ((unix_ts_ms >> 32) & 255)::INT),
      2, ((unix_ts_ms >> 24) & 255)::INT),
      3, ((unix_ts_ms >> 16) & 255)::INT),
      4, ((unix_ts_ms >> 8) & 255)::INT),
      5, (unix_ts_ms & 255)::INT);

  uuid_bytes := set_byte(uuid_bytes, 6, (get_byte(uuid_bytes, 6) & 15) | 112);
  uuid_bytes := set_byte(uuid_bytes, 8, (get_byte(uuid_bytes, 8) & 63) | 128);

  RETURN encode(uuid_bytes, 'hex')::uuid;
END;
$$ LANGUAGE plpgsql VOLATILE;
EOF

    log_info "UUIDv7 function installed"
}

# Run migrations
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
    RUST_LOG=info cargo run -p everruns-api > /tmp/api.log 2>&1 &
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
    cat /tmp/api.log
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
    echo "  Cloud Agent Smoke Test Setup"
    echo "  (PostgreSQL + Temporal, no Docker)"
    echo "==============================================="
    echo ""

    check_root
    check_postgres
    install_temporal
    start_temporal
    init_postgres
    start_postgres
    setup_database
    install_uuidv7
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
    echo "  - PostgreSQL: /tmp/pgdata (socket)"
    echo "  - Temporal:   localhost:7233"
    echo "  - API:        http://localhost:9000"
    echo ""
}

main "$@"
