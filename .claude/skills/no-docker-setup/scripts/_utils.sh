#!/bin/bash
# Utilities for no-docker smoke tests
# Deterministic configuration - no auto-detection

# Fixed configuration - deterministic paths
export PGDATA="/tmp/pgdata"
export PG_LOGFILE="$PGDATA/pg.log"
export API_LOG="/tmp/api.log"
export WORKER_LOG="/tmp/worker.log"
export DATABASE_URL="postgres://everruns:everruns@localhost:5432/everruns"

# Detect PostgreSQL version (use highest available)
detect_pg_version() {
    if [ -d "/usr/lib/postgresql" ]; then
        local version=$(ls /usr/lib/postgresql 2>/dev/null | sort -V | tail -1)
        if [ -n "$version" ]; then
            echo "$version"
            return 0
        fi
    fi
    echo "17"  # fallback
}

export PG_VERSION="${PG_VERSION:-$(detect_pg_version)}"
export PG_BIN="/usr/lib/postgresql/$PG_VERSION/bin"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

check_pass() {
    echo -e "${GREEN}[x]${NC} $1"
}

check_fail() {
    echo -e "${RED}[!]${NC} $1 - FAILED: $2"
}

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Check root access
check_root() {
    if [ "$(id -u)" -ne 0 ]; then
        log_error "This script must be run as root"
        exit 1
    fi
}

# Check for LLM API key
check_api_key() {
    if [ -n "$OPENAI_API_KEY" ]; then
        log_info "OPENAI_API_KEY is set"
        return 0
    fi
    if [ -n "$ANTHROPIC_API_KEY" ]; then
        log_info "ANTHROPIC_API_KEY is set"
        return 0
    fi
    log_error "Neither OPENAI_API_KEY nor ANTHROPIC_API_KEY is set"
    log_error "Export one before running: export OPENAI_API_KEY=your-key"
    exit 1
}

# Set encryption key (deterministic for smoke tests)
set_encryption_key() {
    if [ -z "$SECRETS_ENCRYPTION_KEY" ]; then
        export SECRETS_ENCRYPTION_KEY="kek-v1:8B3uCQ4Znx45hl5nB+PKVriRrj/KtEVM+wBZ2VGa9vY="
        log_info "Using standard SECRETS_ENCRYPTION_KEY"
    else
        log_info "SECRETS_ENCRYPTION_KEY is set"
    fi
}

# Check jq is installed
check_jq() {
    if command -v jq &> /dev/null; then
        check_pass "jq - found"
        return 0
    fi
    log_error "jq is not installed. Install with: apt-get install jq"
    exit 1
}

# Check PostgreSQL binaries exist
check_postgres_binaries() {
    if [ -f "$PG_BIN/initdb" ] && [ -f "$PG_BIN/pg_ctl" ]; then
        check_pass "PostgreSQL binaries - found at $PG_BIN"
        return 0
    fi
    log_error "PostgreSQL $PG_VERSION binaries not found at $PG_BIN"
    log_error "Install PostgreSQL: apt-get install postgresql-$PG_VERSION"
    exit 1
}

# Ensure sqlx CLI is installed (fast install with minimal features)
ensure_sqlx() {
    if command -v sqlx &> /dev/null; then
        check_pass "sqlx - found"
        return 0
    fi
    log_info "Installing sqlx CLI..."
    cargo install sqlx-cli --no-default-features --features postgres 2>&1 | tail -5
    check_pass "sqlx - installed"
}

# Get project root
get_project_root() {
    local script_dir="$(cd "$(dirname "${BASH_SOURCE[1]}")" && pwd)"
    echo "$(cd "$script_dir/../../../.." && pwd)"
}
