#!/usr/bin/env bash
# Repository-owned shell tests live in scripts/test-*.sh.
# Workflow-specific shell harnesses under .github/scripts stay owned by their jobs.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_ROOT"

declare -a tests=()

if [ "$#" -gt 0 ]; then
  tests=("$@")
else
  while IFS= read -r test_script; do
    tests+=("$test_script")
  done < <(find "$PROJECT_ROOT/scripts" -maxdepth 1 -type f -name 'test-*.sh' | LC_ALL=C sort)
fi

if [ "${#tests[@]}" -eq 0 ]; then
  echo "No repo shell tests found."
  exit 0
fi

failures=0

echo "Running ${#tests[@]} repo shell test(s)..."

for i in "${!tests[@]}"; do
  test_script="${tests[$i]}"

  if [ ! -f "$test_script" ] && [ -f "$PROJECT_ROOT/$test_script" ]; then
    test_script="$PROJECT_ROOT/$test_script"
  fi

  if [ ! -f "$test_script" ]; then
    echo "[$((i + 1))/${#tests[@]}] FAIL: missing test script '$test_script'" >&2
    failures=$((failures + 1))
    continue
  fi

  rel_path="${test_script#$PROJECT_ROOT/}"
  echo "[$((i + 1))/${#tests[@]}] $rel_path"

  if bash "$test_script"; then
    echo "PASS: $rel_path"
  else
    echo "FAIL: $rel_path" >&2
    failures=$((failures + 1))
  fi

  echo ""
done

if [ "$failures" -ne 0 ]; then
  echo "$failures repo shell test(s) failed." >&2
  exit 1
fi

echo "All repo shell tests passed."
