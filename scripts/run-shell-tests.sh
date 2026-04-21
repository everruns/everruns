#!/usr/bin/env bash
# Repository-owned shell tests live in scripts/test-*.sh.
# Workflow-specific shell harnesses under .github/scripts stay owned by their jobs.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_ROOT"

declare -a tests=()

resolve_repo_shell_test() {
  local candidate="$1"
  local resolved_dir resolved_path

  if [ ! -f "$candidate" ] && [ -f "$PROJECT_ROOT/$candidate" ]; then
    candidate="$PROJECT_ROOT/$candidate"
  fi

  if [ ! -f "$candidate" ]; then
    echo "missing test script '$candidate'" >&2
    return 1
  fi

  resolved_dir="$(cd "$(dirname "$candidate")" && pwd -P)"
  resolved_path="$resolved_dir/$(basename "$candidate")"

  case "$resolved_path" in
    "$PROJECT_ROOT"/scripts/test-*.sh)
      printf '%s\n' "$resolved_path"
      ;;
    *)
      echo "test script must stay under scripts/test-*.sh: '$candidate'" >&2
      return 1
      ;;
  esac
}

if [ "$#" -gt 0 ]; then
  for candidate in "$@"; do
    tests+=("$(resolve_repo_shell_test "$candidate")")
  done
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
