#!/usr/bin/env bash
# Check the hand-written docs catalogs against the code, and prove the check
# bites.
#
# scripts/check_docs_catalogs.py compares docs/capabilities/index.md, the
# built-in harness pages, docs/event-reference.md and the environment-variable
# summary against the registry snapshot (docs/api/capability-catalog.json) and
# the Rust sources. The first case runs it on this checkout. The rest replay
# the drift it exists to catch on a scratch copy and assert each one fails.
#
# The snapshot itself is kept fresh by a Rust test:
#   UPDATE_DOCS_CATALOG=1 cargo test -p everruns-server docs_catalog

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
ROOT="$(pwd)"
CHECK="$ROOT/scripts/check_docs_catalogs.py"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

failures=0

pass() { printf 'ok   %s\n' "$1"; }
fail() {
  printf 'FAIL %s\n' "$1" >&2
  failures=$((failures + 1))
}

# A scratch checkout: docs/ copied (the cases edit it), source trees linked
# file by file so a case can swap one file for an edited copy.
scratch() {
  rm -rf "$WORK/repo"
  mkdir -p "$WORK/repo"
  cp -R "$ROOT/docs" "$WORK/repo/docs"
  cp -Rs "$ROOT/crates" "$WORK/repo/crates"
  cp -Rs "$ROOT/integrations" "$WORK/repo/integrations"
}

# Replace a linked source file with a real copy so it can be edited.
own() {
  local file="$WORK/repo/$1"
  cp --remove-destination "$(readlink -f "$file")" "$file"
}

# expect_drift NAME PATTERN: the check fails and reports PATTERN.
expect_drift() {
  local name="$1" pattern="$2" output
  if output="$(python3 "$CHECK" --root "$WORK/repo" 2>&1)"; then
    fail "$name: check passed on drifted docs"
  elif grep -qF -- "$pattern" <<<"$output"; then
    pass "$name"
  else
    fail "$name: expected '$pattern' in:"$'\n'"$output"
  fi
}

if output="$(python3 "$CHECK" 2>&1)"; then
  pass "repository docs match the code"
else
  fail "repository docs drifted:"$'\n'"$output"
fi

INDEX=docs/capabilities/index.md

scratch
printf '| [Fake](/capabilities/fake/) | `fake_capability` | 1 |\n' >"$WORK/row"
sed -i "/^| \[File System\]/r $WORK/row" "$WORK/repo/$INDEX"
expect_drift "index lists a capability that does not exist" '`fake_capability` is not a registered capability'

scratch
sed -i '/`session_sql_database`/d' "$WORK/repo/$INDEX"
expect_drift "index omits a production capability" 'production capability `session_sql_database`'

scratch
sed -i 's/^| \[Daytona\](\/capabilities\/daytona\/) | `daytona` | 10 |/| [Daytona](\/capabilities\/daytona\/) | `daytona` | 9 |/' "$WORK/repo/$INDEX"
expect_drift "index states a wrong tool count" '`daytona` lists 9 tools, the registry has 10'

scratch
sed -i 's/| 1 (dev-only) |/| 1 |/' "$WORK/repo/$INDEX"
expect_drift "index drops a dev-only marker" '`computer_use` is registered only at dev grade'

scratch
sed -i '/^| \[E2B\](\/capabilities\/e2b\/) | \[Storage\]/d' "$WORK/repo/$INDEX"
expect_drift "dependencies table drops a dependency" "\`e2b\` depends on ['session_storage']"

scratch
sed -i 's/configures 25 capabilities/configures 24 capabilities/' "$WORK/repo/docs/built-ins/harnesses/generic.md"
expect_drift "harness page states a wrong count" 'says it configures 24 capabilities'

scratch
sed -i '/^| Platform |/d' "$WORK/repo/docs/built-ins/harnesses/platform-chat.md"
expect_drift "Agent page omits a capability" 'missing `platform`'

scratch
sed -i '/^| `budget.resumed` |/d' "$WORK/repo/docs/event-reference.md"
expect_drift "event reference omits an event" '`budget.resumed` is defined'

scratch
own crates/core/src/events/mod.rs
sed -i 's/^pub const FILE_WRITTEN: &str = "file.written";/&\npub const FILE_DELETED: \&str = "file.deleted";/' \
  "$WORK/repo/crates/core/src/events/mod.rs"
expect_drift "code adds an event the reference lacks" '`file.deleted` is defined'

scratch
printf '| `subagent.started` | Retired. |\n' >"$WORK/row"
sed -i "/^| \`file.written\` |/r $WORK/row" "$WORK/repo/docs/event-reference.md"
expect_drift "event reference lists a retired event" '`subagent.started` is in the table but not defined'

scratch
sed -i 's/^| \[`VALKEY_URL`\](#valkey_url) |/| [`VALKEY_ADDR`](#valkey_url) |/' "$WORK/repo/docs/sre/environment-variables.md"
expect_drift "env summary names a variable nothing reads" '`VALKEY_ADDR`'

if [ "$failures" -gt 0 ]; then
  echo "$failures docs catalog case(s) failed" >&2
  exit 1
fi
echo "All docs catalog cases passed."
