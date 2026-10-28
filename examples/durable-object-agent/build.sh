#!/usr/bin/env bash
set -euo pipefail
APP_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$APP_ROOT/../.." && pwd)"
if [[ -f /workspace/.tools/activate.sh ]]; then source /workspace/.tools/activate.sh; fi
: "${CARGO_TARGET_DIR:=$PROJECT_ROOT/target/wasm-agent}"
export CARGO_TARGET_DIR
# A compatible Engine/host deployment must not mint a new Agent identity.
# Hash the immutable application definition and model, not framework/adapter code.
# Retain previous definitions under their original keys when behavior changes.
export AGENT_MODEL="${AGENT_MODEL:-gpt-6-luna}"
BUILD_DIGEST="$( { cat "$PROJECT_ROOT/crates/durable-object/examples/shared/agent.rs"; printf '\0%s' "$AGENT_MODEL"; } | sha256sum | cut -c1-24)"
export AGENT_BUILD_ID="counter-$BUILD_DIGEST"
cargo build --manifest-path "$APP_ROOT/Cargo.toml" --locked --release --target wasm32-unknown-unknown
# Debugging custom sections do not affect exports or the stable Agent identity.
wasm-bindgen "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/everruns_object_agent.wasm" --target web --remove-name-section --remove-producers-section --out-dir "$APP_ROOT/pkg"
printf '{"agent":"counter","build_id":"%s","framework":1}\n' "$AGENT_BUILD_ID" > "$APP_ROOT/worker/build.json"
