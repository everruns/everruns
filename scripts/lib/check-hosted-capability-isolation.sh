#!/usr/bin/env bash
# Architecture guard (EVE-885, extended by EVE-886): service-backed product
# capabilities live in everruns-platform. Core owns only neutral capability/
# tool/task/event/delegation contracts; portable policy implementations live in
# builtins.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

FAILED=0
HOSTED_MODULES=(
  a2a_delegation agent_handoff background_execution citation_retrieval
  citation_verification data_knowledge delegation_result knowledge_base
  knowledge_index memory model_scout monitors openrouter_workspace research
  session_schedule session_tasks subagents user_hooks
  # EVE-886: the session-service family — each needs a session store, session
  # key/value + secret storage, a SQL database, or a sandbox provider.
  session session_sandbox session_sql_database session_storage
)

for module in "${HOSTED_MODULES[@]}"; do
  if [ -e "crates/core/src/capabilities/$module.rs" ]; then
    echo "Hosted capability module remains in core: $module.rs"
    FAILED=1
  fi
done

for module in memory vector_store; do
  if [ -e "crates/core/src/$module.rs" ]; then
    echo "Hosted service/domain module remains in core: $module.rs"
    FAILED=1
  fi
done

HOSTED_DEFINITION_PATTERN='pub (struct|enum|trait) (KnowledgeStore|KnowledgeSearchHit|KnowledgeIndexSearch|VectorStore|Memory|MemoryConfig|MemoryStatus|SubagentCapability|AgentHandoffCapability|UserHooksCapability|SessionCapability|SessionStorageCapability|SessionSqlDatabaseCapability|SessionSandboxCapability)([[:space:]<{(]|$)'
if matches=$(grep -rnE "$HOSTED_DEFINITION_PATTERN" crates/core/src --include='*.rs' 2>/dev/null); then
  echo "Hosted capability or service definitions must not be declared by everruns-core:"
  echo "$matches"
  FAILED=1
fi

if grep -qE '^(a2a|a2a-client)[[:space:]]*=' crates/core/Cargo.toml; then
  echo "everruns-core must not declare outbound A2A implementation dependencies"
  FAILED=1
fi

CORE_TREE=$(cargo tree -p everruns-core --edges normal,build --prefix none 2>/dev/null)
if echo "$CORE_TREE" | grep -qE '^(a2a-lf|a2a-client-lf) '; then
  echo "everruns-core must not ship the outbound A2A implementation subtree:"
  echo "$CORE_TREE" | grep -E '^(a2a-lf|a2a-client-lf) ' | sort -u
  FAILED=1
fi

if [ "$FAILED" -ne 0 ]; then
  echo "Hosted capability isolation guard failed. Product implementations and services belong in crates/platform (EVE-885, EVE-886)."
  exit 1
fi

echo "Hosted capability isolation guard passed: hosted implementations stay in platform and core carries neutral contracts only."
