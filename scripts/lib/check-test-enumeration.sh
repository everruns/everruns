#!/usr/bin/env bash
# Check that driver libraries and crate integration targets have workflow
# invocations, or an explicit allowlist reason. Ignored cases remain opt-in.
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
# Out of scope: crates under a `tests/fixtures/` path. Those are downstream
# consumer fixtures in their own workspace, covered by the shell CI job.
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
# Join shell continuations before matching packages and targets. Ignore comments:
# mentioning an omitted command in an explanation does not execute it.
workflows="$(awk '
  /^[[:space:]]*#/ { next }
  /\\$/ { sub(/\\$/, ""); printf "%s ", $0; next }
  { print }
' "$WORKFLOW_DIR"/*.yml)"

violations=()
checked=0
drivers=0

while IFS= read -r manifest; do
  # Fixture crates under a crate's tests/fixtures/ form their own cargo
  # workspace and are never workspace members, so no `cargo test -p <package>`
  # can reach them. They are not unrun tests: the `shell` CI job runs them
  # through scripts/test-external-consumer.sh, which owns their coverage.
  case "$manifest" in
    */tests/fixtures/*) continue ;;
  esac

  manifest="$PROJECT_ROOT/$manifest"
  crate_dir="$(dirname "$manifest")"
  tests_dir="$crate_dir/tests"

  package="$(sed -n 's/^name[[:space:]]*=[[:space:]]*"\(.*\)"/\1/p' "$manifest" | head -1)"
  [ -n "$package" ] || continue

  invocations="$(grep -E -- "cargo test .*-p[[:space:]]+${package}([[:space:]]|$)" <<<"$workflows" || true)"

  # A library-only driver (e.g. Bedrock) must not disappear from coverage just
  # because it has no tests/ directory. Named integration/doc runs don't run it.
  case "$crate_dir" in
    "$PROJECT_ROOT/crates/drivers/"*)
      drivers=$((drivers + 1))
      if ! grep -Ev -- '(--test[[:space:]]|--doc|--ignored)' <<<"$invocations" | grep -q 'cargo test'; then
        violations+=("${package}: driver library has no unit-test invocation")
      fi
      ;;
  esac

  # Unit tests written inline in a bin target (`#[cfg(test)]` in src/main.rs or
  # src/bin/*.rs) are a third target kind the rules above never considered. A
  # crate whose invocations all filter to `--test <name>` runs its integration
  # targets and nothing else, so those unit tests never execute. crates/cli
  # carried `contract_golden` — the guard on the shipped command line — in
  # src/main.rs while CI ran three named --test targets beside it, so the golden
  # drifted across four merged feature PRs with CI green the whole time.
  if [ -n "$invocations" ]; then
    bin_unit_tests=""
    shopt -s nullglob
    for bin_src in "$crate_dir/src/main.rs" "$crate_dir"/src/bin/*.rs; do
      [ -f "$bin_src" ] || continue
      if grep -q '#\[cfg(test)\]' "$bin_src"; then
        bin_unit_tests=1
        break
      fi
    done
    shopt -u nullglob

    if [ -n "$bin_unit_tests" ]; then
      checked=$((checked + 1))
      # Covered when some invocation reaches bin targets: it names them
      # (`--bins`/`--bin <name>`), or it applies no target filter at all.
      if ! grep -qE -- '--bins?([[:space:]]|$)' <<<"$invocations" &&
        ! grep -qvE -- '(--lib|--bins?[[:space:]]|--test[[:space:]]|--doc|--ignored)' <<<"$invocations"; then
        violations+=("${package}: unit tests in a bin target never run — every 'cargo test -p ${package}' filters to --lib/--test, so add --bins")
      fi
    fi
  fi

  [ -d "$tests_dir" ] || continue
  shopt -s nullglob
  test_files=("$tests_dir"/*.rs)
  # A directory-form integration target (tests/<name>/main.rs) is one test
  # binary named <name> that merges several modules into a single link,
  # e.g. tests/domain/main.rs for everruns-server. Cargo names the target
  # after the directory, not "main", so include it here rather than missing
  # it because the glob above is non-recursive.
  test_files+=("$tests_dir"/*/main.rs)
  shopt -u nullglob
  [ "${#test_files[@]}" -gt 0 ] || continue

  # A test file's target name is its stem, except a directory-form target
  # (tests/<name>/main.rs), whose target name is the directory name.
  target_name() {
    if [ "$(basename "$1")" = "main.rs" ]; then
      basename "$(dirname "$1")"
    else
      basename "$1" .rs
    fi
  }

  if [ -z "$invocations" ]; then
    for path in "${test_files[@]}"; do
      name="$(target_name "$path")"
      allowlist_reason "${package}:${name}" >/dev/null && continue
      violations+=("${package}: ${path#"$PROJECT_ROOT"/} — no workflow runs 'cargo test -p ${package}'")
      checked=$((checked + 1))
    done
    continue
  fi

  # A run that filters neither to --lib nor to named --test targets executes
  # the crate's whole suite, integration tests included.
  if grep -qvE -- "(--lib|--test[[:space:]]|--doc|--ignored)" <<<"$invocations"; then
    checked=$((checked + ${#test_files[@]}))
    continue
  fi

  for path in "${test_files[@]}"; do
    name="$(target_name "$path")"
    checked=$((checked + 1))
    allowlist_reason "${package}:${name}" >/dev/null && continue
    grep -qE -- "--test[[:space:]]+${name}([[:space:]]|$)" <<<"$invocations" && continue
    violations+=("${package}: ${path#"$PROJECT_ROOT"/} — not run as '--test ${name}'")
  done
done < <(git -C "$PROJECT_ROOT" ls-files -- 'crates/**/Cargo.toml' 'integrations/**/Cargo.toml' | sort)

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

echo "✅ all ${drivers} driver libraries and ${checked} crate test targets are covered by a workflow or allowlisted (ignored cases remain opt-in)"
