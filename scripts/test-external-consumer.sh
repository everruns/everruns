#!/usr/bin/env bash
# Build and run a real out-of-workspace consumer against the public `everruns`
# Framework crate. The fixture is deliberately outside the workspace so it
# resolves the facade the way a downstream application does, and it runs under
# `-D warnings` so a deprecated or noisy public path fails the build.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
FIXTURE="$PROJECT_ROOT/tests/fixtures/external-consumer/Cargo.toml"
TARGET_DIR="$(mktemp -d "${TMPDIR:-/tmp}/everruns-external-consumer.XXXXXX")"
trap 'rm -rf "$TARGET_DIR"' EXIT

answer="$({
  CARGO_TARGET_DIR="$TARGET_DIR" RUSTFLAGS="-D warnings" \
    cargo run --quiet --locked --manifest-path "$FIXTURE" -p external-consumer-app
})"

if [ "$answer" != "4" ]; then
  echo "external consumer produced '$answer', expected '4'" >&2
  exit 1
fi

echo "External consumer runs on the public everruns facade under -D warnings: $answer"
