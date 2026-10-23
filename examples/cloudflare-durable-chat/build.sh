#!/usr/bin/env bash
set -euo pipefail
APP_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ -f /workspace/.tools/activate.sh ]]; then source /workspace/.tools/activate.sh; fi
: "${CARGO_TARGET_DIR:=$APP_ROOT/target}"
export CARGO_TARGET_DIR
# Compatible Engine/adapter upgrades do not change the immutable Agent policy.
# Model selection is retained per session; host services preserve this tool ABI.
BUILD_DIGEST="$(node --input-type=module - "$APP_ROOT" <<'JS'
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
const root = process.argv[2];
const policy = createHash('sha256').update(readFileSync(`${root}/src/agent.rs`)).digest('hex');
const retained = JSON.parse(readFileSync(`${root}/retained-builds.json`, 'utf8'));
if (retained.build_ids.length && retained.policy_sha256 !== policy)
  throw new Error('Retained Agent policy changed: keep its original factory before changing behavior.');
process.stdout.write(policy.slice(0, 24));
JS
)"
export AGENT_BUILD_ID="chat-$BUILD_DIGEST"
cargo build --manifest-path "$APP_ROOT/Cargo.toml" --locked --release --target wasm32-unknown-unknown
wasm-bindgen "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/everruns_cloudflare_durable_chat.wasm" --target web --out-dir "$APP_ROOT/pkg"
printf '{"build_id":"%s","framework":1}\n' "$AGENT_BUILD_ID" > "$APP_ROOT/worker/build.json"
