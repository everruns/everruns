#!/usr/bin/env bash
# Guard scripts/lib/prune-build-cache.sh against the one way it could hurt:
# reclaiming disk by deleting artifacts instead of incremental state.
#
# `just prune` exists so a developer can free several GB without paying for a
# cold rebuild. If it ever removed .rlib/.rmeta/binaries it would still "work"
# — the disk number improves either way — so the distinction is asserted here
# rather than left to whoever notices the next rebuild took ten minutes.

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

target="$workdir/target"
mkdir -p "$target/debug/incremental/everruns_core-abc" \
  "$target/debug/deps" \
  "$target/release/incremental/everruns_core-def" \
  "$target/debug/build/everruns-core-123"

echo incremental-state >"$target/debug/incremental/everruns_core-abc/dep-graph.bin"
echo incremental-state >"$target/release/incremental/everruns_core-def/dep-graph.bin"
echo artifact >"$target/debug/deps/libeverruns_core-abc.rlib"
echo artifact >"$target/debug/deps/libeverruns_core-abc.rmeta"
echo artifact >"$target/debug/build/everruns-core-123/output"

output="$(TMPDIR="$workdir" CARGO_TARGET_DIR="$target" bash "$PROJECT_ROOT/scripts/lib/prune-build-cache.sh")"

fail() {
  echo "FAIL: $1" >&2
  echo "--- prune output ---" >&2
  echo "$output" >&2
  exit 1
}

for profile in debug release; do
  [ ! -d "$target/$profile/incremental" ] ||
    fail "$profile/incremental survived the prune"
done

for kept in \
  "$target/debug/deps/libeverruns_core-abc.rlib" \
  "$target/debug/deps/libeverruns_core-abc.rmeta" \
  "$target/debug/build/everruns-core-123/output"; do
  [ -f "$kept" ] || fail "prune deleted a compiled artifact: ${kept#"$target"/}"
done

# A second run must stay quiet and exit clean: the recipe is safe to re-run.
output="$(TMPDIR="$workdir" CARGO_TARGET_DIR="$target" bash "$PROJECT_ROOT/scripts/lib/prune-build-cache.sh")"
case "$output" in
*"nothing to prune"*) ;;
*) fail "re-running prune on a clean target did not report 'nothing to prune'" ;;
esac

echo "PASS: prune-build-cache removes incremental state and keeps artifacts"
