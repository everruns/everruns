#!/usr/bin/env bash
# Architecture guard (EVE-873): the capability identity/configuration contract
# is owned by crates/capability (`everruns-capability`). Core, host, and
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

GUARDED_TREES=(crates/core/src crates/host/src crates/platform/src)

FAILED=0

# 1. No competing capability identity/config/definition type declarations.
TYPE_PATTERN='(struct|enum|type)[[:space:]]+(CapabilityId|CapabilityRef|AgentCapabilityConfig|BuiltInCapabilityDefinition|CapabilitySpec)\b'
if matches=$(grep -rnE "$TYPE_PATTERN" "${GUARDED_TREES[@]}" --include='*.rs'); then
  echo "Capability identity/config types must live in crates/capability, not core/host/platform:"
  echo "$matches"
  FAILED=1
fi

# 2. No inherent impls on the neutral types (extension traits are fine).
IMPL_PATTERN='^impl[[:space:]]+(CapabilityId|CapabilityRef)[[:space:]]*\{'
if matches=$(grep -rnE "$IMPL_PATTERN" "${GUARDED_TREES[@]}" --include='*.rs'); then
  echo "Inherent impls on neutral capability types are not possible outside crates/capability; use an extension trait:"
  echo "$matches"
  FAILED=1
fi

# 3. No re-implementation of the shared ID/config validation rules.
VALIDATE_PATTERN='fn[[:space:]]+(validate_capability_id|validate_capability_config)\b'
if matches=$(grep -rnE "$VALIDATE_PATTERN" "${GUARDED_TREES[@]}" --include='*.rs'); then
  echo "Capability ID/config validation is owned by crates/capability; re-export it instead of redefining:"
  echo "$matches"
  FAILED=1
fi

# 4. `trait IntoCapability` exists once, in the contract crate.
if matches=$(grep -rnE 'trait[[:space:]]+IntoCapability\b' crates integrations --include='*.rs' | grep -v '^crates/capability/'); then
  echo "IntoCapability is defined by crates/capability only:"
  echo "$matches"
  FAILED=1
fi

if [ "$FAILED" -ne 0 ]; then
  echo "Capability contract guard failed. See knowledge/execution/capabilities.md (Neutral capability contract)."
  exit 1
fi

echo "Capability contract guard passed: no competing capability types in core/host/platform."
