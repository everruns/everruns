#!/usr/bin/env bash
# Actual workerd + native-process crash recovery. Uses only public test credentials.
set -euo pipefail
WORKER_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROJECT_ROOT="$(cd "$WORKER_ROOT/../../.." && pwd)"
cd "$PROJECT_ROOT"
if [[ -f /workspace/.tools/activate.sh ]]; then source /workspace/.tools/activate.sh; fi
if curl -s --max-time 1 http://127.0.0.1:8789/ >/dev/null 2>&1 || curl -s --max-time 1 http://127.0.0.1:8790/ >/dev/null 2>&1; then
  echo "Recovery test needs unused ports 8789 and 8790" >&2
  exit 1
fi
cargo test --locked -p everruns-durable-object --test recovery --no-run
RUN_DIR="$(mktemp -d /tmp/everruns-object-recovery.XXXXXX)"
AUTHORITY_PID=""
cleanup() {
  if [[ -n "$AUTHORITY_PID" ]]; then kill -- "-$AUTHORITY_PID" 2>/dev/null || true; wait "$AUTHORITY_PID" 2>/dev/null || true; kill -KILL -- "-$AUTHORITY_PID" 2>/dev/null || true; fi
}
trap cleanup EXIT INT TERM
cd "$WORKER_ROOT"
WRANGLER_SEND_METRICS=false setsid node node_modules/wrangler/bin/wrangler.js dev \
  --ip 127.0.0.1 --port 8789 --inspector-port 0 --persist-to "$RUN_DIR/state" \
  --var TENANT_ID:test \
  --var AUTHORITY_TOKEN:authority-test-token-at-least-32-bytes \
  --var EXECUTOR_TOKEN:executor-test-token-at-least-32-bytes \
  >"$RUN_DIR/authority.log" 2>&1 &
AUTHORITY_PID=$!
READY=false
for ((attempt = 0; attempt < 150; attempt++)); do
  if curl -s --max-time 1 http://127.0.0.1:8789/ >/dev/null 2>&1; then READY=true; break; fi
  if ! kill -0 "$AUTHORITY_PID" 2>/dev/null; then cat "$RUN_DIR/authority.log"; exit 1; fi
  sleep 0.2
done
if [[ "$READY" != true ]]; then cat "$RUN_DIR/authority.log"; exit 1; fi
cd "$PROJECT_ROOT"
echo "Authority evidence: $RUN_DIR/authority.log"
cargo test --locked -p everruns-durable-object --test recovery \
  a_killed_native_executor_recovers_from_do_without_resending_input -- --ignored --exact --nocapture

cargo test --locked -p everruns-durable-object --test conformance -- --ignored --nocapture
