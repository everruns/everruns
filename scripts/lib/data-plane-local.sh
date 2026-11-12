#!/usr/bin/env bash
# Local data plane v2 (DATA_PLANE=v2 just start-all): one cell of three
# nodes, one executor and the edge router, next to the usual stack.
#
# Decisions:
# - Ports come from the stack's PORT_PREFIX: nodes ${PORT_PREFIX}41..43,
#   the executor ${PORT_PREFIX}44, the edge ${PORT_PREFIX}45, so stacks in
#   other worktrees never collide.
# - The link's token and the data plane's wrapping key are made once per
#   prefix and kept (mode 0600) under .local/data-plane/, like the local
#   secrets key: a restart keeps the orgs already moved working.
# - The proxy sends /api/v1/sessions and below to the edge (SESSIONS_UPSTREAM);
#   the edge passes everything that is not a data plane session's call on
#   to the API server unchanged.
# - The cell binary is `cell-testnode`: the standard host plus the
#   simulated `llmsim` provider, so an agent can chat without any key, and
#   an org's own providers (a real model) through its sealed connections.
#
# Usage (sourced by services.sh): data_plane_local_env, then
# data_plane_local_start once the API answers; data_plane_local_stop.

DP_LOCAL_ID="dp-local"

data_plane_local_dir() {
  echo "$PROJECT_ROOT/.local/data-plane/${PORT_PREFIX:-default}"
}

# Export what the API server and the cells share. Call before the API starts.
data_plane_local_env() {
  local dir
  dir="$(data_plane_local_dir)"
  mkdir -p "$dir"
  chmod 700 "$dir"
  if [ ! -s "$dir/link-token" ]; then
    (umask 077 && head -c 32 /dev/urandom | base64 | tr -d '=+/\n' > "$dir/link-token")
  fi
  if [ ! -s "$dir/dp-key" ]; then
    (umask 077 && head -c 32 /dev/urandom | base64 | tr -d '\n' > "$dir/dp-key")
  fi
  EVERRUNS_DP_TOKEN="$(cat "$dir/link-token")"
  EVERRUNS_DP_KEY="$(cat "$dir/dp-key")"
  export EVERRUNS_DP_TOKEN EVERRUNS_DP_KEY
  export CELL_DP_TOKEN="$EVERRUNS_DP_TOKEN"
  export CELL_DP_KEY="$EVERRUNS_DP_KEY"
  export CELL_DATA_PLANE="$DP_LOCAL_ID"
  export CELL_CONTROL_PLANE="http://127.0.0.1:${API_PORT}/api"
  export CELL_JWKS_URL="http://127.0.0.1:${API_PORT}/api/.well-known/jwks.json"
  export DP_EDGE_PORT="${PORT_PREFIX}45"
  export SESSIONS_UPSTREAM="127.0.0.1:${DP_EDGE_PORT}"
}

data_plane_local_topology() {
  local dir="$1"
  cat > "$dir/topology.json" <<EOF
{
  "cells": {
    "1": {
      "1": "127.0.0.1:${PORT_PREFIX}41",
      "2": "127.0.0.1:${PORT_PREFIX}42",
      "3": "127.0.0.1:${PORT_PREFIX}43"
    }
  }
}
EOF
}

# Build and start the fleet, register it with the control plane.
data_plane_local_start() {
  local dir bin store
  dir="$(data_plane_local_dir)"
  mkdir -p "$dir/logs" "$dir/data" "$dir/store"
  data_plane_local_topology "$dir"
  echo "   Building the cell binary..."
  cargo build -p everruns-cell-testkit --bin cell-testnode || return 1
  bin="${CARGO_TARGET_DIR:-$PROJECT_ROOT/target}/debug/cell-testnode"
  store="file://$dir/store"
  local node
  for node in 1 2 3; do
    CELL_GATEWAY="127.0.0.1:${PORT_PREFIX}4${node}" CELL_EXECUTOR_CAP=256 \
      "$bin" node --topology "$dir/topology.json" --cell 1 --node "$node" \
      --data "$dir/data/n$node" --store "$store" \
      --control-plane "$CELL_CONTROL_PLANE" \
      >> "$dir/logs/n$node.log" 2>&1 &
    record_pid "cell-n$node" "$!"
    CHILD_PIDS+=("$!")
  done
  "$bin" executor --topology "$dir/topology.json" --id $((1 << 32)) \
    --store "$store" --listen "127.0.0.1:${PORT_PREFIX}44" \
    >> "$dir/logs/executor.log" 2>&1 &
  record_pid cell-executor "$!"
  CHILD_PIDS+=("$!")
  "$bin" edge --topology "$dir/topology.json" --listen "127.0.0.1:${DP_EDGE_PORT}" \
    --control-plane "http://127.0.0.1:${API_PORT}" --api-prefix /api \
    >> "$dir/logs/edge.log" 2>&1 &
  record_pid cell-edge "$!"
  CHILD_PIDS+=("$!")

  local i
  for i in $(seq 1 60); do
    if curl -fsS "http://127.0.0.1:${PORT_PREFIX}41/health" > /dev/null 2>&1 \
      && curl -fsS "http://127.0.0.1:${DP_EDGE_PORT}/health" > /dev/null 2>&1; then
      break
    fi
    sleep 1
  done
  # The control plane's note of the data plane: its gateway (any node) for
  # config and sessions, the same node for org admin.
  cargo run -q -p everruns-server --bin everruns-dataplane -- \
    register "$DP_LOCAL_ID" "http://127.0.0.1:${PORT_PREFIX}41" || return 1
  echo "   ✅ Data plane $DP_LOCAL_ID: cell 1 (3 nodes), 1 executor, edge :${DP_EDGE_PORT}"
  echo "      Move an org onto it: just dp-enable <org public id>"
}

# Put an org on the local data plane (the stack must be running).
data_plane_local_enable() {
  local org="$1"
  [ -n "$org" ] || { echo "usage: just dp-enable <org public id>" >&2; return 2; }
  data_plane_local_env
  export DATABASE_URL="${DATABASE_URL:-$DB_URL_DEFAULT}"
  configure_local_development_encryption_key || return 1
  cargo run -q -p everruns-server --bin everruns-dataplane -- enable "$org" "$DP_LOCAL_ID"
}
