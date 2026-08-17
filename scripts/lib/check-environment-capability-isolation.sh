#!/usr/bin/env bash
# Architecture guard (EVE-883): effectful environment capability
# implementations and their concrete transport/interpreter dependencies live
# in focused integration crates, not the neutral execution kernel.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

FAILED=0
fail() {
  printf '%s\n' "$1"
  FAILED=1
}

if [ -e crates/local/Cargo.toml ] || [ ! -e crates/everruns/src/local/mod.rs ]; then
  fail "Local embedded persistence must remain owned by the everruns local feature"
fi

FORBIDDEN_CORE_MODULES=(
  crates/core/src/capabilities/bashkit_shell
  crates/core/src/capabilities/file_system.rs
  crates/core/src/capabilities/lua.rs
  crates/core/src/capabilities/lua_code_mode.rs
  crates/core/src/capabilities/mcp.rs
  crates/core/src/capabilities/model_scout.rs
  crates/core/src/capabilities/openrouter_workspace.rs
  crates/core/src/capabilities/web_fetch
  crates/core/src/hook_dispatch.rs
)

for path in "${FORBIDDEN_CORE_MODULES[@]}"; do
  if [ -e "$path" ]; then
    fail "effectful implementation remains in everruns-core: $path"
  fi
done

MANIFEST_PATTERN='^[[:space:]]*(bashkit|fetchkit|mlua|reqwest|eventsource-stream)[[:space:]]*='
if matches=$(grep -nE "$MANIFEST_PATTERN" crates/core/Cargo.toml); then
  fail "concrete HTTP/shell/Lua dependencies remain in crates/core/Cargo.toml:"
  printf '%s\n' "$matches"
fi

SOURCE_PATTERN='(tokio::process|std::process::Stdio|everruns_mcp::|reqwest::|(^|[^[:alnum:]_])bashkit::|mlua::)'
if matches=$(grep -rnE "$SOURCE_PATTERN" crates/core/src --include='*.rs'); then
  fail "concrete process/HTTP/MCP/interpreter implementation references remain in core:"
  printf '%s\n' "$matches"
fi

assert_tree_excludes() {
  local package="$1"
  shift
  local tree
  tree=$(cargo tree -p "$package" -e normal --prefix none)
  local dependency
  for dependency in "$@"; do
    if grep -Eq "^${dependency}( |$)" <<<"$tree"; then
      fail "$package default dependency tree includes forbidden edge: $dependency"
    fi
  done
}

assert_tree_excludes everruns-core bashkit fetchkit mlua everruns-mcp reqwest
assert_tree_excludes everruns bashkit fetchkit mlua everruns-mcp reqwest

if [ "$FAILED" -ne 0 ]; then
  echo "Environment capability isolation guard failed."
  exit 1
fi

echo "Environment capability isolation guard passed: core stays effect-neutral and default Framework excludes opt-in execution stacks."
