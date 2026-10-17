#!/usr/bin/env bash
set -euo pipefail
APP_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$APP_ROOT/../.." && pwd)"
if [[ -f /workspace/.tools/activate.sh ]]; then source /workspace/.tools/activate.sh; fi
: "${CARGO_TARGET_DIR:=$PROJECT_ROOT/target/wasm-agent}"
export CARGO_TARGET_DIR
# Pin the compiled agent to its complete source/lock inputs, including local edits.
BUILD_DIGEST="$(cd "$PROJECT_ROOT"
{
  find crates/everruns/src crates/core/src crates/contracts/src crates/durable-object/src examples/durable-object-agent/src crates/durable-object/worker/src examples/durable-object-agent/worker -type f ! -name build.json -print0
  printf '%s\0' Cargo.toml Cargo.lock rust-toolchain.toml crates/everruns/Cargo.toml crates/core/Cargo.toml crates/contracts/Cargo.toml crates/durable-object/Cargo.toml examples/durable-object-agent/Cargo.toml examples/durable-object-agent/Cargo.lock examples/durable-object-agent/wrangler.jsonc examples/durable-object-agent/build.sh
} | sort -z | xargs -0 sha256sum | sha256sum | cut -c1-24)"
export AGENT_BUILD_ID="counter-$BUILD_DIGEST"
cargo build --manifest-path "$APP_ROOT/Cargo.toml" --locked --release --target wasm32-unknown-unknown
wasm-bindgen "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/everruns_object_agent.wasm" --target web --out-dir "$APP_ROOT/pkg"
printf '{"agent":"counter","build_id":"%s","framework":1}\n' "$AGENT_BUILD_ID" > "$APP_ROOT/worker/build.json"
