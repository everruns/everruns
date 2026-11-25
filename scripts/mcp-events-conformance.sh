#!/usr/bin/env bash
# Unreleased upstream Events scenarios, reviewed at this immutable revision.
# Base MCP conformance remains on the released npm package in mcp-conformance/.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dir="$root/.local/mcp-events-conformance/upstream"
revision=c8625de3118c2d5dc0561696cddeb0baaf124f2c
if [ ! -d "$dir/.git" ]; then
  mkdir -p "$dir"
  git -C "$dir" init --quiet
  git -C "$dir" remote add origin https://github.com/modelcontextprotocol/conformance.git
fi
git -C "$dir" fetch --depth=1 origin "$revision"
git -C "$dir" checkout --detach "$revision"
test "$(git -C "$dir" rev-parse HEAD)" = "$revision"
test -z "$(git -C "$dir" status --porcelain --untracked-files=no)"
# No lifecycle hooks, developer credentials, or inherited app secrets.
(cd "$dir" && env -i PATH="$PATH" HOME=/tmp npm ci --ignore-scripts --no-audit --no-fund && env -i PATH="$PATH" HOME=/tmp npm run build)
if [ "${1:-}" = --setup-only ]; then exit 0; fi
cd "$root"
MCP_EVENTS_CONFORMANCE_DIR="$dir" cargo test -p everruns-server --test domain official_events_checks -- --ignored --nocapture --test-threads=1
