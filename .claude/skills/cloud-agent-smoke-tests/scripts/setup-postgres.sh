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

# Install PostgreSQL from PGDG repository
install_postgres() {
    log_info "Installing PostgreSQL $PG_VERSION from PGDG repository..."

    # Add PostgreSQL APT repository
    apt-get install -y curl ca-certificates gnupg > /dev/null 2>&1
    if ! curl -fsSL https://www.postgresql.org/media/keys/ACCC4CF8.asc 2>/dev/null | gpg --dearmor -o /usr/share/keyrings/postgresql-keyring.gpg 2>/dev/null; then
        log_warn "Failed to fetch PostgreSQL GPG key (network issue)"
        return 1
    fi
    echo "deb [signed-by=/usr/share/keyrings/postgresql-keyring.gpg] http://apt.postgresql.org/pub/repos/apt $(lsb_release -cs)-pgdg main" > /etc/apt/sources.list.d/pgdg.list

    # Install PostgreSQL
    apt-get update > /dev/null 2>&1
    apt-get install -y postgresql-$PG_VERSION > /dev/null 2>&1

    # Stop the auto-started service (we'll run our own cluster)
    systemctl stop postgresql@$PG_VERSION-main 2>/dev/null || true
    systemctl disable postgresql@$PG_VERSION-main 2>/dev/null || true

    log_info "PostgreSQL $PG_VERSION installed"
}

# Check if PostgreSQL is installed
check_postgres() {
    # Try preferred version first
    if [ -f "$PG_BIN/initdb" ]; then
        log_info "PostgreSQL $PG_VERSION found at $PG_BIN"
        export NEED_UUIDV7_POLYFILL="false"
        return 0
    fi

    # Try to install preferred version
    log_warn "PostgreSQL $PG_VERSION binaries not found at $PG_BIN"
    if install_postgres && [ -f "$PG_BIN/initdb" ]; then
        export NEED_UUIDV7_POLYFILL="false"
        return 0
    fi

    # Fall back to PostgreSQL 16 if available
    if [ -f "/usr/lib/postgresql/16/bin/initdb" ]; then
        log_warn "Falling back to PostgreSQL 16 (will use UUIDv7 polyfill)"
        export PG_VERSION="16"
        export PG_BIN="/usr/lib/postgresql/16/bin"
        export NEED_UUIDV7_POLYFILL="true"
        log_info "PostgreSQL 16 found at $PG_BIN"
        return 0
    fi

    log_error "No PostgreSQL installation found and unable to install"
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

# Install UUIDv7 polyfill for PostgreSQL < 17
install_uuidv7() {
    if [ "$NEED_UUIDV7_POLYFILL" != "true" ]; then
        log_info "PostgreSQL $PG_VERSION has native uuidv7() - no polyfill needed"
        return 0
    fi

    log_info "Installing UUIDv7 polyfill for PostgreSQL $PG_VERSION..."

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

    log_info "UUIDv7 polyfill installed"
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
