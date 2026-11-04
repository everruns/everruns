#!/usr/bin/env bash
# Real workerd, WASM drivers and host fetch; scripted model server, no API key.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."
worker-build --dev --no-opt tests/wasm-providers --locked
npm ci --prefix tests/wasm-providers
node tests/wasm-providers/test.mjs
