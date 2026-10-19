#!/usr/bin/env bash
# A scan that never completed must not count as a clean architecture boundary.
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REAL_RG="$(command -v rg)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$WORK/scripts/lib" "$WORK/crates/durable" "$WORK/crates/worker/src" "$WORK/bin"
cp "$PROJECT_ROOT/scripts/lib/check-durable-isolation.sh" "$PROJECT_ROOT/scripts/lib/guard-cargo.sh" "$WORK/scripts/lib/"
printf '[package]\nname = "everruns-durable"\nversion = "0.48.0"\n[dependencies]\n' > "$WORK/crates/durable/Cargo.toml"

# An explicit PATH makes missing rg deterministic even on a developer machine.
for tool in awk bash dirname grep mktemp rm sed sort; do
  ln -s "$(command -v "$tool")" "$WORK/bin/$tool"
done
cat > "$WORK/bin/cargo" <<'CARGO'
#!/bin/sh
printf '%s\n' 'everruns-durable v0.48.0'
CARGO
chmod +x "$WORK/bin/cargo"

failures=0
run_case() {
  local name="$1" expected="$2" output status
  if output="$(PATH="$WORK/bin" "$BASH" "$WORK/scripts/lib/check-durable-isolation.sh" 2>&1)"; then
    status=0
  else
    status=$?
  fi
  if [ "$status" -ne "$expected" ]; then
    echo "FAIL $name: expected exit $expected, got $status"
    echo "$output"
    failures=$((failures + 1))
  elif [ "$expected" -ne 0 ] && [[ "$output" == *"Durable isolation guard passed:"* ]]; then
    echo "FAIL $name: incomplete or rejected scan printed success"
    failures=$((failures + 1))
  elif [ "$expected" -eq 2 ] && [[ "$output" != *"Guard could not run:"* ]]; then
    echo "FAIL $name: missing incomplete-check diagnostic"
    failures=$((failures + 1))
  elif [ "$expected" -eq 0 ] && [[ "$output" != *"Durable isolation guard passed:"* ]]; then
    echo "FAIL $name: clean scan did not report success"
    failures=$((failures + 1))
  else
    echo "ok   $name"
  fi
}

run_case 'missing scanner is an incomplete check' 2
cat > "$WORK/bin/rg" <<'RG'
#!/bin/sh
echo 'simulated scanner read failure' >&2
exit 2
RG
chmod +x "$WORK/bin/rg"
run_case 'scanner read error is an incomplete check' 2
printf '#!/bin/sh\nexit 1\n' > "$WORK/bin/rg"
run_case 'no matches pass' 0
printf '#!/bin/sh\nprintf "forbidden worker import\\n"\nexit 0\n' > "$WORK/bin/rg"
run_case 'a matching scanner result rejects the boundary' 1

rm "$WORK/bin/rg"
ln -s "$REAL_RG" "$WORK/bin/rg"
printf 'use everruns_core::Probe;\n' > "$WORK/crates/worker/src/probe.rs"
run_case 'real worker import is rejected' 1
rm "$WORK/crates/worker/src/probe.rs"
printf 'pub use everruns_core::Probe;\n' > "$WORK/crates/worker/src/lib.rs"
run_case 'crate-root re-export remains allowed' 0

if [ "$failures" -ne 0 ]; then
  echo "$failures durable isolation checks failed"
  exit 1
fi
echo 'Durable isolation scanner distinguishes a clean boundary from an incomplete check.'
