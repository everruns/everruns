#!/usr/bin/env bash
set -euo pipefail
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$PROJECT_ROOT/scripts/lib/common.sh"

caddy() { printf '%s\n' "$TEST_CADDY_VERSION"; }
for TEST_CADDY_VERSION in 'v2.9.1' 'v2.11.3' 'v2.11.4-rc.1' unknown; do
  if (require_command caddy "") >/dev/null 2>&1; then
    echo "FAIL: unsupported Caddy accepted: $TEST_CADDY_VERSION" >&2
    exit 1
  fi
done
for TEST_CADDY_VERSION in 'v2.11.4 h1:fixture' v2.11.5 v2.12.0 v3.0.0; do
  (require_command caddy "")
done
echo "Caddy version checks passed"
