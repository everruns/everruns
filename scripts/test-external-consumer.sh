#!/usr/bin/env bash
# Build and run real out-of-workspace consumers of the published crates:
#
#   external-consumer-app  runs one offline turn on the `everruns` facade.
#   external-event-log     implements the canonical `EventLog`/`EventReader`
#                          SPI from `everruns-host` and supplies it to host
#                          composition, so the SPI fails here if it stops being
#                          implementable without in-crate access.
#
# The fixtures are deliberately outside the workspace so they resolve the
# published crates the way a downstream application does, and they build under
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

CARGO_TARGET_DIR="$TARGET_DIR" RUSTFLAGS="-D warnings" \
  cargo test --quiet --locked --manifest-path "$FIXTURE" -p external-event-log

echo "External event log implements the public host SPI under -D warnings."
