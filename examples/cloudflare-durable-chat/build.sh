#!/usr/bin/env bash
set -euo pipefail
APP_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$APP_ROOT/../.." && pwd)"
if [[ -f /workspace/.tools/activate.sh ]]; then source /workspace/.tools/activate.sh; fi
: "${CARGO_TARGET_DIR:=$APP_ROOT/target}"
export CARGO_TARGET_DIR
BUILD_DIGEST="$(cd "$PROJECT_ROOT"
{
  find crates/everruns/src crates/core/src crates/contracts/src crates/drivers/drivers/src crates/durable-object/src crates/durable-object/worker/src examples/cloudflare-durable-chat/src examples/cloudflare-durable-chat/worker -type f ! -name build.json -print0
  printf '%s\0' Cargo.toml Cargo.lock rust-toolchain.toml crates/everruns/Cargo.toml crates/core/Cargo.toml crates/contracts/Cargo.toml crates/drivers/drivers/Cargo.toml crates/durable-object/Cargo.toml examples/cloudflare-durable-chat/Cargo.toml examples/cloudflare-durable-chat/Cargo.lock examples/cloudflare-durable-chat/wrangler.jsonc examples/cloudflare-durable-chat/build.sh
} | sort -z | xargs -0 sha256sum | sha256sum | cut -c1-24)"
export AGENT_BUILD_ID="chat-$BUILD_DIGEST"
cargo build --manifest-path "$APP_ROOT/Cargo.toml" --locked --release --target wasm32-unknown-unknown
wasm-bindgen "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/everruns_cloudflare_durable_chat.wasm" --target web --out-dir "$APP_ROOT/pkg"
printf '{"build_id":"%s","framework":1}\n' "$AGENT_BUILD_ID" > "$APP_ROOT/worker/build.json"
