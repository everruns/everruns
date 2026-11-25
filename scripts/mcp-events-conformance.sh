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
# Preserve only transport trust/proxy settings and the package cache.
runner_env=(env -i "PATH=$PATH" HOME=/tmp)
for setting in HTTP_PROXY HTTPS_PROXY ALL_PROXY NO_PROXY SSL_CERT_FILE SSL_CERT_DIR NODE_EXTRA_CA_CERTS npm_config_cache; do
  if [[ -v "$setting" ]]; then runner_env+=("$setting=${!setting}"); fi
done
(cd "$dir" && "${runner_env[@]}" npm ci --ignore-scripts --no-audit --no-fund && "${runner_env[@]}" npm run build)
if [ "${1:-}" = --setup-only ]; then exit 0; fi
cd "$root"
MCP_EVENTS_CONFORMANCE_DIR="$dir" cargo test -p everruns-server --test domain official_events_checks -- --nocapture --test-threads=1
