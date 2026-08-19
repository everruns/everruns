#!/usr/bin/env bash
# Reclaim Rust build-cache disk without discarding compiled artifacts.
#
# Only incremental-compilation state is removed. Incremental caches are the
# largest single consumer of `target/` (measured: 2.6 GB against 2.0 GB of
# actual artifacts after building core/platform/provider alone) because they
# store per-CGU dependency graphs for crates whose metadata is already large.
# Dropping them costs one non-incremental rebuild (~2.5x a warm edit rebuild
# of `everruns-core`) and keeps every .rlib, binary, and build-script output.
#
# Prunes the workspace target dir and the shared pre-push target dir
# (scripts/lib/pre-push-context.sh), which already builds with
# CARGO_INCREMENTAL=0 but may hold state from other entry points.
#
# Usage: bash scripts/lib/prune-build-cache.sh

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=scripts/lib/pre-push-context.sh
source "$PROJECT_ROOT/scripts/lib/pre-push-context.sh"

dir_size() {
  [ -d "$1" ] || { echo 0; return; }
  du -sk "$1" 2>/dev/null | awk '{print $1}'
}

human() {
  awk -v kb="$1" 'BEGIN {
    split("KB MB GB TB", unit, " ")
    i = 1
    while (kb >= 1024 && i < 4) { kb /= 1024; i++ }
    printf "%.1f %s\n", kb, unit[i]
  }'
}

freed_kb=0

prune_target() {
  local target_dir="$1" label="$2" incremental before

  [ -d "$target_dir" ] || return 0

  # Only recurse into a directory cargo owns. CARGO_TARGET_DIR is inherited from
  # the caller's environment, and this function deletes what it finds, so a
  # stray or misspelled value must not turn into `rm -rf` against an unrelated
  # tree. Cargo writes CACHEDIR.TAG into every target dir it creates.
  if [ ! -f "$target_dir/CACHEDIR.TAG" ]; then
    echo "   skipped $label $target_dir (not a cargo target directory)"
    return 0
  fi

  # Incremental state lives at <target>/<profile>/incremental.
  while IFS= read -r incremental; do
    before="$(dir_size "$incremental")"
    [ "$before" -gt 0 ] || continue
    rm -rf "$incremental"
    freed_kb=$((freed_kb + before))
    echo "   removed $label ${incremental#"$target_dir"/} ($(human "$before"))"
  done < <(find "$target_dir" -maxdepth 2 -type d -name incremental)
}

echo "Pruning Rust incremental caches..."
prune_target "${CARGO_TARGET_DIR:-$PROJECT_ROOT/target}" "workspace"
prune_target "$(pre_push_shared_target_dir)" "pre-push"

if [ "$freed_kb" -eq 0 ]; then
  echo "   nothing to prune"
else
  echo "Freed $(human "$freed_kb"). Next build is non-incremental."
fi
