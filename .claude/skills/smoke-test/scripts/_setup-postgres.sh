#!/bin/bash
# PostgreSQL setup for smoke tests (no-Docker mode)
# Sets up a local PostgreSQL cluster without Docker
#
# This script can be:
# - Sourced by run-no-docker.sh (_utils.sh must be sourced first)
# - Executed directly (will source _utils.sh itself)
#
# Supports two modes:
# 1. Pre-installed PostgreSQL (detected via pg_ctlcluster) - starts existing cluster
# 2. Fresh install from PGDG repository - creates new cluster in /tmp

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    # Running directly - source _utils.sh
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    source "$SCRIPT_DIR/_utils.sh"
    set -e
fi

# Check if PostgreSQL is installed via system packages (pg_ctlcluster available)
is_system_postgres() {
    command -v pg_ctlcluster &> /dev/null
}

# Get installed PostgreSQL version from system
get_system_pg_version() {
    if [ -d "/etc/postgresql" ]; then
        ls /etc/postgresql 2>/dev/null | sort -V | tail -1
    fi
}

# Install PostgreSQL from PGDG repository
install_postgres() {
    log_info "Installing PostgreSQL $PG_VERSION from PGDG repository..."

    # Add PostgreSQL APT repository
    apt-get install -y curl ca-certificates gnupg > /dev/null 2>&1
    curl -fsSL https://www.postgresql.org/media/keys/ACCC4CF8.asc | gpg --dearmor -o /usr/share/keyrings/postgresql-keyring.gpg 2>/dev/null
    echo "deb [signed-by=/usr/share/keyrings/postgresql-keyring.gpg] http://apt.postgresql.org/pub/repos/apt $(lsb_release -cs)-pgdg main" > /etc/apt/sources.list.d/pgdg.list

    # Install PostgreSQL
    apt-get update > /dev/null 2>&1
    apt-get install -y postgresql-$PG_VERSION > /dev/null 2>&1

    # Stop the auto-started service (we'll run our own cluster)
    systemctl stop postgresql@$PG_VERSION-main 2>/dev/null || true
    systemctl disable postgresql@$PG_VERSION-main 2>/dev/null || true

    check_pass "PostgreSQL install - version $PG_VERSION installed"
}

# Check if PostgreSQL is installed, install if not
check_postgres() {
    # First check for system PostgreSQL (pg_ctlcluster available)
    if is_system_postgres; then
        local sys_version=$(get_system_pg_version)
        if [ -n "$sys_version" ]; then
            export PG_VERSION="$sys_version"
            export PG_BIN="/usr/lib/postgresql/$PG_VERSION/bin"
            export USE_SYSTEM_POSTGRES="true"
            check_pass "PostgreSQL install - found system install (version $PG_VERSION)"
            return 0
        fi
    fi

    # Check for PostgreSQL binaries in expected location
    if [ -f "$PG_BIN/initdb" ]; then
        check_pass "PostgreSQL install - found at $PG_BIN"
        return 0
    fi

    log_info "PostgreSQL $PG_VERSION not found, installing..."
    install_postgres

    if [ ! -f "$PG_BIN/initdb" ]; then
        check_fail "PostgreSQL install" "failed to install PostgreSQL $PG_VERSION"
        exit 1
    fi
}

# Start system PostgreSQL cluster using pg_ctlcluster
start_system_postgres() {
    log_info "Starting system PostgreSQL cluster..."

    # Start the cluster
    pg_ctlcluster "$PG_VERSION" main start 2>/dev/null || true

    # Wait for startup
    for i in {1..10}; do
        if pg_isready -h /var/run/postgresql > /dev/null 2>&1; then
            check_pass "PostgreSQL cluster - started (system install)"
            return 0
        fi
        sleep 1
    done

    check_fail "PostgreSQL cluster" "failed to start system cluster"
    exit 1
}

# Initialize PostgreSQL cluster (for non-system installs)
init_postgres() {
    # Skip if using system PostgreSQL
    if [ "$USE_SYSTEM_POSTGRES" = "true" ]; then
        log_info "Using system PostgreSQL, skipping init..."
        return 0
    fi

    log_info "Initializing PostgreSQL cluster..."

    # Clean up previous data
    rm -rf "$PGDATA"
    mkdir -p "$PGDATA"
    chown postgres:postgres "$PGDATA"

    # Initialize cluster
    su - postgres -c "export PATH=$PG_BIN:\$PATH && initdb -D $PGDATA --auth=trust" > /dev/null 2>&1

    # Configure socket directory
    su - postgres -c "echo \"unix_socket_directories = '$PGDATA'\" >> $PGDATA/postgresql.conf"

    check_pass "PostgreSQL cluster - initialized at $PGDATA"
}

# Start PostgreSQL
start_postgres() {
    # Use system method if available
    if [ "$USE_SYSTEM_POSTGRES" = "true" ]; then
        start_system_postgres
        return $?
    fi

    log_info "Starting PostgreSQL..."

    # Create log file with correct permissions
    touch "$PG_LOGFILE"
    chown postgres:postgres "$PG_LOGFILE"

    # Start server
    su - postgres -c "export PATH=$PG_BIN:\$PATH && pg_ctl -D $PGDATA -l $PG_LOGFILE start" > /dev/null 2>&1

    # Wait for startup
    for i in {1..10}; do
        if pg_isready -h "$PGDATA" > /dev/null 2>&1; then
            check_pass "PostgreSQL cluster - started and ready"
            return 0
        fi
        sleep 1
    done

    check_fail "PostgreSQL cluster" "failed to start (see $PG_LOGFILE)"
    cat "$PG_LOGFILE"
    exit 1
}

# Stop PostgreSQL
stop_postgres() {
    if [ "$USE_SYSTEM_POSTGRES" = "true" ]; then
        pg_ctlcluster "$PG_VERSION" main stop 2>/dev/null || true
        log_info "Stopped system PostgreSQL"
        return 0
    fi

    if [ -d "$PGDATA" ]; then
        su - postgres -c "export PATH=$PG_BIN:\$PATH && pg_ctl -D $PGDATA stop -m fast" 2>/dev/null || true
        log_info "Stopped PostgreSQL"
    fi
}

# Create database and user
setup_database() {
    log_info "Setting up database..."

    local psql_host
    if [ "$USE_SYSTEM_POSTGRES" = "true" ]; then
        psql_host="/var/run/postgresql"
    else
        psql_host="$PGDATA"
    fi

    # Create user and database (ignore errors if they already exist)
    su - postgres -c "export PATH=$PG_BIN:\$PATH && psql -h $psql_host -c \"CREATE USER everruns WITH PASSWORD 'everruns';\"" > /dev/null 2>&1 || true
    su - postgres -c "export PATH=$PG_BIN:\$PATH && psql -h $psql_host -c \"CREATE DATABASE everruns OWNER everruns;\"" > /dev/null 2>&1 || true
    su - postgres -c "export PATH=$PG_BIN:\$PATH && psql -h $psql_host -c \"GRANT ALL PRIVILEGES ON DATABASE everruns TO everruns;\"" > /dev/null 2>&1 || true

    check_pass "Database setup - database 'everruns' created"
}

# Get the database URL based on install type
get_database_url() {
    if [ "$USE_SYSTEM_POSTGRES" = "true" ]; then
        echo "postgres://everruns:everruns@localhost:5432/everruns"
    else
        echo "postgres://everruns:everruns@%2Ftmp%2Fpgdata/everruns"
    fi
}

# Full PostgreSQL setup
setup_all() {
    check_root
    check_postgres
    init_postgres
    start_postgres
    setup_database
}

# Run if executed directly
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    setup_all
fi
