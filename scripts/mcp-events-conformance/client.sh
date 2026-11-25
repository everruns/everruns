#!/usr/bin/env bash
set -euo pipefail
export EVERRUNS_EVENTS_URL="$1"
exec "$EVERRUNS_EVENTS_TEST_BINARY" --exact mcp_events_conformance_test::client_driver --ignored --nocapture
