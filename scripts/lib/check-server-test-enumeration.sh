#!/usr/bin/env bash
# Check that every server integration test file in crates/server/tests/ is
# either run by CI (referenced as `--test <name>` in .github/workflows/ci.yml)
# or explicitly listed in the ALLOWLIST below as intentionally not run.
#
# Why: CI hand-enumerates server integration tests by name. Without this guard
# a *new* test file under crates/server/tests/ is silently skipped until
# someone remembers to add a `--test` line — a real bug class (EVE-664). This
# makes the enumeration self-enforcing: add a test file and CI fails until you
# either wire it into a job or justify the exclusion here.
#
# Used by: scripts/lib/pre-push.sh, the `server-test-enumeration` CI job.
#
# Exits 0 on success, 1 on violation. Never silently skips.
#
# Usage: bash scripts/lib/check-server-test-enumeration.sh

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TESTS_DIR="$PROJECT_ROOT/crates/server/tests"
CI_FILE="$PROJECT_ROOT/.github/workflows/ci.yml"

# Test files that are NOT a runnable integration-test target on their own, or
# are intentionally excluded from CI. Each entry MUST carry a one-line reason.
# Files failing against a fresh migrated DB are parked here (explicit skip, not
# silent) with a tracking issue, until their failures are fixed and they are
# wired into a CI server-test step.
declare -A ALLOWLIST=(
  [test_harness]="shared TestServer helper module, not a standalone test target"
  [mcp_endpoint_test]="8 MCP tool/contract tests fail on a fresh DB — see EVE-668"
  [skills_integration_test]="12 tests fail on fixed-name collisions (isolation) — see EVE-668"
  [openapi_coverage_test]="13 plugin handlers missing from ApiDoc — see EVE-667"
  [openapi_descriptions_test]="doc/example coverage below MIN_*_PCT floors — see EVE-668"
  [reporting_integration_test]="backfill test omits NOT NULL owner_principal_id — see EVE-668"
)

if [ ! -d "$TESTS_DIR" ]; then
  echo "error: server tests directory not found: $TESTS_DIR"
  exit 1
fi
if [ ! -f "$CI_FILE" ]; then
  echo "error: CI workflow not found: $CI_FILE"
  exit 1
fi

shopt -s nullglob
test_files=("$TESTS_DIR"/*.rs)
shopt -u nullglob

if [ "${#test_files[@]}" -eq 0 ]; then
  echo "error: no test files found under $TESTS_DIR (expected many)"
  exit 1
fi

ci_contents="$(cat "$CI_FILE")"
missing=()

for path in "${test_files[@]}"; do
  name="$(basename "$path" .rs)"
  if [ -n "${ALLOWLIST[$name]+x}" ]; then
    continue
  fi
  # A test is "run" when CI invokes it as `--test <name>` for everruns-server.
  # Use a here-string, not `printf | grep -q`: grep -q exits on first match and
  # closes the pipe, so printf gets SIGPIPE; under `set -o pipefail` that turns a
  # match into a non-zero pipeline status (a false "missing"). Flaky by ci.yml
  # size vs the pipe buffer; a here-string avoids the pipe entirely.
  if grep -qE -- "--test[[:space:]]+${name}([[:space:]]|\$)" <<<"$ci_contents"; then
    continue
  fi
  missing+=("$name")
done

if [ "${#missing[@]}" -gt 0 ]; then
  echo "❌ server integration test files not run by CI and not allowlisted:"
  for name in "${missing[@]}"; do
    echo "   - crates/server/tests/${name}.rs"
  done
  echo ""
  echo "Fix one of:"
  echo "  1. Add '--test ${missing[0]}' to a server test step in .github/workflows/ci.yml, or"
  echo "  2. If the file is a shared helper or intentionally excluded, add it to"
  echo "     ALLOWLIST in scripts/lib/check-server-test-enumeration.sh with a reason."
  exit 1
fi

echo "✅ all ${#test_files[@]} server test files are run by CI or allowlisted"
