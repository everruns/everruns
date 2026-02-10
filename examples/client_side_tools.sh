#!/usr/bin/env bash
#
# Client-Side Tools Example
#
# Demonstrates the full client-side tool call flow using the Everruns API:
#   1. Create an agent with a client-side tool
#   2. Create a session
#   3. Send a message that triggers the client-side tool
#   4. Poll SSE for the tool.call_requested event
#   5. Submit tool results
#   6. Poll SSE for the final response
#
# Prerequisites:
#   - jq installed (https://stedolan.github.io/jq/)
#   - Everruns server running (default: http://localhost:9000)
#   - An LLM provider configured with an API key
#
# Usage:
#   ./examples/client_side_tools.sh
#   BASE_URL=http://my-server:9000 ./examples/client_side_tools.sh

set -euo pipefail

BASE_URL="${BASE_URL:-http://localhost:9000}"

# ── Helpers ──────────────────────────────────────────────────────────────────

log()  { printf "\n\033[1;34m==> %s\033[0m\n" "$1"; }
info() { printf "    %s\n" "$1"; }
fail() { printf "\033[1;31mERROR: %s\033[0m\n" "$1" >&2; exit 1; }

# ── Step 1: Create an agent with a client-side tool ─────────────────────────

log "Step 1: Creating agent with client-side tool"

AGENT_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "${BASE_URL}/v1/agents" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "CRM Assistant",
    "system_prompt": "You help users look up customer information. When the user asks about a customer, use the lookup_crm tool with their customer ID.",
    "client_tools": [
      {
        "type": "client",
        "name": "lookup_crm",
        "description": "Look up a customer record in the CRM system by customer ID. Returns name, email, plan, and signup date.",
        "parameters": {
          "type": "object",
          "properties": {
            "customer_id": {
              "type": "string",
              "description": "The customer ID to look up (e.g. CUST-42)"
            }
          },
          "required": ["customer_id"]
        }
      }
    ]
  }')

HTTP_CODE=$(echo "$AGENT_RESPONSE" | tail -1)
AGENT_BODY=$(echo "$AGENT_RESPONSE" | sed '$d')

if [ "$HTTP_CODE" -ne 200 ] && [ "$HTTP_CODE" -ne 201 ]; then
  fail "Failed to create agent (HTTP $HTTP_CODE): $AGENT_BODY"
fi

AGENT_ID=$(echo "$AGENT_BODY" | jq -r '.id')
info "Agent created: $AGENT_ID"

# ── Step 2: Create a session ────────────────────────────────────────────────

log "Step 2: Creating session"

SESSION_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "${BASE_URL}/v1/sessions" \
  -H "Content-Type: application/json" \
  -d "{
    \"agent_id\": \"${AGENT_ID}\"
  }")

HTTP_CODE=$(echo "$SESSION_RESPONSE" | tail -1)
SESSION_BODY=$(echo "$SESSION_RESPONSE" | sed '$d')

if [ "$HTTP_CODE" -ne 200 ] && [ "$HTTP_CODE" -ne 201 ]; then
  fail "Failed to create session (HTTP $HTTP_CODE): $SESSION_BODY"
fi

SESSION_ID=$(echo "$SESSION_BODY" | jq -r '.id')
info "Session created: $SESSION_ID"

# ── Step 3: Send a message that should trigger the client tool ──────────────

log "Step 3: Sending message to trigger client-side tool"

MSG_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "${BASE_URL}/v1/sessions/${SESSION_ID}/messages" \
  -H "Content-Type: application/json" \
  -d '{
    "message": {
      "content": [
        { "type": "text", "text": "Look up customer CUST-42 for me." }
      ]
    }
  }')

HTTP_CODE=$(echo "$MSG_RESPONSE" | tail -1)
MSG_BODY=$(echo "$MSG_RESPONSE" | sed '$d')

if [ "$HTTP_CODE" -ne 200 ] && [ "$HTTP_CODE" -ne 201 ]; then
  fail "Failed to send message (HTTP $HTTP_CODE): $MSG_BODY"
fi

info "Message sent. Waiting for tool.call_requested event..."

# ── Step 4: Poll SSE for tool.call_requested event ──────────────────────────

log "Step 4: Polling SSE for tool.call_requested"

# Read SSE stream with a timeout. We look for the tool.call_requested event
# which signals the LLM wants to call our client-side tool.
TOOL_CALL_DATA=""
TIMEOUT=60  # seconds

# Use curl to stream SSE events, parsing line by line
TOOL_CALL_DATA=$(timeout "$TIMEOUT" curl -s -N \
  "${BASE_URL}/v1/sessions/${SESSION_ID}/sse" 2>/dev/null | \
  while IFS= read -r line; do
    # SSE format: "event: <type>\ndata: <json>\n\n"
    if echo "$line" | grep -q "^data:"; then
      DATA=$(echo "$line" | sed 's/^data: *//')
      EVENT_TYPE=$(echo "$DATA" | jq -r '.type // empty' 2>/dev/null)
      if [ "$EVENT_TYPE" = "tool.call_requested" ]; then
        echo "$DATA"
        break
      fi
    fi
  done) || true

if [ -z "$TOOL_CALL_DATA" ]; then
  fail "Timed out waiting for tool.call_requested event (${TIMEOUT}s)"
fi

info "Received tool.call_requested event"

# Extract tool call details
TOOL_CALL_ID=$(echo "$TOOL_CALL_DATA" | jq -r '.data.tool_calls[0].id')
TOOL_NAME=$(echo "$TOOL_CALL_DATA" | jq -r '.data.tool_calls[0].name')
TOOL_ARGS=$(echo "$TOOL_CALL_DATA" | jq -c '.data.tool_calls[0].arguments')

info "Tool: $TOOL_NAME"
info "Call ID: $TOOL_CALL_ID"
info "Arguments: $TOOL_ARGS"

# ── Step 5: Submit tool results ─────────────────────────────────────────────

log "Step 5: Submitting tool results"

# Simulate CRM lookup -- in a real integration this would call your CRM API
CUSTOMER_ID=$(echo "$TOOL_ARGS" | jq -r '.customer_id')
info "Simulating CRM lookup for: $CUSTOMER_ID"

RESULT_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "${BASE_URL}/v1/sessions/${SESSION_ID}/tool-results" \
  -H "Content-Type: application/json" \
  -d "{
    \"tool_results\": [
      {
        \"tool_call_id\": \"${TOOL_CALL_ID}\",
        \"result\": {
          \"customer_id\": \"${CUSTOMER_ID}\",
          \"name\": \"Alice Johnson\",
          \"email\": \"alice@example.com\",
          \"plan\": \"enterprise\",
          \"since\": \"2022-03-15\",
          \"status\": \"active\"
        }
      }
    ]
  }")

HTTP_CODE=$(echo "$RESULT_RESPONSE" | tail -1)
RESULT_BODY=$(echo "$RESULT_RESPONSE" | sed '$d')

if [ "$HTTP_CODE" -ne 200 ]; then
  fail "Failed to submit tool results (HTTP $HTTP_CODE): $RESULT_BODY"
fi

info "Tool results accepted"

# ── Step 6: Poll SSE for the final agent response ───────────────────────────

log "Step 6: Polling SSE for final response"

FINAL_RESPONSE=""

FINAL_RESPONSE=$(timeout "$TIMEOUT" curl -s -N \
  "${BASE_URL}/v1/sessions/${SESSION_ID}/sse" 2>/dev/null | \
  while IFS= read -r line; do
    if echo "$line" | grep -q "^data:"; then
      DATA=$(echo "$line" | sed 's/^data: *//')
      EVENT_TYPE=$(echo "$DATA" | jq -r '.type // empty' 2>/dev/null)
      if [ "$EVENT_TYPE" = "output.message.completed" ]; then
        echo "$DATA"
        break
      fi
    fi
  done) || true

if [ -z "$FINAL_RESPONSE" ]; then
  fail "Timed out waiting for output.message.completed event (${TIMEOUT}s)"
fi

# Extract and display the agent's response
AGENT_TEXT=$(echo "$FINAL_RESPONSE" | jq -r '.data.message.content[] | select(.type == "text") | .text')

log "Agent response:"
info "$AGENT_TEXT"

# ── Cleanup info ─────────────────────────────────────────────────────────────

log "Done"
info "Agent ID:   $AGENT_ID"
info "Session ID: $SESSION_ID"
info ""
info "You can continue the conversation:"
info "  curl -X POST ${BASE_URL}/v1/sessions/${SESSION_ID}/messages \\"
info "    -H 'Content-Type: application/json' \\"
info "    -d '{\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"What plan is this customer on?\"}]}}'"
