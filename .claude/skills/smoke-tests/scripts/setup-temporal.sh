#!/bin/bash
# Temporal setup for smoke tests (no-Docker mode)
# Installs and starts Temporal dev server without Docker
#
# This script can be:
# - Sourced by run-no-docker.sh (common.sh must be sourced first)
# - Executed directly (will source common.sh itself)

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    # Running directly - source common.sh
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    source "$SCRIPT_DIR/common.sh"
    set -e
fi

# Install Temporal CLI if not present
install_temporal() {
    if [ -f "$TEMPORAL_BIN" ]; then
        check_pass "Temporal install - $($TEMPORAL_BIN --version)"
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

    check_pass "Temporal install - $($TEMPORAL_BIN --version)"
}

# Start Temporal dev server
# Returns the PID of the started process
start_temporal() {
    log_info "Starting Temporal dev server..."

    # Start Temporal dev server in background (uses in-memory SQLite)
    "$TEMPORAL_BIN" server start-dev --headless > "$TEMPORAL_LOG" 2>&1 &
    local pid=$!
    echo $pid

    # Wait for Temporal to be ready
    for i in {1..30}; do
        if nc -z localhost 7233 2>/dev/null; then
            check_pass "Temporal server - started on localhost:7233"
            return 0
        fi
        sleep 1
    done

    check_fail "Temporal server" "failed to start (see $TEMPORAL_LOG)"
    cat "$TEMPORAL_LOG"
    exit 1
}

# Stop Temporal server by PID
stop_temporal() {
    local pid=$1
    if [ -n "$pid" ]; then
        kill "$pid" 2>/dev/null || true
        log_info "Stopped Temporal server"
    fi
}

# Full Temporal setup (install + start)
setup_all() {
    install_temporal
    start_temporal
}

# Run if executed directly
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    setup_all
fi
