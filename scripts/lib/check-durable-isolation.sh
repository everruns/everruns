#!/usr/bin/env bash
# Architecture guard: `everruns-durable` is a generic durable-execution engine
# (workflows, activities, tasks, schedules) with no agent or turn semantics.
# What its tasks mean lives in the worker and server, which build on it.
#
# 1. The durable manifest declares no `everruns-*` normal or build dependency.
#    (A dev-dependency is allowed: the database-failure drift test pins the
#    engine's local log wording to `everruns-core`'s.)
# 2. `cargo tree` for the shipped (normal + build) edges contains no
#    `everruns-*` crate other than `everruns-durable` itself, so the engine
#    never compiles the agent stack.
# 3. Worker source reaches core and the durable queue through its crate
#    re-exports (`crate::core`, `crate::durable`, ...), never by crate name,
#    so lib.rs is the one place that names them.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

# Keeps cargo's stderr, so "the guard could not run" never looks like
# "the guard found a violation". See guard-cargo.sh.
source "$SCRIPT_DIR/guard-cargo.sh"

if ! command -v rg >/dev/null 2>&1; then
  echo "Guard could not run: ripgrep (rg) is required for the worker source scan." >&2
  exit "$GUARD_INFRA_EXIT"
fi

MANIFEST=crates/durable/Cargo.toml
FAILED=0

# 1. Manifest: only [dev-dependencies] may name an everruns-* crate.
if matches=$(awk '
  /^\[/ { section = $0 }
  /^[[:space:]]*everruns-[a-z0-9-]+[[:space:]]*[.=]/ {
    if (section != "[dev-dependencies]") print FILENAME ":" NR ": " section " " $0
  }
' "$MANIFEST"); [ -n "$matches" ]; then
  echo "everruns-durable must not declare an everruns-* normal or build dependency:"
  echo "$matches"
  FAILED=1
fi

# 2. Shipped dependency tree: no everruns-* crate besides the engine itself.
tree=$(guard_cargo_tree -p everruns-durable --edges normal,build --prefix none)
if leaked=$(echo "$tree" | grep -E '^everruns-' | grep -vE '^everruns-durable ' | sort -u) \
  && [ -n "$leaked" ]; then
  echo "everruns-durable must not depend on any everruns-* crate (normal/build edges):"
  echo "$leaked"
  FAILED=1
fi

if [ "$FAILED" -ne 0 ]; then
  echo "Durable isolation guard failed. everruns-durable is a generic engine; move agent/turn semantics to everruns-worker or everruns-server."
  exit 1
fi

# 3. Worker source names core and durable only in lib.rs.
if matches=$(rg -n 'everruns_(core|durable)::' crates/worker --glob '*.rs' --glob '!crates/worker/src/lib.rs'); then
  echo "Worker source must reach core and durable through the crate re-exports in lib.rs:"
  echo "$matches"
  exit 1
else
  scan_status=$?
  if [ "$scan_status" -ne 1 ]; then
    echo "Guard could not run: worker source scan exited $scan_status; the boundary was not checked." >&2
    exit "$GUARD_INFRA_EXIT"
  fi
fi

echo "Durable isolation guard passed: durable remains generic; worker source reaches core and durable through its re-exports."
