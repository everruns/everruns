#!/usr/bin/env bash
# Guard the option-1 Engine boundary: Scale owns portable distributed
# orchestration, durable stays generic, and product crates consume Scale.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$(dirname "$SCRIPT_DIR")")"
cd "$PROJECT_ROOT"

failures=()

search_source() {
  local pattern="$1"
  shift
  if command -v rg >/dev/null 2>&1; then
    rg -q "$pattern" "$@"
  else
    grep -REq --include='*.rs' --include='Cargo.toml' "$pattern" "$@"
  fi
}

if search_source 'everruns-(server|worker)' crates/scale/Cargo.toml; then
  failures+=("everruns-scale must not depend on server or worker")
fi

if search_source '^everruns(-|[[:space:]]*=)' crates/durable/Cargo.toml; then
  failures+=("everruns-durable must remain a generic workflow substrate")
fi

if search_source 'trait AgentRunner|struct DurableRunner|dyn AgentRunner|DurableRunner::' \
  crates/server crates/worker crates/scale; then
  failures+=("the legacy AgentRunner/DurableRunner strategy must not return")
fi

if ! search_source 'everruns-scale\.workspace = true' crates/server/Cargo.toml; then
  failures+=("everruns-server must adopt everruns-scale")
fi

if ! search_source 'everruns-scale\.workspace = true' crates/worker/Cargo.toml; then
  failures+=("everruns-worker must adopt everruns-scale")
fi

if search_source 'impl[[:space:]]+.*Serialize[[:space:]]+for[[:space:]]+Agent|serde::Serialize.*Agent' crates/scale crates/everruns; then
  failures+=("arbitrary everruns::Agent values must never be serialized")
fi

for required in \
  'pub struct PortableAgent' \
  'pub struct RegistrationPath' \
  'pub struct Registry' \
  'impl Engine for ScaleEngine' \
  'PortableAgentRequired' \
  'everruns_scale_schema_migrations'; do
  if ! search_source "$required" crates/scale crates/everruns/src/engine.rs; then
    failures+=("missing Scale boundary marker: $required")
  fi
done

if ((${#failures[@]})); then
  printf 'FAIL: %s\n' "${failures[@]}" >&2
  exit 1
fi

echo "Scale Engine boundary verified: portable public crate, generic durable substrate, product adoption."
