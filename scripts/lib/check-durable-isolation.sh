#!/usr/bin/env bash
# Architecture guard: `everruns-durable` is a generic durable-execution engine
# (workflows, activities, tasks, signals, schedules) with no agent or turn
# semantics. Agent semantics live in durable-engine and server, which build on it.
#
# 1. The durable manifest declares no `everruns-*` normal or build dependency.
#    (A dev-dependency is allowed: the database-failure drift test pins the
#    engine's local log wording to `everruns-core`'s.)
# 2. `cargo tree` for the shipped (normal + build) edges contains no
#    `everruns-*` crate other than `everruns-durable` itself, so the engine
#    never compiles the agent stack.
# 3. `everruns-durable-engine` carries no transport: neither its manifest nor
#    its shipped (normal + build) tree contains `tonic` or
#    `everruns-internal-protocol`. The worker owns the gRPC stores and runner
#    constructors, so durable-engine can ship as a framework backend.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

# Keeps cargo's stderr, so "the guard could not run" never looks like
# "the guard found a violation". See guard-cargo.sh.
source "$SCRIPT_DIR/guard-cargo.sh"

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
  echo "Durable isolation guard failed. everruns-durable is a generic engine; move agent/turn semantics to everruns-durable-engine or everruns-server."
  exit 1
fi

# 3. durable-engine is transport-free (manifest and shipped tree).
ENGINE_MANIFEST=crates/durable-engine/Cargo.toml
if matches=$(awk '
  /^\[/ { section = $0 }
  /^[[:space:]]*(tonic[a-z0-9-]*|everruns-internal-protocol)[[:space:]]*[.=]/ {
    if (section != "[dev-dependencies]") print FILENAME ":" NR ": " section " " $0
  }
' "$ENGINE_MANIFEST"); [ -n "$matches" ]; then
  echo "everruns-durable-engine must not declare tonic or everruns-internal-protocol (normal/build):"
  echo "$matches"
  exit 1
fi
engine_tree=$(guard_cargo_tree -p everruns-durable-engine --edges normal,build --prefix none)
if leaked=$(echo "$engine_tree" | grep -E '^(tonic[a-z0-9-]* |everruns-internal-protocol )' | sort -u) \
  && [ -n "$leaked" ]; then
  echo "everruns-durable-engine must not depend on tonic or everruns-internal-protocol (normal/build edges):"
  echo "$leaked"
  echo "Keep gRPC stores and runner constructors in everruns-worker."
  exit 1
fi

# Worker composition enters the engine through the private durable-engine.
# `everruns-core` is allowed in the manifest only to select worker-only core
# features (MCP, telemetry, ...); source still goes through durable-engine.
if matches=$(awk '
  /^\[/ { section = $0 }
  /^[[:space:]]*(everruns-(engine|host|builtins|mcp|ag-ui|durable)|sqlx)[[:space:]]*[.=]/ {
    if (section != "[dev-dependencies]") print FILENAME ":" NR ": " section " " $0
  }
' crates/worker/Cargo.toml); [ -n "$matches" ]; then
  echo "Worker engine/database dependencies must pass through durable-engine:"
  echo "$matches"
  exit 1
fi
if matches=$(rg -n 'everruns_(core|engine|host|builtins|mcp|ag_ui|durable)::' crates/worker --glob '*.rs'); then
  echo "Worker source bypasses durable-engine:"
  echo "$matches"
  exit 1
fi
if ! grep -q '^publish = false$' crates/durable-engine/Cargo.toml; then
  echo "everruns-durable-engine is a private process entry point, not a published library"
  exit 1
fi

echo "Durable isolation guard passed: durable remains generic; durable-engine is transport-free; worker uses the private durable-engine entry."
