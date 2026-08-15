#!/usr/bin/env bash
# Keep every public `everruns` crate example on the facade API and compiled in CI.

set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

EXAMPLES_DIR="$PROJECT_ROOT/crates/everruns/examples"
MANIFEST="$PROJECT_ROOT/crates/everruns/Cargo.toml"
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

FILES="$WORK_DIR/files"
TARGETS="$WORK_DIR/targets"

find "$EXAMPLES_DIR" -maxdepth 1 -type f -name '*.rs' \
  -exec basename {} .rs \; | LC_ALL=C sort > "$FILES"

if [ ! -s "$FILES" ]; then
  echo "No public everruns examples found in $EXAMPLES_DIR" >&2
  exit 1
fi

cargo metadata --no-deps --format-version 1 \
  | jq -r --arg manifest "$MANIFEST" '
      .packages[]
      | select(.manifest_path == $manifest)
      | .targets[]
      | select(.kind | index("example"))
      | .name
    ' \
  | LC_ALL=C sort > "$TARGETS"

if ! diff -u "$FILES" "$TARGETS"; then
  echo "Filesystem examples and Cargo example targets differ." >&2
  exit 1
fi

for file in "$EXAMPLES_DIR"/*.rs; do
  if ! grep -Eq 'everruns::' "$file"; then
    echo "$(basename "$file") does not use the public everruns crate" >&2
    exit 1
  fi
done

if grep -En \
  'everruns[_-](core|host)|everruns::core(::|[^[:alnum:]_]|$)' \
  "$EXAMPLES_DIR"/*.rs; then
  echo "Public examples must not import internal crates or facade escape hatches." >&2
  exit 1
fi

echo "Discovered public everruns examples:"
sed 's/^/  - /' "$FILES"

if [ "${EVERRUNS_EXAMPLES_SKIP_COMPILE:-0}" = "1" ]; then
  echo "Example compilation covered by the caller."
  exit 0
fi

# One Cargo command discovers and compiles the complete target set. No example
# is executed here, so this check needs no provider credential or network call.
cargo check --locked -p everruns --all-features --examples

echo "All public everruns examples use the facade and compile."
