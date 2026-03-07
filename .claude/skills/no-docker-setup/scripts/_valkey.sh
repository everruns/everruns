#!/bin/bash
# Valkey setup for no-docker environment
# Installs and starts Valkey for distributed rate limiting

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    source "$SCRIPT_DIR/_utils.sh"
    set -e
fi

export VALKEY_LOG="/tmp/valkey.log"
export VALKEY_PIDFILE="/tmp/valkey.pid"
export VALKEY_PORT=6379

# Install Valkey if not present
install_valkey() {
    if command -v valkey-server &> /dev/null; then
        check_pass "Valkey - found"
        return 0
    fi

    # Fall back to Redis-compatible server if available
    if command -v redis-server &> /dev/null; then
        check_pass "Redis-compatible server found (using as Valkey substitute)"
        return 0
    fi

    log_info "Installing Valkey..."
    # Try apt first (Ubuntu/Debian)
    if command -v apt-get &> /dev/null; then
        # Valkey may not be in default repos yet, try redis as fallback
        if apt-get install -y -qq valkey > /dev/null 2>&1; then
            check_pass "Valkey - installed via apt"
            return 0
        fi
        if apt-get install -y -qq redis-server > /dev/null 2>&1; then
            check_pass "Redis - installed as Valkey substitute"
            return 0
        fi
    fi

    log_warn "Could not install Valkey — distributed rate limiting unavailable"
    log_warn "Falling back to per-instance rate limiting"
    return 1
}

# Start Valkey server
start_valkey() {
    stop_valkey

    local server_cmd=""
    if command -v valkey-server &> /dev/null; then
        server_cmd="valkey-server"
    elif command -v redis-server &> /dev/null; then
        server_cmd="redis-server"
    else
        log_warn "No Valkey/Redis server found — skipping"
        return 1
    fi

    log_info "Starting Valkey on port $VALKEY_PORT..."
    $server_cmd --port $VALKEY_PORT --daemonize yes --pidfile "$VALKEY_PIDFILE" --logfile "$VALKEY_LOG" --save "" > /dev/null 2>&1

    # Wait for startup
    for i in {1..10}; do
        if valkey-cli -p $VALKEY_PORT ping 2>/dev/null | grep -q PONG; then
            check_pass "Valkey - started on localhost:$VALKEY_PORT"
            export VALKEY_URL="redis://localhost:$VALKEY_PORT"
            return 0
        fi
        if redis-cli -p $VALKEY_PORT ping 2>/dev/null | grep -q PONG; then
            check_pass "Valkey - started on localhost:$VALKEY_PORT"
            export VALKEY_URL="redis://localhost:$VALKEY_PORT"
            return 0
        fi
        sleep 1
    done

    log_warn "Valkey did not start (check $VALKEY_LOG)"
    return 1
}

# Stop Valkey
stop_valkey() {
    if [ -f "$VALKEY_PIDFILE" ]; then
        local pid
        pid=$(cat "$VALKEY_PIDFILE" 2>/dev/null || true)
        if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
            kill "$pid" 2>/dev/null || true
            sleep 1
        fi
        rm -f "$VALKEY_PIDFILE"
    fi
    # Kill any server on the port
    local port_pid
    port_pid=$(lsof -ti :$VALKEY_PORT 2>/dev/null || true)
    if [ -n "$port_pid" ]; then
        kill $port_pid 2>/dev/null || true
        sleep 1
    fi
}

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    check_root
    install_valkey && start_valkey
fi
