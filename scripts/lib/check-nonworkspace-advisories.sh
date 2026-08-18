#!/usr/bin/env bash
# RustSec advisory scan for every committed Cargo.lock that is NOT the workspace lock.
#
# `cargo deny check advisories` at the repo root only ever resolves the workspace
# Cargo.lock. Crates in the root Cargo.toml `[workspace] exclude` array carry their
# own lockfile and separate dependency tree, so without this guard an advisory in
# one of them is reported by nothing (EVE-915; same failure mode as EVE-914).
#
# Scope is committed lockfiles: a checked-in Cargo.lock is the pin we actually ship
# and reproduce. Excluded crates without one (research/*) resolve fresh at build
# time and are deliberately out of scope here.
#
# Policy comes from the repo's single deny.toml, so this guard cannot drift from
# the workspace gate — same tool, same ignore list, same yanked/unmaintained rules.
#
# Usage:
#   check-nonworkspace-advisories.sh          scan every discovered lockfile
#   check-nonworkspace-advisories.sh --list   print discovered crate dirs, scan nothing

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

# Derive the list from git rather than a hand-maintained array: a new excluded
# crate is covered the moment its lockfile is committed, with nothing to update.
discover_crate_dirs() {
  git ls-files -- '*Cargo.lock' | while IFS= read -r lock; do
    [ "$lock" = "Cargo.lock" ] && continue # the workspace lock, already covered
    printf '%s\n' "$(dirname "$lock")"
  done
}

mapfile -t crate_dirs < <(discover_crate_dirs)

if [ "${1:-}" = "--list" ]; then
  # Guard the empty case explicitly: `printf '%s\n' "${empty[@]}"` emits a blank
  # line, which a caller would read back as one phantom entry.
  [ "${#crate_dirs[@]}" -gt 0 ] && printf '%s\n' "${crate_dirs[@]}"
  exit 0
fi

if [ "${#crate_dirs[@]}" -eq 0 ]; then
  echo "No non-workspace lockfiles found; nothing to scan."
  exit 0
fi

# cargo-deny resolves --config relative to the *manifest* directory, not the cwd,
# so a bare "deny.toml" silently falls back to cargo-deny's built-in defaults and
# scans with the wrong policy. Always pass the repo config by absolute path.
DENY_CONFIG="$PROJECT_ROOT/deny.toml"
if [ ! -f "$DENY_CONFIG" ]; then
  echo "::error::deny.toml not found at $DENY_CONFIG" >&2
  exit 1
fi

failed=""
for dir in "${crate_dirs[@]}"; do
  manifest="$dir/Cargo.toml"
  if [ ! -f "$manifest" ]; then
    echo "::error::'$dir' has a Cargo.lock but no Cargo.toml (orphaned lockfile?)"
    failed="$failed $dir(no-manifest)"
    continue
  fi

  echo "::group::cargo deny check advisories — $dir"
  # Keep going after a failure so one bad tree does not mask advisories in another.
  if cargo deny --manifest-path "$manifest" --config "$DENY_CONFIG" check advisories; then
    echo "OK: $dir"
  else
    echo "::error::Advisory check failed for non-workspace crate: $dir"
    failed="$failed $dir"
  fi
  echo "::endgroup::"
done

if [ -n "$failed" ]; then
  echo "Advisory check failed for:$failed"
  exit 1
fi

echo "All ${#crate_dirs[@]} non-workspace lockfiles passed the advisory check."
