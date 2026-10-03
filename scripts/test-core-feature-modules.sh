#!/usr/bin/env bash
# Ownership moves may not turn kernel scans into a blanket source exclusion.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
source scripts/lib/core-feature-modules.sh
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
for pair in 'host host' 'engine engine' 'builtins builtins' 'mcp mcp' 'ag_ui ag-ui' 'a2a a2a'; do
  read -r module feature <<<"$pair"
  printf '#[cfg(feature = "%s")]\npub mod %s;\n' "$feature" "$module" >> "$work/lib.rs"
done
assert_core_feature_module_gates "$work/lib.rs"
for feature in host mcp ag-ui a2a; do
  sed "/cfg.*\"$feature\"/d" "$work/lib.rs" > "$work/ungated.rs"
  if assert_core_feature_module_gates "$work/ungated.rs" > "$work/out" 2>&1; then
    echo "FAIL: an ungated relocated module escaped the guard"
    exit 1
  fi
  grep -q 'must stay behind feature' "$work/out"
done
mkdir -p "$work/crates/core/src/host" "$work/crates/core/src/mcp" "$work/crates/core/src/ag_ui"
printf 'reqwest::Client;\n' > "$work/crates/core/src/kernel.rs"
printf 'reqwest::Client;\n' > "$work/crates/core/src/host/transport.rs"
printf 'reqwest::Client;\n' > "$work/crates/core/src/a2a.rs"
cd "$work"
files="$(core_kernel_source_files)"
grep -q 'kernel.rs' <<<"$files"
if grep -qE '/host/|/mcp/|/ag_ui/|/a2a.rs' <<<"$files"; then
  echo 'FAIL: feature-owned source leaked into the default kernel source list'
  exit 1
fi
if ! printf '%s\n' "$files" | xargs grep -q 'reqwest::'; then
  echo 'FAIL: kernel transport regression escaped the source scan'
  exit 1
fi
printf 'Core feature ownership guards passed: gates fail closed; kernel transport leaks remain visible.\n'
