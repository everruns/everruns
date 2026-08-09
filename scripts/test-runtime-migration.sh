#!/usr/bin/env bash
# Build and run real downstream consumers on both sides of the 0.17 migration.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
FIXTURE="$PROJECT_ROOT/tests/fixtures/runtime-migration/Cargo.toml"
TARGET_DIR="$(mktemp -d "${TMPDIR:-/tmp}/everruns-runtime-migration.XXXXXX")"
trap 'rm -rf "$TARGET_DIR"' EXIT

if rg -n '^#!\[deprecated' "$PROJECT_ROOT/crates/runtime/src/lib.rs"; then
  echo "everruns-runtime must not use blanket crate deprecation" >&2
  exit 1
fi

legacy="$({
  CARGO_TARGET_DIR="$TARGET_DIR" RUSTFLAGS="-D warnings" \
    cargo run --quiet --locked --manifest-path "$FIXTURE" -p runtime-migration-legacy
})"
framework="$({
  CARGO_TARGET_DIR="$TARGET_DIR" RUSTFLAGS="-D warnings" \
    cargo run --quiet --locked --manifest-path "$FIXTURE" -p runtime-migration-framework
})"

if [ "$legacy" != "4" ] || [ "$framework" != "$legacy" ]; then
  echo "runtime migration behavior mismatch: legacy='$legacy' framework='$framework'" >&2
  exit 1
fi

if rg -n 'everruns[-_]runtime' "$PROJECT_ROOT/tests/fixtures/runtime-migration/framework"; then
  echo "Framework migration fixture must not depend on or import everruns-runtime" >&2
  exit 1
fi

echo "Runtime migration fixtures agree under -D warnings: $framework"
