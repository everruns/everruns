#!/usr/bin/env bash
# Architecture guard (EVE-884): portable policy implementations live in the
# optional everruns-builtins bundle, while core retains only neutral execution
# contracts and Framework can compile without the bundle.

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
)

for module in "${FORBIDDEN_CORE_MODULES[@]}"; do
  path="crates/core/src/capabilities/${module}.rs"
  if [ -e "$path" ]; then
    fail "portable policy implementation remains in everruns-core: $path"
  fi
done

IMPLEMENTATION_PATTERN='pub struct (AgentInstructionsCapability|AutoToolSearchCapability|CompactionCapability|CurrentTimeCapability|ErrorDisclosureCapability|GuardrailsCapability|MessageMetadataCapability|PromptCachingCapability|ToolCallRepairCapability|ToolSearchCapability|UsageLimitAutoContinueCapability)'
if matches=$(rg -n "$IMPLEMENTATION_PATTERN" crates/core/src --glob '*.rs'); then
  fail "portable policy implementation types remain in core source:"
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
  everruns-host everruns-platform everruns-server everruns-worker everruns-mcp everruns-http \
  everruns-integrations-filesystem everruns-integrations-bashkit \
  everruns-integrations-web-fetch everruns-integrations-lua \
  reqwest sqlx bashkit fetchkit mlua

FRAMEWORK_MINIMAL_TREE=$(cargo tree -p everruns --no-default-features -e normal --prefix none)
assert_tree_excludes \
  "everruns --no-default-features normal dependency tree" \
  "$FRAMEWORK_MINIMAL_TREE" \
  everruns-builtins

if [ "$FAILED" -ne 0 ]; then
  echo "Portable built-ins isolation guard failed."
  exit 1
fi

echo "Portable built-ins isolation guard passed: core owns contracts, the bundle stays effect-neutral, and Framework can exclude it."
