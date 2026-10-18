#!/usr/bin/env bash
# Portability guard: the execution kernel (everruns-contracts and default everruns-core,
# followed by the core `engine` feature) builds for wasm32-unknown-unknown, so it can run
# inside a JavaScript isolate such as a celld or Cloudflare Durable Object.
#
# Check neutral contracts, providers, the public Framework facade and the
# shared Engine application. Celld imports that same application module.
#
# What breaks it: a dependency that needs threads, sockets or an OS clock
# (Tokio's `full` feature, mio, aws-lc), or `std::time::Instant::now`, which
# panics in the isolate. Use `everruns_contracts::rt` and `web_time` instead.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

python3 scripts/lib/check-session-hosting-boundary.py

TARGET=wasm32-unknown-unknown
if command -v rustup >/dev/null 2>&1; then
  rustup target add "$TARGET" >/dev/null
fi

echo "1. kernel crates for $TARGET"
cargo check --locked --target "$TARGET" --no-default-features \
  -p everruns-contracts -p everruns-core
cargo check --locked --target "$TARGET" --no-default-features \
  -p everruns-core --features engine

echo "2. provider protocols and official chat drivers for $TARGET"
cargo check --locked --target "$TARGET" --no-default-features \
  -p everruns-drivers --features openai,openrouter

echo "Shared actor execution host and Durable Object transport for $TARGET"
cargo check --locked --target "$TARGET" --no-default-features \
  --manifest-path crates/durable-object/Cargo.toml --features wasm
cargo check --locked --manifest-path examples/durable-object-agent/Cargo.toml --target "$TARGET"

echo "3. public Framework Engine, Agent, Session, tools, hooks and Harness for $TARGET"
cargo check --locked --target "$TARGET" --no-default-features \
  -p everruns --features macros,capabilities,builtins

echo "Wasm portability guard passed: public Framework and shared application build for $TARGET."
