#!/usr/bin/env bash
set -euo pipefail
export EVERRUNS_EVENTS_URL="$1"
cd "$(dirname "${BASH_SOURCE[0]}")/../../crates/server"
exec "$EVERRUNS_EVENTS_TEST_BINARY" --exact mcp_events_conformance_test::client_driver --nocapture
