#!/usr/bin/env bash
# Guard the non-workspace advisory scan: discovery must be complete, and the
# License Check workflow must actually run it on the paths it protects.
#
# The scan itself needs cargo-deny and the RustSec database, so it belongs in CI.
# What this test pins down is everything that can silently rot without it: the
# discovery query, and the workflow wiring that decides when the guard runs.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_ROOT"

GUARD="scripts/lib/check-nonworkspace-advisories.sh"
WORKFLOW=".github/workflows/licenses.yml"

failures=0
fail() {
  echo "FAIL: $1" >&2
  failures=$((failures + 1))
}

[ -x "$GUARD" ] || fail "$GUARD must be executable"

mapfile -t discovered < <(bash "$GUARD" --list)

# Discovery must not silently collapse to nothing — an empty list would make the
# guard pass while scanning zero lockfiles, which is the bug it exists to prevent.
if [ "${#discovered[@]}" -eq 0 ]; then
  fail "discovery returned no crates; the guard would scan nothing"
fi

# Cross-check discovery against git directly. Any committed Cargo.lock other than
# the workspace lock must be discovered, so adding one cannot bypass the guard.
mapfile -t expected < <(git ls-files -- '*Cargo.lock' | grep -v '^Cargo.lock$' | xargs -r -n1 dirname)

expected_sorted="$(printf '%s\n' "${expected[@]}" | LC_ALL=C sort)"
discovered_sorted="$(printf '%s\n' "${discovered[@]}" | LC_ALL=C sort)"
if [ "$expected_sorted" != "$discovered_sorted" ]; then
  fail "discovery does not match committed lockfiles
  expected:
$expected_sorted
  discovered:
$discovered_sorted"
fi

# The workspace lock is covered by the root `cargo deny check advisories`; scanning
# it again here would double-report every workspace advisory.
for dir in "${discovered[@]}"; do
  [ "$dir" = "." ] && fail "discovery must exclude the root workspace lockfile"
  [ -f "$dir/Cargo.toml" ] || fail "$dir has a Cargo.lock but no Cargo.toml"
done

# Wiring: the workflow must actually *run* the guard. Match the run step, not any
# mention — the guard's own path also appears in the `paths:` filters, so a plain
# substring search stays green even after the step is deleted.
grep -qE "^[[:space:]]*run:[[:space:]]*$GUARD[[:space:]]*$" "$WORKFLOW" ||
  fail "$WORKFLOW must have a 'run: $GUARD' step"

# Path filters must cover every discovered lockfile, or a lockfile-only change
# lands without the guard ever running.
paths_block="$(awk '/^on:/{on=1} on && /^jobs:/{exit} on{print}' "$WORKFLOW")"
for dir in "${discovered[@]}"; do
  lock="$dir/Cargo.lock"
  covered=0
  while IFS= read -r pattern; do
    # shellcheck disable=SC2053 # glob match against the workflow pattern is intended
    if [[ "$lock" == $pattern ]]; then
      covered=1
      break
    fi
  done < <(printf '%s\n' "$paths_block" | sed -n "s/^[[:space:]]*-[[:space:]]*'\(.*\)'[[:space:]]*$/\1/p")

  [ "$covered" -eq 1 ] || fail "$WORKFLOW paths filters do not cover $lock"
done

if [ "$failures" -gt 0 ]; then
  echo "$failures check(s) failed" >&2
  exit 1
fi

echo "OK: ${#discovered[@]} non-workspace lockfile(s) discovered and covered by $WORKFLOW"
