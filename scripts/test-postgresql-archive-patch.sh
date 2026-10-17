#!/usr/bin/env bash
# Exercise the patched archive API without adding a second repository lockfile.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FIXTURE="$(mktemp -d)"
trap 'rm -rf "$FIXTURE"' EXIT
cp -R "$ROOT/vendor/postgresql_archive" "$FIXTURE/archive"
cp "$ROOT/Cargo.lock" "$FIXTURE/archive/Cargo.lock"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}" cargo test \
  --manifest-path "$FIXTURE/archive/Cargo.toml" --no-default-features \
  --features zonky,tls-rustls-aws-lc-rs --test zip_compatibility
