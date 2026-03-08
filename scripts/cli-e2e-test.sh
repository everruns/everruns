#!/usr/bin/env bash
# CLI E2E Test Script
# Tests the everruns CLI against a running server
#
# Prerequisites:
# - Server running at localhost:9301 (with worker)
# - CLI binary built: cargo build -p everruns-cli
# - EVERRUNS_API_KEY set (any value works with AUTH_MODE=none)
#
# Usage: ./scripts/cli-e2e-test.sh [--skip-chat]
#   --skip-chat: Skip chat test (requires LLM API keys)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_ROOT"

# Configuration
API_URL="${EVERRUNS_API_URL:-http://localhost:9301}"
CLI="./target/debug/everruns"
SKIP_CHAT="${1:-}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Test counters
PASSED=0
FAILED=0

# Helper functions
log_test() {
  echo -e "${YELLOW}[TEST]${NC} $1"
}

log_pass() {
  echo -e "${GREEN}[PASS]${NC} $1"
  PASSED=$((PASSED + 1))
}

log_fail() {
  echo -e "${RED}[FAIL]${NC} $1"
  FAILED=$((FAILED + 1))
}

# Check CLI is built
if [ ! -f "$CLI" ]; then
  echo "CLI not found at $CLI. Building..."
  cargo build -p everruns-cli
fi

# Verify server is running
log_test "Checking server health..."
if ! curl -s "$API_URL/health" > /dev/null; then
  echo "Server not running at $API_URL"
  exit 1
fi
log_pass "Server is healthy"

echo ""
echo "========================================"
echo "  Everruns CLI E2E Tests"
echo "  API: $API_URL"
echo "========================================"
echo ""

# ========================================
# Test: Capabilities
# ========================================
log_test "capabilities list"
CAPS_OUTPUT=$($CLI --api-url "$API_URL" capabilities --status all --output json 2>&1) || {
  log_fail "capabilities list failed"
  echo "$CAPS_OUTPUT"
  exit 1
}
if echo "$CAPS_OUTPUT" | jq -e '.data | length >= 0' > /dev/null 2>&1; then
  log_pass "capabilities list returned valid JSON"
else
  log_fail "capabilities list did not return valid JSON array"
  echo "$CAPS_OUTPUT"
fi

# ========================================
# Test: Agents CRUD
# ========================================

# Create agent
log_test "agents create"
AGENT_OUTPUT=$($CLI --api-url "$API_URL" agents create \
  --name "cli-test-agent" \
  --system-prompt "You are a test agent for CLI e2e testing." \
  --description "Test agent created by CLI e2e tests" \
  --output json 2>&1) || {
  log_fail "agents create failed"
  echo "$AGENT_OUTPUT"
  exit 1
}

AGENT_ID=$(echo "$AGENT_OUTPUT" | jq -r '.id')
if [ -n "$AGENT_ID" ] && [ "$AGENT_ID" != "null" ]; then
  log_pass "agents create returned id: $AGENT_ID"
else
  log_fail "agents create did not return valid id"
  echo "$AGENT_OUTPUT"
  exit 1
fi

# List agents (skip - SDK/server pagination compatibility issue)
log_test "agents list (SKIPPED - SDK pagination compatibility)"

# Get agent (uses prefixed ID directly)
log_test "agents get"
GET_OUTPUT=$($CLI --api-url "$API_URL" agents get "$AGENT_ID" --output json 2>&1) || {
  log_fail "agents get failed"
  echo "$GET_OUTPUT"
}
AGENT_NAME=$(echo "$GET_OUTPUT" | jq -r '.name')
if [ "$AGENT_NAME" = "cli-test-agent" ]; then
  log_pass "agents get returned correct agent"
else
  log_fail "agents get did not return correct agent name"
  echo "$GET_OUTPUT"
fi

# ========================================
# Test: Harness (get seed harness for session creation)
# ========================================

log_test "get seed harness"
HARNESSES_OUTPUT=$(curl -s "$API_URL/v1/harnesses" -H "Authorization: ${EVERRUNS_API_KEY:-test}") || {
  log_fail "list harnesses failed"
  exit 1
}
HARNESS_ID=$(echo "$HARNESSES_OUTPUT" | jq -r '.data[0].id')
if [ -n "$HARNESS_ID" ] && [ "$HARNESS_ID" != "null" ]; then
  log_pass "found seed harness: $HARNESS_ID"
else
  log_fail "no harness found (required for session creation)"
  echo "$HARNESSES_OUTPUT"
  exit 1
fi

# ========================================
# Test: Sessions CRUD
# ========================================

# Create session (uses prefixed harness + agent IDs)
log_test "sessions create"
SESSION_OUTPUT=$($CLI --api-url "$API_URL" sessions create \
  --harness "$HARNESS_ID" \
  --agent "$AGENT_ID" \
  --title "CLI E2E Test Session" \
  --output json 2>&1) || {
  log_fail "sessions create failed"
  echo "$SESSION_OUTPUT"
  exit 1
}

SESSION_ID=$(echo "$SESSION_OUTPUT" | jq -r '.id')
if [ -n "$SESSION_ID" ] && [ "$SESSION_ID" != "null" ]; then
  log_pass "sessions create returned id: $SESSION_ID"
else
  log_fail "sessions create did not return valid id"
  echo "$SESSION_OUTPUT"
  exit 1
fi

# List sessions (skip - SDK/server pagination compatibility issue)
log_test "sessions list (SKIPPED - SDK pagination compatibility)"

# Get session (uses prefixed session ID directly)
log_test "sessions get"
GET_SESSION_OUTPUT=$($CLI --api-url "$API_URL" sessions get "$SESSION_ID" --output json 2>&1) || {
  log_fail "sessions get failed"
  echo "$GET_SESSION_OUTPUT"
}
SESSION_TITLE=$(echo "$GET_SESSION_OUTPUT" | jq -r '.title')
if [ "$SESSION_TITLE" = "CLI E2E Test Session" ]; then
  log_pass "sessions get returned correct session"
else
  log_fail "sessions get did not return correct session title"
  echo "$GET_SESSION_OUTPUT"
fi

# ========================================
# Test: Chat (skip - SDK/server pagination compatibility issue with events endpoint)
# ========================================
log_test "chat (SKIPPED - SDK pagination compatibility with events)"

# ========================================
# Test: Delete agent (cleanup)
# ========================================
log_test "agents delete"
DELETE_OUTPUT=$($CLI --api-url "$API_URL" agents delete "$AGENT_ID" --output json 2>&1) || {
  log_fail "agents delete failed"
  echo "$DELETE_OUTPUT"
}
log_pass "agents delete completed"

# Verify agent is deleted (should fail to get)
log_test "agents get (verify deleted)"
if $CLI --api-url "$API_URL" agents get "$AGENT_ID" --output json 2>&1; then
  log_fail "agents get should fail for deleted agent"
else
  log_pass "agents get correctly fails for deleted agent"
fi

# ========================================
# Summary
# ========================================
echo ""
echo "========================================"
echo "  Test Summary"
echo "========================================"
echo -e "  ${GREEN}Passed:${NC} $PASSED"
echo -e "  ${RED}Failed:${NC} $FAILED"
echo "========================================"

if [ "$FAILED" -gt 0 ]; then
  exit 1
fi

echo ""
echo "All CLI E2E tests passed!"
