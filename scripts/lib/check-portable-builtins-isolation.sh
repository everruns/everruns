#!/usr/bin/env bash
# Architecture guard (EVE-884, EVE-901): backend-neutral first-party
# implementations live in the optional core builtins module, provider
# behavior lives in focused integrations, and core retains only neutral
# contracts/registry algorithms without selecting a runtime or product preset.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"
source "$SCRIPT_DIR/core-feature-modules.sh"

FAILED=0
fail() {
  printf '%s\n' "$1"
  FAILED=1
}

FORBIDDEN_CORE_MODULES=(
  agent_instructions
  auto_tool_search
  btw
  budgeting
  claude_tool_search
  compaction
  current_time
  error_disclosure
  guardrails
  loop_detection
  message_metadata
  openai_tool_search
  parallel_tool_calls
  progress_guard
  prompt_caching
  prompt_canary_guardrail
  self_budget
  stateless_todo_list
  system_commands
  tool_call_repair
  tool_output_distillation
  tool_output_persistence
  tool_search
  usage_limit_auto_continue
  human_intent
  infinity_context
  skills
  skills_scoped
  attach_skill
  tool_approval
  openui
  a2ui
  openrouter_server_tools
)

for module in "${FORBIDDEN_CORE_MODULES[@]}"; do
  path="crates/core/src/capabilities/${module}.rs"
  if [ -e "$path" ]; then
    fail "portable policy implementation remains in everruns-core: $path"
  fi
done

IMPLEMENTATION_PATTERN='pub struct (AgentInstructionsCapability|AutoToolSearchCapability|CompactionCapability|CurrentTimeCapability|ErrorDisclosureCapability|GuardrailsCapability|MessageMetadataCapability|PromptCachingCapability|ToolCallRepairCapability|ToolSearchCapability|UsageLimitAutoContinueCapability|HumanIntentCapability|InfinityContextCapability|SkillsCapability|ScopedSkillsCapability|AttachSkillCapability|ToolApprovalCapability|OpenUiCapability|A2UiCapability|OpenRouterServerToolsCapability)'
if matches=$(core_kernel_source_files | grep -v '^crates/core/src/builtins/' | xargs grep -nE "$IMPLEMENTATION_PATTERN"); then
  fail "portable policy implementation types remain in core source:"
  printf '%s\n' "$matches"
fi

if matches=$(core_kernel_source_files | grep -v '^crates/core/src/builtins/' | xargs grep -nE '(with_builtins|runtime_builtins|with_builtins_for_grade)'); then
  fail "runtime/product capability preset selection remains in core source:"
  printf '%s\n' "$matches"
fi

if [ -e crates/openui/Cargo.toml ] || [ -e crates/a2ui/Cargo.toml ]; then
  fail "OpenUI and A2UI catalogs belong to core builtins, not standalone crates"
fi

if matches=$(rg -n 'everruns-(openui|a2ui)' Cargo.toml crates/core/Cargo.toml); then
  fail "standalone UI catalog dependencies remain in workspace manifests:"
  printf '%s\n' "$matches"
fi

BUILTINS_EFFECT_PATTERN='(reqwest::|sqlx::|mlua::|everruns_(host|platform|server|worker|mcp|http|integrations_)::|(^|[^[:alnum:]_])bashkit::|fetchkit::|std::(fs|net|process)::|tokio::(fs|net|process)::)'
if matches=$(rg -n "$BUILTINS_EFFECT_PATTERN" crates/core/src/builtins --glob '*.rs'); then
  fail "effectful host/platform/transport implementation leaked into core builtins:"
  printf '%s\n' "$matches"
fi

assert_tree_excludes() {
  local command_description="$1"
  local tree="$2"
  shift 2
  local dependency
  for dependency in "$@"; do
    if rg -q "^${dependency}( |$)" <<<"$tree"; then
      fail "$command_description includes forbidden dependency: $dependency"
    fi
  done
}

BUILTINS_TREE=$(cargo tree -p everruns-core --features builtins -e normal --prefix none)
assert_tree_excludes \
  "everruns-core builtins normal dependency tree" \
  "$BUILTINS_TREE" \
  everruns-host everruns-capabilities everruns-server everruns-worker everruns-mcp \
  everruns-integrations everruns-integrations-experimental \
  reqwest sqlx bashkit fetchkit mlua

CORE_ALL_FEATURES_TREE=$(cargo tree -p everruns-core --all-features -e normal --prefix none)
assert_tree_excludes \
  "everruns-core all-feature normal dependency tree" \
  "$CORE_ALL_FEATURES_TREE" \
  everruns-builtins

FRAMEWORK_MINIMAL_TREE=$(cargo tree -p everruns --no-default-features -e normal --prefix none)
assert_tree_excludes \
  "everruns --no-default-features normal dependency tree" \
  "$FRAMEWORK_MINIMAL_TREE" \
  everruns-builtins everruns-capabilities reqwest rustls hyper

# The typed-judgment integration has two halves: a client + framework surface an
# embedder can carry on its own, and a `hosted` half that registers the connector
# into the platform plugin system. Enabling the facade's `typesafe` feature must
# reach only the first, or `everruns` stops being embeddable without the control
# plane. `inventory` is not listed: everruns-core carries it unconditionally.
FRAMEWORK_TYPESAFE_TREE=$(cargo tree -p everruns --no-default-features --features typesafe -e normal --prefix none)
assert_tree_excludes \
  "everruns --no-default-features --features typesafe normal dependency tree" \
  "$FRAMEWORK_TYPESAFE_TREE" \
  everruns-builtins everruns-capabilities

HOST_MINIMAL_TREE=$(cargo tree -p everruns-core --no-default-features --features host -e normal --prefix none)
assert_tree_excludes \
  "everruns-core --no-default-features --features host normal dependency tree" \
  "$HOST_MINIMAL_TREE" \
  everruns-capabilities reqwest rustls hyper

if [ -e crates/session-services/Cargo.toml ]; then
  fail "session mutation/storage services belong to core host, not a standalone crate"
fi

for module in session_mutator.rs capabilities/session.rs capabilities/session_storage.rs; do
  if [ ! -e "crates/core/src/host/session_services/$module" ]; then
    fail "core host is missing session service module: $module"
  fi
done

if [ "$FAILED" -ne 0 ]; then
  echo "Portable built-ins isolation guard failed."
  exit 1
fi

echo "Portable built-ins isolation guard passed: core owns neutral contracts/algorithms, focused owners hold implementations and presets, and Framework can exclude the bundle."
