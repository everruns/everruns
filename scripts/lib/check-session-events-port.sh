#!/usr/bin/env bash
# Session events port guard: control plane features read a session's events
# through `domains::sessions::events`, never the `events` table directly.
#
# Why: a session that runs on data plane v2 keeps its events in its cell, and
# the control plane's table has none of them. A feature that calls
# `StorageBackend::list_events` or `list_events_advanced` itself sees an
# empty session there and fails quietly: no pending question, an eval that
# never ends, a summary of nothing. The port answers the same rows for either
# plane, so a feature written against it works on both.
#
# The allowlist below is the v1 runtime: code that only ever runs for a
# session on the platform's own workers, or that reads after its own data
# plane branch. Each entry says why. Test code is not checked: files named
# `tests*.rs` or `*_tests.rs`, `tests/` folders, and everything after a
# file's first `#[cfg(test)]`.
#
# Fix a violation by calling `crate::domains::sessions::events::list` or
# `::advanced` with the same arguments, not by adding to the allowlist.
#
# Usage: check-session-events-port.sh [SERVER_SRC_DIR]
#   SERVER_SRC_DIR defaults to crates/server/src.
#
# Used by: scripts/lib/pre-push.sh, the repository guards CI job.
# Exits 0 on success, 1 on violation. Never silently skips.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

SRC="${1:-crates/server/src}"
if [ ! -d "$SRC" ]; then
  echo "Session events port guard: missing $SRC; the guard would check nothing." >&2
  exit 1
fi

# Path prefixes under $SRC, and why each may read the table.
ALLOWED=(
  "storage/"                                  # the table itself
  "domains/sessions/events.rs"                # the port
  "services/event/"                           # the v1 event service: writes, replay, human waits
  "turns/"                                    # the v1 turn runtime
  "background/tool_result_timeout.rs"         # sweeps turns parked on the platform's workers
  "live_updates/"                             # v1 delivery; a v2 session streams from its cell
  "domains/sessions/commands/mod.rs"          # reads after its own data plane branch
  "domains/sessions/service/fork.rs"          # v1 fork; fork_plane.rs forks a v2 session
  "domains/messages/service/mod.rs"           # v1 admission; a v2 session admits in its cell
)

allowed() {
  local rel="$1" prefix
  for prefix in "${ALLOWED[@]}"; do
    case "$rel" in "$prefix"*) return 0 ;; esac
  done
  return 1
}

FAILED=0
checked=0
while IFS= read -r -d '' file; do
  rel="${file#"$SRC"/}"
  case "$rel" in tests/* | */tests/* | tests*.rs | */tests*.rs | *_tests.rs) continue ;; esac
  checked=$((checked + 1))
  allowed "$rel" && continue
  # Production code only: stop at the first `#[cfg(test)]`. A call can
  # span lines (`db\n.list_events(`), so match the method name.
  hits=$(awk '/^[[:space:]]*#\[cfg\(test\)\]/ { exit } /\.?list_events(_advanced)?\(/ { print FILENAME ":" FNR ": " $0 }' "$file" \
    | grep -E '(^|[^_[:alnum:]])list_events(_advanced)?\(' \
    | grep -vE 'fn list_events' || true)
  if [ -n "$hits" ]; then
    echo "$hits" | sed 's/^/  /'
    FAILED=1
  fi
done < <(find "$SRC" -type f -name '*.rs' -print0)

if [ "$FAILED" -ne 0 ]; then
  echo "Session events port guard failed: read a session's events with"
  echo "crate::domains::sessions::events::{list, advanced}, which also answers for a"
  echo "session on data plane v2."
  exit 1
fi

echo "Session events port guard passed: $checked files read session events through the port or are v1 runtime."
