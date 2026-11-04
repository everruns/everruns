#!/usr/bin/env bash
# Architecture guard (EVE-873): the capability identity/configuration contract
# is owned by crates/contracts/src/capability (`everruns-contracts`). Core, host, and
# platform consume it — they must not grow competing capability ID, reference,
# config, or definition types, and must not re-implement the shared ID/config
# validation rules.
#
# Extension traits on the neutral types (e.g. `SkillCapabilityIdExt`) and thin
# serialization adapters (e.g. the OpenAPI doc shadow
# `AgentCapabilityConfigSchema`) are allowed; a second semantic model is not.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

GUARDED_TREES=(crates/core/src crates/core/src/host crates/capabilities/src)

# A missing scanner/tree or read failure is an incomplete check, never a clean
# boundary. Keep rg's status separate from its no-matches status (1).
if ! command -v rg >/dev/null 2>&1; then
  echo "Guard could not run: ripgrep (rg) is required for the capability source scan." >&2
  exit 2
fi
for tree in "${GUARDED_TREES[@]}"; do
  if [ ! -d "$tree" ]; then
    echo "Guard could not run: missing capability source tree $tree." >&2
    exit 2
  fi
done

scan() {
  local status
  if matches=$(rg -n --glob '*.rs' "$@"); then
    return 0
  else
    status=$?
    if [ "$status" -ne 1 ]; then
      echo "Guard could not run: capability source scan exited $status; the boundary was not checked." >&2
      exit 2
    fi
    return 1
  fi
}

FAILED=0

# 1. No competing capability identity/config/definition type declarations.
TYPE_PATTERN='(struct|enum|type)[[:space:]]+(CapabilityId|CapabilityRef|AgentCapabilityConfig|BuiltInCapabilityDefinition|CapabilitySpec)\b'
if scan "$TYPE_PATTERN" "${GUARDED_TREES[@]}"; then
  echo "Capability identity/config types must live in crates/contracts/src/capability, not core/host/platform:"
  echo "$matches"
  FAILED=1
fi

# 2. No inherent impls on the neutral types (extension traits are fine).
IMPL_PATTERN='^impl[[:space:]]+(CapabilityId|CapabilityRef)[[:space:]]*\{'
if scan "$IMPL_PATTERN" "${GUARDED_TREES[@]}"; then
  echo "Inherent impls on neutral capability types are not possible outside crates/contracts/src/capability; use an extension trait:"
  echo "$matches"
  FAILED=1
fi

# 3. No re-implementation of the shared ID/config validation rules.
VALIDATE_PATTERN='fn[[:space:]]+(validate_capability_id|validate_capability_config)\b'
if scan "$VALIDATE_PATTERN" "${GUARDED_TREES[@]}"; then
  echo "Capability ID/config validation is owned by crates/contracts/src/capability; re-export it instead of redefining:"
  echo "$matches"
  FAILED=1
fi

# 4. `trait IntoCapability` exists once, in the contract crate.
if scan 'trait[[:space:]]+IntoCapability\b' crates --glob '!crates/contracts/src/capability/**'; then
  echo "IntoCapability is defined by crates/contracts/src/capability only:"
  echo "$matches"
  FAILED=1
fi

if [ "$FAILED" -ne 0 ]; then
  echo "Capability contract guard failed. See knowledge/execution/capabilities.md (Neutral capability contract)."
  exit 1
fi

echo "Capability contract guard passed: no competing capability types in core/host/platform."
