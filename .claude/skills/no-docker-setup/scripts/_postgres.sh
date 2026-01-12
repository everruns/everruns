#!/bin/bash
# Deterministic PostgreSQL setup for no-docker smoke tests
# Uses fixed paths and always creates a fresh cluster

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    source "$SCRIPT_DIR/_utils.sh"
    set -e
fi

# Kill any existing postgres processes using our data directory
kill_existing_postgres() {
    if [ -f "$PGDATA/postmaster.pid" ]; then
        local pid=$(head -1 "$PGDATA/postmaster.pid" 2>/dev/null || true)
        if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
            log_info "Stopping existing PostgreSQL (PID: $pid)..."
            kill "$pid" 2>/dev/null || true
            sleep 2
        fi
    fi
    # Also check for any postgres on port 5432
    local port_pid=$(lsof -ti :5432 2>/dev/null || true)
    if [ -n "$port_pid" ]; then
        log_warn "Killing process on port 5432 (PID: $port_pid)..."
        kill -9 $port_pid 2>/dev/null || true
        sleep 1
    fi
}

# Initialize fresh PostgreSQL cluster
init_postgres() {
    log_info "Initializing PostgreSQL cluster..."

    # Clean up previous data
    kill_existing_postgres
    rm -rf "$PGDATA"
    mkdir -p "$PGDATA"

    # Ensure postgres user exists
    if ! id postgres &> /dev/null; then
        log_info "Creating postgres user..."
        useradd -r -s /bin/bash postgres 2>/dev/null || true
    fi

    chown postgres:postgres "$PGDATA"

    # Initialize cluster
    su - postgres -c "export PATH=$PG_BIN:\$PATH && initdb -D $PGDATA --auth=trust" > /dev/null 2>&1

    # Configure for local connections
    cat >> "$PGDATA/postgresql.conf" <<EOF
unix_socket_directories = '$PGDATA'
listen_addresses = 'localhost'
port = 5432
EOF

    cat >> "$PGDATA/pg_hba.conf" <<EOF
local   all             all                                     trust
host    all             all             127.0.0.1/32            trust
host    all             all             ::1/128                 trust
EOF

    check_pass "PostgreSQL cluster - initialized at $PGDATA"
}

# Start PostgreSQL
start_postgres() {
    log_info "Starting PostgreSQL..."

    touch "$PG_LOGFILE"
    chown postgres:postgres "$PG_LOGFILE"

    su - postgres -c "export PATH=$PG_BIN:\$PATH && pg_ctl -D $PGDATA -l $PG_LOGFILE start" > /dev/null 2>&1

    # Wait for startup
    for i in {1..15}; do
        if pg_isready -h localhost -p 5432 > /dev/null 2>&1; then
            check_pass "PostgreSQL - started on localhost:5432"
            return 0
        fi
        sleep 1
    done

    check_fail "PostgreSQL startup" "failed (see $PG_LOGFILE)"
    tail -20 "$PG_LOGFILE" 2>/dev/null || true
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
    log_info "Creating database..."

    su - postgres -c "export PATH=$PG_BIN:\$PATH && psql -h localhost -c \"CREATE USER everruns WITH PASSWORD 'everruns';\"" > /dev/null 2>&1 || true
    su - postgres -c "export PATH=$PG_BIN:\$PATH && psql -h localhost -c \"CREATE DATABASE everruns OWNER everruns;\"" > /dev/null 2>&1 || true
    su - postgres -c "export PATH=$PG_BIN:\$PATH && psql -h localhost -c \"GRANT ALL PRIVILEGES ON DATABASE everruns TO everruns;\"" > /dev/null 2>&1 || true

    check_pass "Database - everruns created"
}

# Full setup
setup_all() {
    check_root
    check_postgres_binaries
    init_postgres
    start_postgres
    setup_database
}

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    setup_all
fi
