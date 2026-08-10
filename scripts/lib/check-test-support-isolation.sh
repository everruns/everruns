#!/usr/bin/env bash
# Architecture guard (EVE-875): deterministic simulation and demo fixtures
# live in crates/test-support (`everruns-test-support`). Production code must
# not ship them:
#
# 1. Production server/worker/core/host/platform/local/cli sources must not
#    import the demo fixture capabilities (fake AWS/CRM/financial/warehouse,
#    test math/weather, sample-data, noop). The worker may use the `sim`
#    surface (`llmsim_driver`) for its seeded LlmSim provider — but never the
#    fixture surface.
# 2. `everruns-core` must carry no llmsim or test-support edge at all, on any
#    edge kind (normal, build, or dev).
# 3. Provider-only crates must carry no llmsim / test-support / fake
#    capability dependency in their normal (shipped) dependency tree.
# 4. Product binaries (server/worker) may carry test-support only through the
#    `sim` surface (the worker's seeded LlmSim provider); the demo `fixtures`
#    feature must never be linked.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

FAILED=0

# 1. No fixture capability imports in production trees. Tests and examples
#    under tests/ and examples/ directories are exempt; in-source #[cfg(test)]
#    modules are intentionally NOT exempt — production files must not name the
#    fixtures at all.
GUARDED_TREES=(
  crates/server/src
  crates/worker/src
  crates/core/src
  crates/host/src
  crates/platform/src
  crates/local/src
  crates/cli/src
)
FIXTURE_PATTERN='everruns_test_support::(capabilities|FakeAwsCapability|FakeCrmCapability|FakeFinancialCapability|FakeWarehouseCapability|SampleDataCapability|TestMathCapability|TestWeatherCapability|NoopCapability)'
if matches=$(grep -rnE "$FIXTURE_PATTERN" "${GUARDED_TREES[@]}" --include='*.rs' 2>/dev/null); then
  echo "Demo fixture capabilities must not be referenced from production trees:"
  echo "$matches"
  FAILED=1
fi

# 2. Core: no llmsim / test-support on ANY edge (including dev), so
#    `cargo tree -p everruns-core` stays clean.
CORE_TREE=$(cargo tree -p everruns-core --edges normal,build,dev --prefix none 2>/dev/null)
if echo "$CORE_TREE" | grep -qE '^(llmsim|everruns-test-support) '; then
  echo "everruns-core must not depend on llmsim or everruns-test-support (any edge):"
  echo "$CORE_TREE" | grep -E '^(llmsim|everruns-test-support) '
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
  if echo "$tree" | grep -qE '^(llmsim|everruns-test-support) '; then
    echo "$crate must not ship llmsim or everruns-test-support in its normal dependency tree:"
    echo "$tree" | grep -E '^(llmsim|everruns-test-support) '
    FAILED=1
  fi
done

# 4. Product binaries (server embeds the worker, which registers the seeded
#    LlmSim provider): the test-support edge is allowed only with the `sim`
#    surface — the demo `fixtures` feature must stay unlinked.
for crate in everruns-server everruns-worker; do
  ts=$(cargo tree -p "$crate" --edges normal --prefix none -f '{p} {f}' 2>/dev/null | grep -E '^everruns-test-support ' | head -1 || true)
  if [ -n "$ts" ] && echo "$ts" | grep -qE '(^|[ ,])fixtures(,|$)'; then
    echo "$crate must not enable the everruns-test-support 'fixtures' feature:"
    echo "$ts"
    FAILED=1
  fi
done

if [ "$FAILED" -ne 0 ]; then
  echo "Test-support isolation guard failed. Demo/simulation code belongs in crates/test-support (EVE-875)."
  exit 1
fi

echo "Test-support isolation guard passed: no fixture capabilities or llmsim in production trees."
