#!/usr/bin/env bash
# Guard against `#[ignore]` Rust tests.
#
# Why: an ignored test is one nothing runs. On 2026-10-25 main carried 37 of
# them; 29 had not run anywhere since they were written. A test that cannot run
# on every PR moves where something runs it: behind a cargo feature in a
# scheduled workflow that supplies its keys or services (and fails, not skips,
# without them), into a `harness = false` bench, or into an example. A process
# fixture is the test itself, started as its own child.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

if MATCHES="$(git grep -n -I -E '^[[:space:]]*#\[ignore' -- '*.rs')"; then
  echo "Ignored tests; run them in a scheduled job, a bench or an example instead:"
  printf '%s\n' "$MATCHES" | sed 's/^/  /'
  exit 1
fi
echo "No ignored tests."
