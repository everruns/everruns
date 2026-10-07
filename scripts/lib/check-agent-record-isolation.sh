#!/usr/bin/env bash
# Architecture guard: persistence/API records are server-only (EVE-1159).
# Contracts own neutral host-service SPIs. Capabilities own hosted orchestration.
# Kernel sources and dependency edges must consume only portable definitions,
# execution views, snapshots, and effect contracts. Provider consumers must stay
# independent of hosted capabilities; control-plane vocabulary is derived from
# the server owner by check_control_plane_records.py.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

# Keeps cargo's stderr, so "the guard could not run" never looks like
# "the guard found a violation". See guard-cargo.sh.
source "$SCRIPT_DIR/guard-cargo.sh"

# The server owns the full persistence/API aggregate vocabulary.
python3 scripts/check_control_plane_records.py
python3 scripts/test-control-plane-record-isolation.py

FAILED=0

# 1. Kernel sources: no platform-crate references (src, tests, and examples —
#    even kernel tests must not need the stored records).
KERNEL_TREES=(
  crates/core
  crates/contracts
)
if matches=$(grep -rnE 'everruns_(platform|capabilities|server)(::|;)' "${KERNEL_TREES[@]}" --include='*.rs' 2>/dev/null); then
  echo "Kernel crates must not reference everruns_platform (EVE-877, EVE-881, EVE-882, EVE-878):"
  echo "$matches"
  FAILED=1
fi

# Exact declaration exemptions preserve the backend-owned Workspace execution
# handle and stateless MCP OAuth protocol client; persisted rows stay forbidden.
# 1b. Kernel sources: the moved stored-record, provisioning, management, and
#     connector/OAuth/email infrastructure types must not be re-declared
#     inside the kernel (EVE-877 agents, EVE-881 harnesses, EVE-882 sessions,
#     EVE-878 eval/observer/feature-management records, EVE-879
#     connector/OAuth/email infrastructure).
RECORD_TYPES='Agent|AgentStatus|Harness|HarnessStatus|BuiltInHarnessDefinition|BuiltInHarnessRole|Session|SessionStatus|SessionSource|SessionActivity|SessionParticipant|SessionParticipantKind|SessionParticipantRole'
RECORD_TYPES="${RECORD_TYPES}|Eval|EvalCase|EvalRun|EvalCaseResult|EvalRunDataset|EvalTarget|Scorer"
RECORD_TYPES="${RECORD_TYPES}|Observer|ObserverMatch|LlmJudgeConfig|TraceScore"
RECORD_TYPES="${RECORD_TYPES}|FeatureFlags|FeatureFlagMap|FeatureFlagDefinition"
RECORD_TYPES="${RECORD_TYPES}|Connector|ConnectorPlugin|ConnectorRegistry|ConnectorRegistryBuilder|ConnectorType|ConnectorValidation"
RECORD_TYPES="${RECORD_TYPES}|EmailAddress|EmailMessage|EmailTag|EmailTemplate|MinimalEmailTemplate|BasicEmailTemplate|RenderedEmail|SentEmail|NoopEmailSender|DisabledEmailSender|ResendEmailSender|ResendEmailConfig|SystemEmailConfig"
RECORD_TYPES="${RECORD_TYPES}|OAuthClient|OAuthError|TokenSet|PkcePair|ProtectedResourceMetadata|AuthorizationServerMetadata|RegisteredClient|ClientRegistration"
RECORD_TYPES="${RECORD_TYPES}|Workspace|WorkspaceStatus"
RECORD_TYPES="${RECORD_TYPES}|SessionSqlDbStore|SessionSqlDbStoreExt|SessionSqlDbError|DatabaseInfo|SqlQueryResult|SqlExecuteResult|TableSchema|ColumnSchema"
RECORD_TYPES="${RECORD_TYPES}|SessionSandboxConfig|SessionSandboxInitConfig|SessionSandboxStatus|SessionSandboxStatusResponse|SessionSandboxInstance|SessionSandboxState|SessionSandboxExecRequest|SessionSandboxExecResponse|SessionSandboxReadFileResponse|SessionSandboxWriteFileResponse|SessionSandboxProvider|SessionSandboxProviderPlugin"
# `trait` is included so the sandbox provider SPI cannot reappear in the
# kernel: integration crates register providers against platform, and a turn
# reaches a sandbox only through the capability.
if matches=$(grep -rnE "^[[:space:]]*pub (struct|enum|trait) (${RECORD_TYPES})[[:space:]{<(]" \
  crates/core --include='*.rs' 2>/dev/null \
  | grep -v -E '^crates/core/src/host/workspace.rs:[0-9]+:pub struct Workspace \{' \
  | grep -v -E '^crates/core/src/mcp/oauth/protocol.rs:[0-9]+:pub (struct|enum) (OAuthClient|OAuthError|TokenSet|PkcePair|ProtectedResourceMetadata|AuthorizationServerMetadata|RegisteredClient|ClientRegistration)[[:space:]<{]'); then
  echo "Kernel crates must not declare stored platform record types or moved connector/OAuth/email infrastructure (EVE-877, EVE-881, EVE-882, EVE-878, EVE-879, EVE-880):"
  echo "$matches"
  FAILED=1
fi

# Contracts own extension SPIs, never persisted control-plane records.
if matches=$(grep -rnE "^[[:space:]]*pub (struct|enum) (Agent|AgentStatus|Harness|HarnessStatus|Session|SessionStatus|SessionParticipant|Organization|Principal|Workspace|Eval|Observer|FeatureFlags)[[:space:]{<(]" crates/contracts --include='*.rs' 2>/dev/null); then
  echo "Contracts must not declare control-plane records:"
  echo "$matches"
  FAILED=1
fi

# Runtime hosted capabilities and their store also consume portable execution
# views. Reject qualified record references, including test-only mocks: those
# otherwise conceal a downstream implementation's dependency on hosted records.
if matches=$(grep -rnE '(crate|everruns_platform|everruns_capabilities)::((agent|harness|session)::)?(Agent|AgentStatus|Harness|HarnessStatus|Session|SessionStatus|SessionParticipant|SessionParticipantKind|SessionParticipantRole)([^[:alnum:]_]|$)' \
  crates/capabilities/src/capabilities crates/capabilities/src/platform_store.rs crates/everruns/src/local/platform_store.rs --include='*.rs' 2>/dev/null); then
  echo "Hosted capabilities and PlatformStore must use portable runtime views:"
  echo "$matches"
  FAILED=1
fi

# 2. Kernel crates: no everruns-platform edge of any kind.
KERNEL_CRATES=(
  everruns-core
  everruns-contracts
)
for crate in "${KERNEL_CRATES[@]}"; do
  tree=$(guard_cargo_tree -p "$crate" --edges normal,build,dev --prefix none)
  if echo "$tree" | grep -qE '^everruns-(platform|capabilities|server) '; then
    echo "$crate must not depend on everruns-platform (any edge):"
    echo "$tree" | grep -E '^everruns-(platform|capabilities|server) '
    FAILED=1
  fi
done

# The reusable execution host is below the hosted product layer. Platform may
# implement host extension ports; host must never import or ship platform.
host_tree=$(guard_cargo_tree -p everruns-core --features host --edges normal --prefix none)
if echo "$host_tree" | grep -qE '^everruns-(platform|capabilities|server) '; then
  echo "everruns-core host must not depend on everruns-platform in its shipped graph:"
  echo "$host_tree" | grep -E '^everruns-(platform|capabilities|server) '
  FAILED=1
fi
if matches=$(grep -rnE 'everruns_(platform|capabilities)::|use[[:space:]]+everruns_(platform|capabilities)' crates/core/src/host --include='*.rs' 2>/dev/null); then
  echo "everruns-core host source must use neutral extension ports rather than platform types:"
  echo "$matches"
  FAILED=1
fi

# 3. Provider-only builds enable every vendor feature; the shipped dependency
#    tree remains free of platform records.
PROVIDER_CRATES=(
  everruns-drivers
)
for crate in "${PROVIDER_CRATES[@]}"; do
  tree=$(guard_cargo_tree -p "$crate" --all-features --edges normal --prefix none)
  if echo "$tree" | grep -qE '^everruns-(platform|capabilities|server) '; then
    echo "$crate must not ship everruns-platform in its normal dependency tree:"
    echo "$tree" | grep -E '^everruns-(platform|capabilities|server) '
    FAILED=1
  fi
done

# 5. Store-backed context orchestration must not drift back into the kernel.
CORE_CONTEXT_FILES=(
  crates/core/src/atoms/reason.rs
  crates/contracts/src/runtime/command_host.rs
  crates/contracts/src/runtime/dependency_blocker.rs
  crates/core/src/execution_snapshot.rs
  crates/core/src/runtime_context.rs
)
STORE_ORCHESTRATION_PATTERN='\b(AgentStore|HarnessStore|SessionStore|ProviderStore|DriverRegistry|StoreCommandHost|load_execution_snapshot|inspect_turn_context)\b'
if matches=$(grep -nE "$STORE_ORCHESTRATION_PATTERN" "${CORE_CONTEXT_FILES[@]}" 2>/dev/null); then
  echo "Core execution paths must not own store-backed context/provider orchestration (EVE-905):"
  echo "$matches"
  FAILED=1
fi

COMMAND_EFFECT_PATTERN='\b(ChatDriver|ProviderConfig|ProviderEndpoint|LlmCallConfig|ImageResolver|ResolvedImage)\b'
if matches=$(grep -nE "$COMMAND_EFFECT_PATTERN" crates/contracts/src/runtime/command_host.rs 2>/dev/null); then
  echo "Concrete provider/image command completion must live in everruns-core host (EVE-905):"
  echo "$matches"
  FAILED=1
fi

for host_owner in command_host execution_snapshot runtime_context; do
  if [ ! -f "crates/core/src/host/${host_owner}.rs" ]; then
    echo "everruns-core host must own ${host_owner}.rs orchestration (EVE-905)."
    FAILED=1
  fi
done

if [ "$FAILED" -ne 0 ]; then
  echo "Agent-record isolation guard failed. Stored records stay in server and store-backed execution orchestration stays in host (EVE-877, EVE-881, EVE-882, EVE-878, EVE-879, EVE-880, EVE-905)."
  exit 1
fi

echo "Agent-record isolation guard passed: kernel execution is value-first; server records and store-backed context/provider orchestration stay outside core."
