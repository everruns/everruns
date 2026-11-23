#!/usr/bin/env bash
# Shard control plane calls guard: a shard reaches its control plane only
# through the link module.
#
# Why: a data plane should run with no control plane at all. Its traffic with
# one is pushed desired state and reports on the feed; at call time a
# session's tools reach the control plane's injected MCP servers, named in the
# session's create (`injected::CONTROLLER_SERVERS`), with a session token. A
# new feature that builds its own request to the control plane adds a
# dependency nobody declared, which then fails on a standalone shard.
#
# What it checks: in `crates/agentd/src`, outside tests, the control plane's
# outbound paths (`/dp/v1/tenants/...` and `/dp/v1/feed`) appear only in the
# allowlisted files. Routes served (`.route(...)` lines: the shard's inbound
# `/dp/v1/orgs/...`, and the control plane a standalone install runs for its
# own shards, `install_stream.rs`) are not calls and are not checked.
#
# Fix a violation by serving the call from an injected server or by pushing
# the value as desired state, not by adding to the allowlist.
#
# Usage: check-agentd-control-plane-calls.sh [AGENTD_SRC_DIR]
#
# Used by: scripts/lib/pre-push.sh, the repository guards CI job.
# Exits 0 on success, 1 on violation. Never silently skips.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

SRC="${1:-crates/agentd/src}"
if [ ! -d "$SRC" ]; then
  echo "Shard control plane calls guard: missing $SRC; the guard would check nothing." >&2
  exit 1
fi

ALLOWED=(
  "node_control.rs"   # the link: feed, allowances
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
    | grep -E "$PATTERN" | grep -vE ':[0-9]+: *//' | grep -vE ':[0-9]+: *\.route\(' || true)
  if [ -n "$hits" ]; then
    echo "$hits" | sed 's/^/  /'
    FAILED=1
  fi
done < <(find "$SRC" -type f -name '*.rs' -print0)

if [ "$FAILED" -ne 0 ]; then
  echo "Shard control plane calls guard failed: a shard asks its control plane only"
  echo "through the link (node_control.rs), its injected servers, or pushed state."
  exit 1
fi

echo "Shard control plane calls guard passed: $checked files reach the control plane only through the link."
