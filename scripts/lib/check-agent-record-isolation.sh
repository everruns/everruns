#!/usr/bin/env bash
# Architecture guard (EVE-877, EVE-881): stored platform persistence records —
# Agent/AgentVersion (lifecycle status, versioning/publication metadata, fork
# lineage) and Harness (lifecycle status, hierarchy identifiers, built-in
# flags, display metadata, timestamps) plus the built-in harness provisioning
# templates — live in crates/platform (`everruns-platform`). The execution
# kernel consumes only the portable `everruns_core::AgentDefinition` /
# `everruns_core::HarnessDefinition` and the resolved execution snapshot:
#
# 1. Kernel crate sources (core, engine, provider, capability) must not
#    reference `everruns_platform` at all — the platform Agent/Harness records
#    must never flow back into the kernel.
# 2. Kernel crates must carry no everruns-platform edge of any kind (normal,
#    build, or dev), so `cargo tree` stays clean.
# 3. Provider-only crates must ship no everruns-platform subtree on their
#    normal (shipped) dependency edges.
# 4. Kernel crates must not re-declare the moved stored-record/provisioning
#    types (Agent, AgentVersion, AgentStatus, Harness, HarnessStatus,
#    BuiltInHarnessDefinition, BuiltInHarnessRole).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

FAILED=0

# 1. Kernel sources: no platform-crate references (src, tests, and examples —
#    even kernel tests must not need the stored records).
KERNEL_TREES=(
  crates/core
  crates/engine
  crates/provider
  crates/capability
)
if matches=$(grep -rnE 'everruns_platform(::|;)' "${KERNEL_TREES[@]}" --include='*.rs' 2>/dev/null); then
  echo "Kernel crates must not reference everruns_platform (EVE-877, EVE-881):"
  echo "$matches"
  FAILED=1
fi

# 1b. Kernel sources: the moved stored-record and provisioning types must not
#     be re-declared inside the kernel (EVE-877 agents, EVE-881 harnesses).
RECORD_TYPES='Agent|AgentVersion|AgentStatus|Harness|HarnessStatus|BuiltInHarnessDefinition|BuiltInHarnessRole'
if matches=$(grep -rnE "^[[:space:]]*pub (struct|enum) (${RECORD_TYPES})[[:space:]{<(]" \
  "${KERNEL_TREES[@]}" --include='*.rs' 2>/dev/null); then
  echo "Kernel crates must not declare stored platform record types (EVE-877, EVE-881):"
  echo "$matches"
  FAILED=1
fi

# 2. Kernel crates: no everruns-platform edge of any kind.
KERNEL_CRATES=(
  everruns-core
  everruns-engine
  everruns-provider
  everruns-capability
)
for crate in "${KERNEL_CRATES[@]}"; do
  tree=$(cargo tree -p "$crate" --edges normal,build,dev --prefix none 2>/dev/null)
  if echo "$tree" | grep -qE '^everruns-platform '; then
    echo "$crate must not depend on everruns-platform (any edge):"
    echo "$tree" | grep -E '^everruns-platform '
    FAILED=1
  fi
done

# 3. Provider-only crates: shipped dependency tree free of platform records.
PROVIDER_CRATES=(
  everruns-openai
  everruns-anthropic
  everruns-openrouter
  everruns-gemini
  everruns-bedrock
  everruns-mai
  everruns-fireworks
  everruns-meta
)
for crate in "${PROVIDER_CRATES[@]}"; do
  tree=$(cargo tree -p "$crate" --edges normal --prefix none 2>/dev/null)
  if echo "$tree" | grep -qE '^everruns-platform '; then
    echo "$crate must not ship everruns-platform in its normal dependency tree:"
    echo "$tree" | grep -E '^everruns-platform '
    FAILED=1
  fi
done

if [ "$FAILED" -ne 0 ]; then
  echo "Agent-record isolation guard failed. Stored Agent/AgentVersion and Harness records belong in crates/platform (EVE-877, EVE-881)."
  exit 1
fi

echo "Agent-record isolation guard passed: kernel crates consume AgentDefinition/HarnessDefinition only; platform records stay in crates/platform."
