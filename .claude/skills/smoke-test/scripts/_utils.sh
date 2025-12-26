#!/bin/bash
# Common utilities for smoke tests (no-Docker mode)

# Configuration
export PGDATA="/tmp/pgdata"
export PG_LOGFILE="$PGDATA/pg.log"
export PG_VERSION="17"
export PG_BIN="/usr/lib/postgresql/$PG_VERSION/bin"
export TEMPORAL_BIN="/usr/local/bin/temporal"
export TEMPORAL_LOG="/tmp/temporal.log"
export API_LOG="/tmp/api.log"
export WORKER_LOG="/tmp/worker.log"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Checkbox output format
check_pass() {
    echo -e "${GREEN}[x]${NC} $1"
}

check_fail() {
    echo -e "${RED}[!]${NC} $1 - FAILED: $2"
}

check_pending() {
    echo -e "[ ] $1"
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

# Check if running as root
check_root() {
    if [ "$(id -u)" -ne 0 ]; then
        log_error "This script must be run as root to initialize PostgreSQL"
        exit 1
    fi
}

# Check for required environment variables
check_openai_key() {
    # Check for OPENAI_API_KEY or ANTHROPIC_API_KEY
    if [ -n "$OPENAI_API_KEY" ]; then
        log_info "OPENAI_API_KEY is set"
        return 0
    fi
    if [ -n "$ANTHROPIC_API_KEY" ]; then
        log_info "ANTHROPIC_API_KEY is set (will use Claude models)"
        return 0
    fi
    log_error "Neither OPENAI_API_KEY nor ANTHROPIC_API_KEY environment variable is set"
    log_error "Export one before running: export OPENAI_API_KEY=your-key"
    exit 1
}

# Check/generate SECRETS_ENCRYPTION_KEY
check_encryption_key() {
    if [ -z "$SECRETS_ENCRYPTION_KEY" ]; then
        # Generate a test encryption key for smoke testing
        local key_bytes
        key_bytes=$(python3 -c "import os, base64; print(base64.b64encode(os.urandom(32)).decode())" 2>/dev/null || \
                   openssl rand -base64 32 2>/dev/null || \
                   head -c 32 /dev/urandom | base64)
        export SECRETS_ENCRYPTION_KEY="kek-test:$key_bytes"
        log_info "Generated test SECRETS_ENCRYPTION_KEY"
    else
        log_info "SECRETS_ENCRYPTION_KEY is set"
    fi
}

# Check and install protoc (required for building Temporal SDK)
check_protoc() {
    if command -v protoc &> /dev/null; then
        check_pass "protoc install - $(protoc --version)"
        return 0
    fi

    log_info "protoc not found, installing..."

    # Try apt-get (Debian/Ubuntu)
    if command -v apt-get &> /dev/null; then
        apt-get update -qq > /dev/null 2>&1
        apt-get install -y -qq protobuf-compiler > /dev/null 2>&1
        if command -v protoc &> /dev/null; then
            check_pass "protoc install - $(protoc --version)"
            return 0
        fi
    fi

    # Try downloading from GitHub releases
    log_info "Installing protoc from GitHub releases..."
    local protoc_version="25.1"
    local protoc_url="https://github.com/protocolbuffers/protobuf/releases/download/v${protoc_version}/protoc-${protoc_version}-linux-x86_64.zip"

    curl -L --insecure "$protoc_url" -o /tmp/protoc.zip > /dev/null 2>&1
    unzip -q /tmp/protoc.zip -d /tmp/protoc_extract > /dev/null 2>&1
    mv /tmp/protoc_extract/bin/protoc /usr/local/bin/protoc
    chmod +x /usr/local/bin/protoc
    rm -rf /tmp/protoc.zip /tmp/protoc_extract

    if command -v protoc &> /dev/null; then
        check_pass "protoc install - $(protoc --version)"
        return 0
    fi

    check_fail "protoc install" "could not install protoc"
    exit 1
}

# Check and install jq (required for tests)
check_jq() {
    if command -v jq &> /dev/null; then
        return 0
    fi

    log_info "jq not found, installing..."
    if command -v apt-get &> /dev/null; then
        apt-get update -qq > /dev/null 2>&1
        apt-get install -y -qq jq > /dev/null 2>&1
    fi

    if command -v jq &> /dev/null; then
        check_pass "jq install - installed"
        return 0
    fi

    check_fail "jq install" "could not install jq"
    exit 1
}

# Get project root (relative to skill scripts folder)
get_project_root() {
    local script_dir="$(cd "$(dirname "${BASH_SOURCE[1]}")" && pwd)"
    echo "$(cd "$script_dir/../../../.." && pwd)"
}
