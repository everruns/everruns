#!/usr/bin/env bash
# Deploy an isolated qualification Worker, prove recovery, then delete its class and Worker.
set -euo pipefail
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
cd "$PROJECT_ROOT"
if [[ -f /workspace/.tools/activate.sh ]]; then source /workspace/.tools/activate.sh; fi
: "${CLOUDFLARE_ACCOUNT_ID:?Cloudflare account is required}"
: "${CLOUDFLARE_API_TOKEN:?Cloudflare API token is required}"
test -f examples/durable-object-agent/pkg/everruns_object_agent_bg.wasm
WORKER_NAME="er-object-qual-$(openssl rand -hex 5)"
RUN_DIR="$(mktemp -d /tmp/everruns-cloudflare-qualification.XXXXXX)"
TEST_DIR="$PROJECT_ROOT/crates/durable-object/worker/test"
WRANGLER="$PROJECT_ROOT/crates/durable-object/worker/node_modules/wrangler/bin/wrangler.js"
CLEAN_CONFIG="$TEST_DIR/$WORKER_NAME.generated.json"
CLEAN_SCRIPT="$TEST_DIR/$WORKER_NAME.generated.js"
CREATED=false
cleanup() {
  local original_status=$?
  trap - EXIT
  if [[ "$CREATED" == true ]]; then
    printf 'export default { fetch() { return new Response("qualification finished", {status:404}); } };\n' > "$CLEAN_SCRIPT"
    node --input-type=module - "$CLEAN_CONFIG" "$WORKER_NAME" "$(basename "$CLEAN_SCRIPT")" <<'JS'
import {writeFileSync} from 'node:fs';
const [path,name,main]=process.argv.slice(2);
writeFileSync(path,JSON.stringify({name,main,compatibility_date:'2026-07-01',migrations:[{tag:'v1',new_sqlite_classes:['SessionAuthority']},{tag:'v2',deleted_classes:['SessionAuthority']}]}));
JS
    if ! WRANGLER_SEND_METRICS=false node "$WRANGLER" deploy --config "$CLEAN_CONFIG" >"$RUN_DIR/cleanup.log" 2>&1; then
      echo "Could not delete qualification namespace: $WORKER_NAME (logs: $RUN_DIR/cleanup.log)" >&2
      original_status=1
    fi
    if ! WRANGLER_SEND_METRICS=false node "$WRANGLER" delete --name "$WORKER_NAME" --config "$CLEAN_CONFIG" --force >>"$RUN_DIR/cleanup.log" 2>&1; then
      echo "Could not delete qualification Worker: $WORKER_NAME (logs: $RUN_DIR/cleanup.log)" >&2
      original_status=1
    fi
  fi
  rm -f "$CLEAN_CONFIG" "$CLEAN_SCRIPT" "$RUN_DIR/secrets.json"
  exit "$original_status"
}
trap cleanup EXIT
umask 077
DO_QUALIFY_TOKEN="$(openssl rand -hex 32)"
export DO_QUALIFY_TOKEN
node --input-type=module - "$RUN_DIR/secrets.json" <<'JS'
import {writeFileSync} from 'node:fs';writeFileSync(process.argv[2],JSON.stringify({AUTHORITY_TOKEN:process.env.DO_QUALIFY_TOKEN}),{mode:0o600});
JS
CREATED=true
WRANGLER_SEND_METRICS=false node "$WRANGLER" deploy --config "$TEST_DIR/qualification.json" --name "$WORKER_NAME" --minify >"$RUN_DIR/deploy.log" 2>&1
WRANGLER_SEND_METRICS=false node "$WRANGLER" secret bulk "$RUN_DIR/secrets.json" --config "$TEST_DIR/qualification.json" --name "$WORKER_NAME" >"$RUN_DIR/secrets.log" 2>&1
rm -f "$RUN_DIR/secrets.json"
DO_QUALIFY_URL="$(node --input-type=module - "$RUN_DIR/deploy.log" <<'JS'
import {readFileSync} from 'node:fs';const match=readFileSync(process.argv[2],'utf8').match(/https:\/\/[a-zA-Z0-9.-]+\.workers\.dev/);if(!match)process.exit(1);console.log(match[0]);
JS
)"
export DO_QUALIFY_URL DO_QUALIFY_BACKEND=cloudflare
NODE_USE_ENV_PROXY=1 node "$TEST_DIR/qualify-isolate.mjs" | tee "$RUN_DIR/proof.jsonl"
echo "Cloudflare qualification evidence: $RUN_DIR/proof.jsonl"
