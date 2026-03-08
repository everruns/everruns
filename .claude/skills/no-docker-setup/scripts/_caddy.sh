#!/bin/bash
# Caddy reverse proxy setup for no-docker environment
# Provides unified :9300 entry point matching docker-compose-full.yaml
#
# Routes /api/* -> API_PORT, /* -> UI_PORT
# SSE: h2c backend transport, no response buffering, no read timeout

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    source "$SCRIPT_DIR/_utils.sh"
    set -e
fi

export CADDY_LOG="/tmp/caddy.log"
export CADDY_PIDFILE="/tmp/caddy.pid"

# Install Caddy if not present
install_caddy() {
    if command -v caddy &> /dev/null; then
        check_pass "Caddy - found ($(caddy version 2>/dev/null | head -c 20))"
        return 0
    fi

    log_info "Installing Caddy..."
    apt-get update -qq > /dev/null 2>&1
    apt-get install -y -qq debian-keyring debian-archive-keyring apt-transport-https curl > /dev/null 2>&1
    curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' | gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg 2>/dev/null
    curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' | tee /etc/apt/sources.list.d/caddy-stable.list > /dev/null
    apt-get update -qq > /dev/null 2>&1
    apt-get install -y -qq caddy > /dev/null 2>&1

    if command -v caddy &> /dev/null; then
        check_pass "Caddy - installed"
    else
        log_error "Failed to install Caddy"
        exit 1
    fi
}

# Generate Caddyfile for no-docker setup
# Matches local/Caddyfile but with localhost backends
generate_caddyfile() {
    local caddyfile="/tmp/Caddyfile"
    cat > "$caddyfile" <<CADDYEOF
# No-docker reverse proxy (matches local/Caddyfile)
# API on :${API_PORT}, UI on :${UI_PORT}, unified on :${PROXY_PORT}
:${PROXY_PORT} {
	handle_path /api/* {
		reverse_proxy localhost:${API_PORT} {
			flush_interval -1
		}
	}
	handle /api-doc/* {
		reverse_proxy localhost:${API_PORT}
	}
	handle /health {
		reverse_proxy localhost:${API_PORT}
	}
	handle {
		reverse_proxy localhost:${UI_PORT}
	}
}
CADDYEOF
    check_pass "Caddyfile - generated at $caddyfile" >&2
    echo "$caddyfile"
}

# Start Caddy reverse proxy
start_caddy() {
    stop_caddy

    local caddyfile
    caddyfile=$(generate_caddyfile)

    log_info "Starting Caddy reverse proxy on :${PROXY_PORT}..."
    caddy start --config "$caddyfile" --pidfile "$CADDY_PIDFILE" > "$CADDY_LOG" 2>&1

    # Wait for startup
    for i in {1..10}; do
        if curl -s http://localhost:${PROXY_PORT}/health > /dev/null 2>&1; then
            check_pass "Caddy - started on localhost:${PROXY_PORT}"
            return 0
        fi
        sleep 1
    done

    # May not have a backend yet - check if Caddy is running
    if [ -f "$CADDY_PIDFILE" ] && kill -0 "$(cat "$CADDY_PIDFILE")" 2>/dev/null; then
        check_pass "Caddy - started on localhost:${PROXY_PORT} (backend not ready yet)"
        return 0
    fi

    log_warn "Caddy may not be ready (check $CADDY_LOG)"
}

# Stop Caddy
stop_caddy() {
    caddy stop > /dev/null 2>&1 || true
    if [ -f "$CADDY_PIDFILE" ]; then
        local pid
        pid=$(cat "$CADDY_PIDFILE" 2>/dev/null || true)
        if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
            kill "$pid" 2>/dev/null || true
        fi
        rm -f "$CADDY_PIDFILE"
    fi
    # Kill any caddy on proxy port
    local port_pid
    port_pid=$(lsof -ti :"${PROXY_PORT}" 2>/dev/null || true)
    if [ -n "$port_pid" ]; then
        kill -9 $port_pid 2>/dev/null || true
        sleep 1
    fi
}

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    install_caddy
    start_caddy
fi
