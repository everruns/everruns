#!/bin/bash
# PostgreSQL setup for Cloud Agent smoke tests
# Sets up a local PostgreSQL cluster without Docker
#
# This script can be:
# - Sourced by run.sh (common.sh must be sourced first)
# - Executed directly (will source common.sh itself)

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    # Running directly - source common.sh
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    source "$SCRIPT_DIR/common.sh"
    set -e
fi

# Check if PostgreSQL is installed
check_postgres() {
    if [ ! -f "$PG_BIN/initdb" ]; then
        log_error "PostgreSQL binaries not found at $PG_BIN"
        log_info "Install PostgreSQL: apt install postgresql-16"
        exit 1
    fi
    log_info "PostgreSQL found at $PG_BIN"
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
    touch "$PG_LOGFILE"
    chown postgres:postgres "$PG_LOGFILE"

    # Start server
    su - postgres -c "export PATH=$PG_BIN:\$PATH && pg_ctl -D $PGDATA -l $PG_LOGFILE start" > /dev/null 2>&1

    # Wait for startup
    for i in {1..10}; do
        if pg_isready -h "$PGDATA" > /dev/null 2>&1; then
            log_info "PostgreSQL is ready"
            return 0
        fi
        sleep 1
    done

    log_error "PostgreSQL failed to start"
    cat "$PG_LOGFILE"
    exit 1
}

# Stop PostgreSQL
stop_postgres() {
    if [ -d "$PGDATA" ]; then
        su - postgres -c "export PATH=$PG_BIN:\$PATH && pg_ctl -D $PGDATA stop -m fast" 2>/dev/null || true
        log_info "Stopped PostgreSQL"
    fi
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

# Full PostgreSQL setup
setup_all() {
    check_root
    check_postgres
    init_postgres
    start_postgres
    setup_database
    install_uuidv7
}

# Run if executed directly
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    setup_all
fi
