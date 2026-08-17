#!/usr/bin/env bash
# Architecture guard (EVE-875): deterministic production simulation lives in
# crates/llmsim (`everruns-llmsim`); testing/demo helpers live in
# crates/test-support (`everruns-test-support`).
#
# 1. Production source trees must not import `everruns-test-support`.
# 2. `everruns-core` must carry no simulator, host, or test-support edge on
#    any edge kind (normal, build, or dev).
# 3. Provider-only crates must carry no simulator or test-support dependency
#    in their normal (shipped) dependency tree.
# 4. Product binaries and the Framework facade must depend directly on
#    `everruns-llmsim` and carry no normal test-support edge.
# 5. Public in-memory backend ownership stays split: application stores in
#    host, deterministic message/event fixtures in test-support, and no
#    conversation dual-write bridge.
# 6. Simulator implementation ownership stays in `everruns-llmsim`; the
#    test-support crate may only expose its documented 0.18 re-export bridge.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

FAILED=0

# 1. No test-support imports in production trees. Tests and examples
#    under tests/ and examples/ directories are exempt; in-source #[cfg(test)]
#    modules are intentionally NOT exempt.
GUARDED_TREES=(
  crates/server/src
  crates/worker/src
  crates/core/src
  crates/host/src
  crates/platform/src
  crates/local/src
  crates/cli/src
)
if matches=$(grep -rnE 'everruns_test_support(::|[^[:alnum:]_]|$)' "${GUARDED_TREES[@]}" --include='*.rs' 2>/dev/null); then
  echo "Production source trees must not import everruns-test-support:"
  echo "$matches"
  FAILED=1
fi

# 2. Core: no llmsim / host / test-support on ANY edge (including dev), so
#    `cargo tree -p everruns-core` stays clean.
CORE_TREE=$(cargo tree -p everruns-core --edges normal,build,dev --prefix none 2>/dev/null)
if grep -qE '^(llmsim|everruns-llmsim|everruns-host|everruns-test-support) ' <<<"$CORE_TREE"; then
  echo "everruns-core must not depend on llmsim, everruns-llmsim, everruns-host, or everruns-test-support (any edge):"
  grep -E '^(llmsim|everruns-llmsim|everruns-host|everruns-test-support) ' <<<"$CORE_TREE"
  FAILED=1
fi

# 3. Provider-only crates: shipped dependency tree free of simulation/fixtures.
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
  if grep -qE '^(llmsim|everruns-llmsim|everruns-test-support) ' <<<"$tree"; then
    echo "$crate must not ship simulator or test-support crates in its normal dependency tree:"
    grep -E '^(llmsim|everruns-llmsim|everruns-test-support) ' <<<"$tree"
    FAILED=1
  fi
done

# 5/6. Concrete public backend and simulator implementation ownership.
if [ ! -f crates/llmsim/src/lib.rs ] || ! grep -q 'pub struct LlmSimDriver' crates/llmsim/src/lib.rs; then
  echo "LlmSimDriver must be owned by everruns-llmsim."
  FAILED=1
fi

if [ -e crates/test-support/src/llmsim_driver.rs ] || [ -e crates/test-support/src/runtime_ext.rs ]; then
  echo "everruns-test-support must not own simulator implementation modules."
  FAILED=1
fi

if [ -e crates/core/src/in_memory.rs ] || grep -qE '^pub mod in_memory;' crates/core/src/lib.rs; then
  echo "everruns-core must not expose or house the public in_memory backend module."
  FAILED=1
fi

if matches=$(grep -rnE 'everruns_core::in_memory|everruns-core::in_memory' crates apps examples tests --include='*.rs' --include='*.md' 2>/dev/null); then
  echo "Legacy everruns-core in-memory backend imports are forbidden:"
  echo "$matches"
  FAILED=1
fi

for symbol in InMemoryAgentStore InMemoryHarnessStore InMemorySessionStore InMemoryProviderStore; do
  if ! grep -q "pub struct $symbol" crates/host/src/in_memory.rs; then
    echo "$symbol must be owned by everruns-host."
    FAILED=1
  fi
done

for symbol in InMemoryMessageRetriever InMemoryEventEmitter; do
  if ! grep -q "pub struct $symbol" crates/test-support/src/in_memory.rs; then
    echo "$symbol must be owned by everruns-test-support."
    FAILED=1
  fi
done

if matches=$(grep -rnE 'RuntimeMessageStore|PersistingEventEmitter|BridgingEventEmitter' crates/host/src crates/test-support/src --include='*.rs' 2>/dev/null); then
  echo "Writable message-store facades and conversation dual-write bridges are forbidden:"
  echo "$matches"
  FAILED=1
fi

# 4. Product binaries and the Framework facade use the focused simulator
#    directly and must never ship testing/demo helpers.
for crate in everruns-server everruns-worker everruns; do
  tree=$(cargo tree -p "$crate" --edges normal --prefix none 2>/dev/null)
  if grep -qE '^everruns-test-support ' <<<"$tree"; then
    echo "$crate must not ship everruns-test-support in its normal dependency tree:"
    grep -E '^everruns-test-support ' <<<"$tree"
    FAILED=1
  fi
  if ! grep -qE '^everruns-llmsim ' <<<"$tree"; then
    echo "$crate must depend directly or transitively on everruns-llmsim:"
    FAILED=1
  fi
done

if [ "$FAILED" -ne 0 ]; then
  echo "Simulation/test-support isolation guard failed."
  exit 1
fi

echo "Simulation/test-support isolation guard passed: production simulator and testing helpers are isolated."
