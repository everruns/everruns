#!/usr/bin/env bash
# Cell control plane calls guard: a cell reaches its control plane only
# through the link module and the host call path.
#
# Why: a data plane should run with no control plane at all. Its traffic with
# one is pushed desired state, reports on the feed, and host calls
# (`everruns_contracts::host_calls`), the one way to ask at call time. A new
# feature that builds its own request to the control plane adds a dependency
# nobody declared, which then fails on a standalone cell and hides from the
# registry that lists what a data plane needs.
#
# What it checks: in `crates/agentd/src`, outside tests, the control plane's
# outbound paths (`/dp/v1/tenants/...` and `/dp/v1/feed`) appear only in the
# allowlisted files. Inbound `/dp/v1/orgs/...` routes the cell serves are not
# calls and are not checked.
#
# Fix a violation by declaring a host call (`crates/contracts/src/host_calls.rs`)
# or by pushing the value as desired state, not by adding to the allowlist.
#
# Usage: check-cell-control-plane-calls.sh [CELL_SRC_DIR]
#
# Used by: scripts/lib/pre-push.sh, the repository guards CI job.
# Exits 0 on success, 1 on violation. Never silently skips.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

SRC="${1:-crates/agentd/src}"
if [ ! -d "$SRC" ]; then
  echo "Cell control plane calls guard: missing $SRC; the guard would check nothing." >&2
  exit 1
fi

ALLOWED=(
  "node_control.rs"   # the link: feed, allowances
  "host_calls.rs"     # the host call path
  "gateway_dp.rs"     # the legacy lookup route; remove with the release after host calls
)

PATTERN='/dp/v1/(tenants/|feed")'

FAILED=0
checked=0
while IFS= read -r -d '' file; do
  rel="${file#"$SRC"/}"
  case "$rel" in *_tests.rs | tests/* | */tests/*) continue ;; esac
  checked=$((checked + 1))
  skip=0
  for allowed in "${ALLOWED[@]}"; do
    [ "$rel" = "$allowed" ] && skip=1
  done
  [ "$skip" -eq 1 ] && continue
  hits=$(awk '/^[[:space:]]*#\[cfg\(test\)\]/ { exit } { print FILENAME ":" FNR ": " $0 }' "$file" \
    | grep -E "$PATTERN" | grep -vE ':[0-9]+: *//' || true)
  if [ -n "$hits" ]; then
    echo "$hits" | sed 's/^/  /'
    FAILED=1
  fi
done < <(find "$SRC" -type f -name '*.rs' -print0)

if [ "$FAILED" -ne 0 ]; then
  echo "Cell control plane calls guard failed: a cell asks its control plane only"
  echo "through a declared host call (crates/contracts/src/host_calls.rs)."
  exit 1
fi

echo "Cell control plane calls guard passed: $checked files reach the control plane only through the link and host calls."
