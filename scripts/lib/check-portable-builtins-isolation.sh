#!/usr/bin/env bash
# Architecture guard (EVE-884, EVE-901): backend-neutral first-party
# implementations live in the optional everruns-builtins bundle, provider
# behavior lives in focused integrations, and core retains only neutral
# contracts/registry algorithms without selecting a runtime or product preset.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

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
if matches=$(rg -n "$IMPLEMENTATION_PATTERN" crates/core/src --glob '*.rs'); then
  fail "portable policy implementation types remain in core source:"
  printf '%s\n' "$matches"
fi

if matches=$(rg -n '(with_builtins|runtime_builtins|with_builtins_for_grade)' crates/core/src --glob '*.rs'); then
  fail "runtime/product capability preset selection remains in core source:"
  printf '%s\n' "$matches"
fi

if [ -e crates/openui/Cargo.toml ] || [ -e crates/a2ui/Cargo.toml ]; then
  fail "OpenUI and A2UI catalogs belong to everruns-builtins, not standalone crates"
fi

if matches=$(rg -n 'everruns-(openui|a2ui)' Cargo.toml crates/{core,builtins}/Cargo.toml); then
  fail "standalone UI catalog dependencies remain in workspace manifests:"
  printf '%s\n' "$matches"
fi

BUILTINS_EFFECT_PATTERN='(reqwest::|sqlx::|mlua::|everruns_(host|platform|server|worker|mcp|http|integrations_)::|(^|[^[:alnum:]_])bashkit::|fetchkit::|std::(fs|net|process)::|tokio::(fs|net|process)::)'
if matches=$(rg -n "$BUILTINS_EFFECT_PATTERN" crates/builtins/src --glob '*.rs'); then
  fail "effectful host/platform/transport implementation leaked into everruns-builtins:"
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

BUILTINS_TREE=$(cargo tree -p everruns-builtins -e normal --depth 1 --prefix none)
assert_tree_excludes \
  "everruns-builtins direct normal dependency tree" \
  "$BUILTINS_TREE" \
  everruns-host everruns-platform everruns-server everruns-worker everruns-mcp \
  everruns-integrations-filesystem everruns-integrations-bashkit \
  everruns-integrations-web-fetch everruns-integrations-lua \
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
  everruns-builtins everruns-platform reqwest rustls hyper

HOST_MINIMAL_TREE=$(cargo tree -p everruns-host --no-default-features -e normal --prefix none)
assert_tree_excludes \
  "everruns-host --no-default-features normal dependency tree" \
  "$HOST_MINIMAL_TREE" \
  everruns-platform reqwest rustls hyper

SESSION_SERVICES_TREE=$(cargo tree -p everruns-session-services -e normal --prefix none)
assert_tree_excludes \
  "everruns-session-services normal dependency tree" \
  "$SESSION_SERVICES_TREE" \
  everruns-host everruns-platform everruns-server everruns-worker reqwest rustls hyper sqlx

if [ "$FAILED" -ne 0 ]; then
  echo "Portable built-ins isolation guard failed."
  exit 1
fi

echo "Portable built-ins isolation guard passed: core owns neutral contracts/algorithms, focused owners hold implementations and presets, and Framework can exclude the bundle."
