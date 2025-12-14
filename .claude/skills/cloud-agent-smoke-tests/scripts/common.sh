#!/bin/bash
# Common utilities for Cloud Agent smoke tests

# Configuration
export PGDATA="/tmp/pgdata"
export PG_LOGFILE="$PGDATA/pg.log"
export PG_VERSION="17"
export PG_BIN="/usr/lib/postgresql/$PG_VERSION/bin"
export TEMPORAL_BIN="/usr/local/bin/temporal"
export TEMPORAL_LOG="/tmp/temporal.log"
export API_LOG="/tmp/api.log"

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

# Check if running as root
check_root() {
    if [ "$(id -u)" -ne 0 ]; then
        log_error "This script must be run as root to initialize PostgreSQL"
        exit 1
    fi
}

# Check for required environment variables
check_openai_key() {
    if [ -z "$OPENAI_API_KEY" ]; then
        log_error "OPENAI_API_KEY environment variable is not set"
        log_error "Export it before running: export OPENAI_API_KEY=your-key"
        exit 1
    fi
    log_info "OPENAI_API_KEY is set"
}

# Get project root (3 levels up from scripts folder)
get_project_root() {
    local script_dir="$(cd "$(dirname "${BASH_SOURCE[1]}")" && pwd)"
    echo "$(cd "$script_dir/../../.." && pwd)"
}
