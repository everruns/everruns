#!/usr/bin/env bash
# Drive Everruns' /mcp Tasks extension (2026-07-28) with the official MCP
# TypeScript client: `@modelcontextprotocol/client` plus
# `@modelcontextprotocol/ext-tasks`, pinned in scripts/mcp-tasks-interop/.
#
# The Tasks extension has no server conformance suite, so the reference
# client is the closest outside check of our wire format: ext-tasks validates
# every task response against the published schema. The domain test
# `official_ts_client_drives_mcp_tasks` serves /mcp on a real socket with a
# stand-in runner and runs `scripts/mcp-tasks-interop/driver.ts`.
# Needs Node 22+ and npm.
#
# `--setup-only` installs the pinned packages and stops: CI then sets
# MCP_TASKS_INTEROP_DIR on its domain-test step, so the run rides the
# `cargo test` invocation that already builds the domain suite.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dir="$root/scripts/mcp-tasks-interop"
(cd "$dir" && npm ci --silent --ignore-scripts)

if [ "${1:-}" = "--setup-only" ]; then
  exit 0
fi

cd "$root"
MCP_TASKS_INTEROP_DIR="$dir" cargo test -p everruns-server --test domain \
  -- official_ts_client_drives_mcp_tasks --nocapture --test-threads=1
