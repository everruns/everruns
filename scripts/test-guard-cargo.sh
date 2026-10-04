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

# Vendor dependencies are optional in the consolidated driver crate. These
# fixtures model a forbidden subtree visible only when vendor features are on.
REAL_CARGO="$(command -v cargo)"
mkdir "$WORK/bin"
cat >"$WORK/bin/cargo" <<'CARGO'
#!/usr/bin/env bash
# Source guards inspect real standalone workspaces; only dependency trees
# are synthetic so the forbidden vendor edge is controlled by the fixture.
if [ "${1:-}" = metadata ]; then
  exec "$GUARD_REAL_CARGO" "$@"
fi
package=""
all_features=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    -p) package="$2"; shift ;;
    --all-features) all_features=1 ;;
  esac
  shift
done
printf '%s 0.35.0\n' "$package"
if [ "$package" = everruns-drivers ] && [ "$all_features" -eq 1 ]; then
  printf '%s 1.0.0\n' "$GUARD_FAKE_DEPENDENCY"
fi
case "$package" in
  everruns-server|everruns-worker|everruns) printf 'everruns-llmsim 0.35.0\n' ;;
esac
CARGO
chmod +x "$WORK/bin/cargo"

while read -r guard dependency; do
  set +e
  output="$(PATH="$WORK/bin:$PATH" GUARD_REAL_CARGO="$REAL_CARGO" GUARD_FAKE_DEPENDENCY="$dependency" \
    bash "scripts/lib/$guard" 2>"$WORK/err")"
  status=$?
  set -e
  check "$guard rejects vendor-only $dependency" "$([ "$status" -eq 1 ] && echo ok)"
  check "$guard reports the forbidden subtree" \
    "$(grep -q "^$dependency " <<<"$output" && echo ok)"
done <<'GUARDS'
check-provider-isolation.sh everruns-core
check-test-support-isolation.sh llmsim
check-agent-record-isolation.sh everruns-capabilities
check-observability-isolation.sh opentelemetry
GUARDS

if [ "$failures" -ne 0 ]; then
  echo "$failures check(s) failed"
  exit 1
fi
echo "guard cargo helper reports failures instead of swallowing them"
