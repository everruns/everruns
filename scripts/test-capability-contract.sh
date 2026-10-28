#!/usr/bin/env bash
# A failed source scan must never certify the capability contract boundary.
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REAL_RG="$(command -v rg)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$WORK/scripts/lib" "$WORK/bin" "$WORK/crates/core/src/host" \
  "$WORK/crates/capabilities/src" "$WORK/crates/contracts/src/capability" \
  "$WORK/crates/integrations/src"
cp "$PROJECT_ROOT/scripts/lib/check-capability-contract.sh" "$WORK/scripts/lib/"
for tool in bash dirname grep; do
  ln -s "$(command -v "$tool")" "$WORK/bin/$tool"
done
ln -s "$REAL_RG" "$WORK/bin/rg"
printf 'pub trait IntoCapability {}\n' > "$WORK/crates/contracts/src/capability/mod.rs"
printf '// clean\n' > "$WORK/crates/core/src/lib.rs"
printf '// clean\n' > "$WORK/crates/capabilities/src/lib.rs"

run_case() {
  local name="$1" expected="$2" output status=0
  output="$(PATH="$WORK/bin" "$BASH" "$WORK/scripts/lib/check-capability-contract.sh" 2>&1)" || status=$?
  if [ "$status" -ne "$expected" ]; then
    printf 'FAIL %s: expected %s, got %s\n%s\n' "$name" "$expected" "$status" "$output"
    exit 1
  fi
  if [ "$expected" -eq 0 ] && [[ "$output" == *'No such file or directory'* ]]; then
    printf 'FAIL %s: clean scan emitted a missing-path error\n%s\n' "$name" "$output"
    exit 1
  fi
  if [ "$expected" -ne 0 ] && [[ "$output" == *'Capability contract guard passed:'* ]]; then
    echo "FAIL $name: rejected or incomplete scan printed success"
    exit 1
  fi
  if [ "$expected" -eq 2 ] && [[ "$output" != *'Guard could not run:'* ]]; then
    echo "FAIL $name: missing incomplete-check diagnostic"
    exit 1
  fi
  echo "ok   $name"
}

run_case 'current crate layout is clean without a root integrations directory' 0
printf 'pub struct CapabilityId;\n' > "$WORK/crates/core/src/probe.rs"
run_case 'competing capability identity is rejected' 1
printf 'impl CapabilityRef {}\n' > "$WORK/crates/core/src/probe.rs"
run_case 'competing inherent implementation is rejected' 1
printf 'fn validate_capability_config() {}\n' > "$WORK/crates/core/src/probe.rs"
run_case 'competing validation is rejected' 1
rm "$WORK/crates/core/src/probe.rs"
printf 'pub trait IntoCapability {}\n' > "$WORK/crates/integrations/src/probe.rs"
run_case 'competing integration contract is rejected' 1
rm "$WORK/crates/integrations/src/probe.rs"
rmdir "$WORK/crates/core/src/host"
run_case 'missing guarded source tree is incomplete' 2
mkdir "$WORK/crates/core/src/host"
rm "$WORK/bin/rg"
printf '#!/bin/sh\necho "simulated scan failure" >&2\nexit 2\n' > "$WORK/bin/rg"
chmod +x "$WORK/bin/rg"
run_case 'scanner read failure is incomplete' 2
rm "$WORK/bin/rg"
run_case 'missing scanner is incomplete' 2
