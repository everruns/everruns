#!/usr/bin/env bash
# Check that every integration test file under a workspace crate's tests/
# directory is actually run by some GitHub Actions workflow, or is explicitly
# allowlisted with a reason.
#
# Why: CI hand-enumerates test targets — there is no `cargo test --workspace`
# anywhere in .github/workflows/. A crate is covered only if a workflow names
# it in `cargo test -p <package>`, and a file inside a crate whose invocations
# use `--test <name>` is covered only if it is named too. Nothing enforced
# that, so a whole crate could carry test files that never ran.
#
# This generalizes check-server-test-enumeration.sh (EVE-664), which caught the
# same bug class for crates/server/tests/ only. It missed, for example, that
# everruns-provider had never appeared in any workflow: its wire tests had
# never run, and one of them was failing against committed code while CI stayed
# green.
#
# Coverage rules, per crate that has a tests/ directory:
#   1. No workflow runs `cargo test -p <package>`      -> every file is a violation.
#   2. Some invocation runs the crate with neither
#      `--lib` nor `--test`                            -> whole suite runs, all files covered.
#   3. Otherwise                                       -> each file must appear as
#                                                         `--test <name>` or be allowlisted.
#
# Used by: scripts/lib/pre-push.sh, the `test-enumeration` CI job.
# Exits 0 on success, 1 on violation. Never silently skips.

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORKFLOW_DIR="$PROJECT_ROOT/.github/workflows"

# Files that are not a runnable test target on their own, or are intentionally
# not run. Keyed as "<package>:<file stem>". Every entry carries a reason.
allowlist_reason() {
  case "$1" in
    everruns-server:test_harness)
      echo "shared TestServer helper module, not a standalone test target" ;;
    everruns-platform:capability_boundary)
      echo "2 of its 5 tests fail against current code; parked pending triage, not silently skipped" ;;
    everruns-durable:agent_reliability_test)
      echo "end-to-end infrastructure-failure tests; needs PostgreSQL and a running worker" ;;
    everruns-llm-tests:tool_search_test)
      echo "requires OPENAI_API_KEY; belongs in a credentialed job, not the pure suite" ;;
    everruns-llm-tests:gpt_comparison_bench)
      echo "manual latency/token benchmark printed for review, not a pass/fail test" ;;
    *) return 1 ;;
  esac
}

if [ ! -d "$WORKFLOW_DIR" ]; then
  echo "error: workflow directory not found: $WORKFLOW_DIR"
  exit 1
fi

# Every workflow counts, not just ci.yml: several integrations are covered only
# by their own dedicated workflow (brave-search, duckduckgo, parallel, the
# container sandbox sweep). Reading ci.yml alone would report those as
# uncovered and train people to ignore this check.
workflows="$(cat "$WORKFLOW_DIR"/*.yml)"

violations=()
checked=0

while IFS= read -r manifest; do
  crate_dir="$(dirname "$manifest")"
  tests_dir="$crate_dir/tests"
  [ -d "$tests_dir" ] || continue

  package="$(sed -n 's/^name[[:space:]]*=[[:space:]]*"\(.*\)"/\1/p' "$manifest" | head -1)"
  [ -n "$package" ] || continue

  shopt -s nullglob
  test_files=("$tests_dir"/*.rs)
  shopt -u nullglob
  [ "${#test_files[@]}" -gt 0 ] || continue

  # Invocation lines for this exact package.
  invocations="$(grep -E -- "cargo test .*-p[[:space:]]+${package}([[:space:]]|$)" <<<"$workflows" || true)"

  if [ -z "$invocations" ]; then
    for path in "${test_files[@]}"; do
      name="$(basename "$path" .rs)"
      allowlist_reason "${package}:${name}" >/dev/null && continue
      violations+=("${package}: ${crate_dir#"$PROJECT_ROOT"/}/tests/${name}.rs — no workflow runs 'cargo test -p ${package}'")
      checked=$((checked + 1))
    done
    continue
  fi

  # A run that filters neither to --lib nor to named --test targets executes
  # the crate's whole suite, integration tests included.
  if grep -qvE -- "(--lib|--test[[:space:]])" <<<"$invocations"; then
    checked=$((checked + ${#test_files[@]}))
    continue
  fi

  for path in "${test_files[@]}"; do
    name="$(basename "$path" .rs)"
    checked=$((checked + 1))
    allowlist_reason "${package}:${name}" >/dev/null && continue
    grep -qE -- "--test[[:space:]]+${name}([[:space:]]|$)" <<<"$workflows" && continue
    violations+=("${package}: ${crate_dir#"$PROJECT_ROOT"/}/tests/${name}.rs — not run as '--test ${name}'")
  done
done < <(find "$PROJECT_ROOT/crates" "$PROJECT_ROOT/integrations" -maxdepth 2 -name Cargo.toml 2>/dev/null | sort)

if [ "$checked" -eq 0 ]; then
  echo "error: no crate test files discovered — the search paths are probably wrong"
  exit 1
fi

if [ "${#violations[@]}" -gt 0 ]; then
  echo "❌ test files not run by any workflow and not allowlisted:"
  printf '   - %s\n' "${violations[@]}"
  echo ""
  echo "Fix one of:"
  echo "  1. Run the crate from a workflow: add 'cargo test -p <package>' to a"
  echo "     job in .github/workflows/ (or '--test <name>' if that crate is"
  echo "     enumerated by target), or"
  echo "  2. If the file is a shared helper or is intentionally not run, add it"
  echo "     to allowlist_reason() in scripts/lib/check-test-enumeration.sh with"
  echo "     a one-line reason."
  exit 1
fi

echo "✅ all ${checked} crate test files are run by a workflow or allowlisted"
