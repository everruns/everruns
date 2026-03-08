#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# shellcheck disable=SC1091
source "$PROJECT_ROOT/scripts/lib/common.sh"

assert_dead() {
  local pid="$1"
  local label="$2"

  if kill -0 "$pid" 2>/dev/null; then
    echo "FAIL: $label still running (pid $pid)" >&2
    exit 1
  fi
}

wait_for_listener() {
  local port="$1"

  for _ in {1..20}; do
    if lsof -nP -t -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.25
  done

  echo "FAIL: listener never started on port $port" >&2
  exit 1
}

export PORT_PREFIX=278
unset API_PORT WORKER_GRPC_PORT CADDY_ADMIN_PORT UI_PORT PROXY_PORT OTEL_GRPC_PORT OTEL_HTTP_PORT VALKEY_PORT JAEGER_UI_PORT DB_PORT COMPOSE_PROJECT_NAME
apply_port_prefix_defaults

rm -rf "$RUN_STATE_DIR"

(exec -a everruns-server nc -lk 127.0.0.1 "$API_PORT" >/dev/null 2>&1) &
listener_pid=$!

cleanup() {
  kill "$listener_pid" >/dev/null 2>&1 || true
  wait "$listener_pid" 2>/dev/null || true
  rm -rf "$RUN_STATE_DIR"
}

trap cleanup EXIT

wait_for_listener "$API_PORT"

"$PROJECT_ROOT/scripts/lib/services.sh" stop-all >/dev/null

sleep 1
assert_dead "$listener_pid" "managed listener without pid state"

echo "stop-all fallback cleanup checks passed"
