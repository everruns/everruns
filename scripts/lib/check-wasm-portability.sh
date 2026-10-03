#!/usr/bin/env bash
# Portability guard: the execution kernel (everruns-contracts, -core, -engine
# with default features off) builds for wasm32-unknown-unknown, so it can run
# inside a JavaScript isolate such as a celld or Cloudflare Durable Object.
#
# 1. `cargo check` the three crates for wasm32-unknown-unknown.
# 2. The engine-in-a-cell example (examples/celld-engine, its own workspace):
#    native tests of its step machine, and a wasm32 check of the Durable
#    Object itself.
#
# What breaks it: a dependency that needs threads, sockets or an OS clock
# (Tokio's `full` feature, mio, aws-lc), or `std::time::Instant::now`, which
# panics in the isolate. Use `everruns_provider::rt` and `web_time` instead.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

TARGET=wasm32-unknown-unknown
if command -v rustup >/dev/null 2>&1; then
  rustup target add "$TARGET" >/dev/null
fi

echo "1. kernel crates for $TARGET"
cargo check --locked --target "$TARGET" --no-default-features \
  -p everruns-contracts -p everruns-core --features everruns-core/engine

EXAMPLE=examples/celld-engine/Cargo.toml
echo "2. $EXAMPLE: native tests, then $TARGET"
cargo test --locked --manifest-path "$EXAMPLE"
cargo check --locked --manifest-path "$EXAMPLE" --target "$TARGET"

echo "Wasm portability guard passed: the kernel and the engine cell build for $TARGET."
