#!/usr/bin/env bash
# The reference authority executes shared Rust semantics even for native callers.
set -euo pipefail
WORKER_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$WORKER_ROOT/../../.." && pwd)"
if [[ -f /workspace/.tools/activate.sh ]]; then source /workspace/.tools/activate.sh; fi
: "${CARGO_TARGET_DIR:=$PROJECT_ROOT/target/wasm-agent}"
export CARGO_TARGET_DIR
cargo rustc --lib --crate-type cdylib --manifest-path "$WORKER_ROOT/../Cargo.toml" --locked --no-default-features --features wasm --target wasm32-unknown-unknown --release
wasm-bindgen "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/everruns_durable_object.wasm" --target web --out-dir "$WORKER_ROOT/pkg"
