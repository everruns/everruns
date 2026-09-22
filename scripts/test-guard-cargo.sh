#!/usr/bin/env bash
# Exercise scripts/lib/guard-cargo.sh, and check the guards actually use it.
#
# The behaviour under test is a distinction, not an output: "the guard could not
# run" must not look like "the guard found a violation". Before this helper the
# guards ran `cargo tree ... 2>/dev/null` under `set -euo pipefail`, so any
# cargo failure produced a bare `exit code 101` with no output at all — which is
# what an architectural violation would have produced too.

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

failures=0

check() {
  local name="$1" condition="$2"
  if [ "$condition" = "ok" ]; then
    echo "ok   $name"
  else
    echo "FAIL $name"
    failures=$((failures + 1))
  fi
}

# --- success passes the tree through unchanged ------------------------------

set +e
output="$(bash -c 'set -euo pipefail
  source scripts/lib/guard-cargo.sh
  guard_cargo_tree -p everruns-core --edges normal,build --prefix none' 2>"$WORK/err")"
status=$?
set -e

check "a working tree exits 0" "$([ "$status" -eq 0 ] && echo ok)"
check "a working tree returns the tree" \
  "$(grep -q '^everruns-core ' <<<"$output" && echo ok)"
check "a working tree stays quiet on stderr" \
  "$([ ! -s "$WORK/err" ] && echo ok)"

# --- a cargo failure is reported, not swallowed -----------------------------

set +e
output="$(bash -c 'set -euo pipefail
  source scripts/lib/guard-cargo.sh
  guard_cargo_tree -p definitely-not-a-real-crate --prefix none' 2>"$WORK/err")"
status=$?
set -e

# Exit 2, not 1: a violation and an unrunnable check must stay distinguishable.
check "a cargo failure exits 2, not 1" "$([ "$status" -eq 2 ] && echo ok)"
check "a cargo failure says the guard could not run" \
  "$(grep -q 'Guard could not run' "$WORK/err" && echo ok)"
check "a cargo failure disclaims a violation" \
  "$(grep -q 'not an architectural violation' "$WORK/err" && echo ok)"
# The whole point: cargo's own words survive.
check "a cargo failure relays what cargo said" \
  "$(grep -q 'did not match any packages' "$WORK/err" && echo ok)"

# --- every guard that shells out to cargo tree uses the helper --------------
#
# A guard that goes back to `2>/dev/null` silently re-earns the opacity, so the
# one deliberate exception is named here rather than left to a reviewer's eye.

unguarded=0
while IFS= read -r file; do
  while IFS= read -r line; do
    # `-i tokio` exits non-zero when tokio is absent, which is the passing case
    # in check-core-kernel-dependencies.sh, so it keeps `|| true`.
    case "$line" in
      *"|| true"*) continue ;;
    esac
    echo "  unguarded cargo tree in $file: $line"
    unguarded=$((unguarded + 1))
  done < <(grep -n 'cargo tree.*2>/dev/null' "$file" || true)
done < <(grep -l 'cargo tree' scripts/lib/check-*.sh)

check "no guard discards cargo's stderr" "$([ "$unguarded" -eq 0 ] && echo ok)"

if [ "$failures" -ne 0 ]; then
  echo "$failures check(s) failed"
  exit 1
fi
echo "guard cargo helper reports failures instead of swallowing them"
