#!/bin/bash
# Reproduction script: SSE connection cycling through Caddy reverse proxy
#
# Demonstrates SSE connection cycling behavior through Caddy with HTTP/1.1
# vs h2c backend transport.
#
# FIXED in everruns-sdk @ d96e85cf (main, unreleased):
# - Graceful disconnects no longer consume retry budget
# - `connected` event resets backoff and retry_count
# - HTTP client reused across reconnections (connection pooling)
#
# The original issue (everruns-sdk v0.1.2): graceful disconnect events from
# normal 5-min connection cycling were counted against max_retries. With
# max_retries(5), sessions running >25 min would exhaust retries.
#
# Prerequisites:
#   - Running server on :9301 (via `just start-dev` or `just start-all`)
#   - Caddy installed (`just init` or `apt install caddy`)
#   - curl, jq
#
# Usage:
#   # Test with HTTP/1.1 backend (shows the bug)
#   ./scripts/repro-sse-caddy.sh http1
#
#   # Test with h2c backend (shows the fix)
#   ./scripts/repro-sse-caddy.sh h2c
#
#   # Both (comparison)
#   ./scripts/repro-sse-caddy.sh

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

if [ -n "${PORT_PREFIX:-}" ]; then
    API_PORT="${API_PORT:-${PORT_PREFIX}01}"
    PROXY_PORT="${PROXY_PORT:-${PORT_PREFIX}00}"
    CADDY_ADMIN_PORT="${CADDY_ADMIN_PORT:-${PORT_PREFIX}03}"
else
    API_PORT="${API_PORT:-9301}"
    PROXY_PORT="${PROXY_PORT:-9300}"
    CADDY_ADMIN_PORT="${CADDY_ADMIN_PORT:-2019}"
fi
SSE_CYCLE_SECS="${SSE_CYCLE_SECS:-15}"  # Short cycle for faster repro

log() { echo -e "${CYAN}[repro]${NC} $1"; }
log_ok() { echo -e "${GREEN}[  ok]${NC} $1"; }
log_err() { echo -e "${RED}[ err]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[warn]${NC} $1"; }

check_deps() {
    for cmd in curl jq caddy; do
        if ! command -v "$cmd" &> /dev/null; then
            log_err "Missing: $cmd"
            exit 1
        fi
    done

    if ! curl -s "http://localhost:$API_PORT/health" > /dev/null 2>&1; then
        log_err "API server not running on :$API_PORT"
        log_err "Start with: just start-dev --no-watch"
        exit 1
    fi
    log_ok "API server running on :$API_PORT"
}

# Start Caddy with specific transport config
start_caddy_proxy() {
    local mode="$1"  # "http1" or "h2c"
    local caddyfile="/tmp/repro-sse-${mode}.Caddyfile"

    caddy stop > /dev/null 2>&1 || true
    sleep 1

    if [ "$mode" = "h2c" ]; then
        cat > "$caddyfile" <<EOF
admin :${CADDY_ADMIN_PORT}

:${PROXY_PORT} {
    handle /api/* {
        reverse_proxy localhost:${API_PORT} {
            flush_interval -1
            transport http {
                versions h2c 2
                read_timeout 0
            }
        }
    }
    handle /health {
        reverse_proxy localhost:${API_PORT}
    }
}
EOF
    else
        # HTTP/1.1 default (the buggy config)
        cat > "$caddyfile" <<EOF
admin :${CADDY_ADMIN_PORT}

:${PROXY_PORT} {
    handle /api/* {
        reverse_proxy localhost:${API_PORT} {
            flush_interval -1
        }
    }
    handle /health {
        reverse_proxy localhost:${API_PORT}
    }
}
EOF
    fi

    caddy start --config "$caddyfile" > /dev/null 2>&1
    sleep 1

    if curl -s "http://localhost:$PROXY_PORT/health" > /dev/null 2>&1; then
        log_ok "Caddy proxy started (${mode}) on :$PROXY_PORT"
    else
        log_err "Caddy failed to start"
        exit 1
    fi
}

# Open an SSE stream and count events over duration
test_sse_stream() {
    local mode="$1"
    local duration="$2"  # seconds to run
    local url="http://localhost:${PROXY_PORT}/api/v1/sessions/${SESSION_ID}/sse?exclude=output.message.delta&exclude=reason.thinking.delta"

    log "Opening SSE stream through Caddy (${mode}) for ${duration}s..."
    log "  URL: $url"
    log "  SSE cycle interval: ${SSE_CYCLE_SECS}s (server-side)"

    local output_file="/tmp/repro-sse-${mode}.log"
    local event_count=0
    local disconnect_count=0
    local connected_count=0
    local error_count=0

    # Stream SSE events with timeout
    timeout "${duration}" curl -sN \
        -H "Accept: text/event-stream" \
        -H "Cache-Control: no-cache" \
        "$url" > "$output_file" 2>&1 || true

    # Count events
    event_count=$(grep -c "^event:" "$output_file" 2>/dev/null || echo 0)
    disconnect_count=$(grep -c "event: disconnecting" "$output_file" 2>/dev/null || echo 0)
    connected_count=$(grep -c "event: connected" "$output_file" 2>/dev/null || echo 0)

    echo ""
    echo "  ┌─── Results (${mode}) ───"
    echo "  │ Duration:       ${duration}s"
    echo "  │ Total events:   $event_count"
    echo "  │ Connected:      $connected_count"
    echo "  │ Disconnecting:  $disconnect_count"
    echo "  │ Expected cycles: ~$((duration / SSE_CYCLE_SECS))"
    echo "  └───────────────────"
    echo ""

    if [ "$disconnect_count" -gt 0 ]; then
        log_ok "Connection cycling observed ($disconnect_count disconnecting events)"
        log_warn "With SDK max_retries(5): would exhaust after $((5 * SSE_CYCLE_SECS))s"
    fi

    # Show raw log excerpt
    if [ -s "$output_file" ]; then
        log "Raw SSE log (last 20 lines): $output_file"
        tail -20 "$output_file" | sed 's/^/  │ /'
    fi
}

# Create a test session
create_session() {
    local api_url="http://localhost:${API_PORT}"

    # Create agent
    local agent_resp
    agent_resp=$(curl -s -X POST "$api_url/v1/agents" \
        -H "Content-Type: application/json" \
        -d '{"name":"SSE Repro Agent","system_prompt":"You are a test agent."}')

    AGENT_ID=$(echo "$agent_resp" | jq -r '.id // empty')
    if [ -z "$AGENT_ID" ]; then
        log_err "Failed to create agent: $agent_resp"
        exit 1
    fi
    log_ok "Agent: $AGENT_ID"

    # Create session
    local session_resp
    session_resp=$(curl -s -X POST "$api_url/v1/sessions" \
        -H "Content-Type: application/json" \
        -d "{\"agent_id\":\"$AGENT_ID\"}")

    SESSION_ID=$(echo "$session_resp" | jq -r '.id // empty')
    if [ -z "$SESSION_ID" ]; then
        log_err "Failed to create session: $session_resp"
        exit 1
    fi
    log_ok "Session: $SESSION_ID"
}

# Run test with a specific mode
run_test() {
    local mode="$1"
    local duration=$((SSE_CYCLE_SECS * 3 + 5))  # 3 cycles + buffer

    echo ""
    echo "═══════════════════════════════════════════════"
    echo "  Testing SSE through Caddy (${mode})"
    echo "═══════════════════════════════════════════════"
    echo ""

    start_caddy_proxy "$mode"
    create_session
    test_sse_stream "$mode" "$duration"
}

cleanup() {
    caddy stop > /dev/null 2>&1 || true
}
trap cleanup EXIT

# Main
check_deps

# Export short cycle for faster reproduction
export SSE_REALTIME_CYCLE_SECS="$SSE_CYCLE_SECS"
log "Using SSE_REALTIME_CYCLE_SECS=$SSE_CYCLE_SECS for fast reproduction"
log_warn "Server must be restarted with this env var for it to take effect"
echo ""

MODE="${1:-both}"
case "$MODE" in
    http1)
        run_test "http1"
        ;;
    h2c)
        run_test "h2c"
        ;;
    both|*)
        run_test "http1"
        run_test "h2c"
        echo ""
        echo "═══════════════════════════════════════════════"
        echo "  Summary"
        echo "═══════════════════════════════════════════════"
        echo ""
        echo "  Both modes should show connection cycling (disconnecting events)."
        echo "  The difference is in HOW the proxy handles reconnection:"
        echo ""
        echo "  HTTP/1.1: Each SSE reconnect = new TCP connection through Caddy."
        echo "    Under load, concurrent reconnections can fail, triggering exponential"
        echo "    backoff. With SDK v0.1.2, graceful disconnects were counted against"
        echo "    max_retries, causing streams to die after 5 cycles (~25 min)."
        echo ""
        echo "  h2c: SSE reconnects multiplex over existing TCP connection."
        echo "    Reconnections are lightweight and less likely to fail under load."
        echo ""
        echo "  SDK fixes (everruns-sdk @ d96e85cf, on main):"
        echo "    1. Graceful disconnects no longer count against max_retries  [FIXED]"
        echo "    2. 'connected' event resets backoff and retry_count          [FIXED]"
        echo "    3. reqwest::Client reused across reconnections               [FIXED]"
        echo ""
        echo "  With the SDK fix, both HTTP/1.1 and h2c work correctly."
        echo "  h2c remains beneficial for efficiency under load (fewer TCP connections)."
        echo ""
        ;;
esac
