#!/usr/bin/env bash
# Build and run real out-of-workspace consumers of the published crates:
#
#   external-consumer-app     runs offline turns on the `everruns` facade,
#                             including capabilities installed from the
#                             external capability pack.
#   external-capability-pack  implements `IntoCapability` and a code-defined
#                             capability against the neutral
#                             `everruns-capability` contract ALONE (EVE-873),
#                             so the open capability seams fail here if they
#                             stop being usable without core/host access.
#   external-builtin-pack     composes `everruns-builtins` into a fresh core
#                             registry without relying on link-time discovery.
#   external-provider-pack    implements the `ChatDriver` contract and driver
#                             registration against the provider SPI
#                             (`everruns-provider`) ALONE (EVE-874), so custom
#                             downstream providers fail here if they stop
#                             compiling without core/host access.
#   external-event-log        implements the canonical `EventLog`/`EventReader`
#                             SPI and replaces every default `HostBackends`
#                             store through public traits, so downstream host
#                             composition fails here if a seam closes.
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
  cargo test --quiet --locked --manifest-path "$FIXTURE" -p external-capability-pack

echo "External capability pack builds on the neutral everruns-capability contract under -D warnings."

CARGO_TARGET_DIR="$TARGET_DIR" RUSTFLAGS="-D warnings" \
  cargo test --quiet --locked --manifest-path "$FIXTURE" -p external-builtin-pack

echo "External host composes the portable built-in policy bundle explicitly under -D warnings."

CARGO_TARGET_DIR="$TARGET_DIR" RUSTFLAGS="-D warnings" \
  cargo test --quiet --locked --manifest-path "$FIXTURE" -p external-provider-pack

echo "External provider pack builds on the provider SPI (everruns-provider) alone under -D warnings."

CARGO_TARGET_DIR="$TARGET_DIR" RUSTFLAGS="-D warnings" \
  cargo test --quiet --locked --manifest-path "$FIXTURE" -p external-event-log

echo "External event log implements the public host SPI under -D warnings."
