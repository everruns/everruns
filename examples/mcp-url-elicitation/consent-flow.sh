#!/usr/bin/env bash
#
# URL Mode Elicitation Example
#
# Drives the pause-and-consent flow over the Everruns API, the way a client that
# is not the Chat UI would:
#   1. Register the eliciting MCP server and an agent that can call it
#   2. Create a session that declares the `url_elicitation` hint
#   3. Send a message that triggers the tool
#   4. Read the `confirm_url_elicitation` event and print the URL
#   5. Wait for you to finish on that page, then post the decision
#   6. Show the agent's answer, and prove the secret never entered the session
#
# Prerequisites:
#   - jq installed
#   - Everruns running (default: http://localhost:9301)
#   - An LLM provider configured with an API key
#   - elicit-server.mjs running somewhere Everruns can reach. Its URL must pass
#     the SSRF checks: public https, not localhost and not a private range.
#     A tunnel in front of `node elicit-server.mjs` is the easy way.
#
# Usage:
#   MCP_URL=https://elicit.example.com ./examples/mcp-url-elicitation/consent-flow.sh
#   BASE_URL=http://my-server:9301 MCP_URL=… ./examples/mcp-url-elicitation/consent-flow.sh

set -euo pipefail

BASE_URL="${BASE_URL:-http://localhost:9301}"
MCP_URL="${MCP_URL:-}"

log()  { printf "\n\033[1;34m==> %s\033[0m\n" "$1"; }
info() { printf "    %s\n" "$1"; }
fail() { printf "\033[1;31mERROR: %s\033[0m\n" "$1" >&2; exit 1; }

[ -n "$MCP_URL" ] || fail "Set MCP_URL to the address of elicit-server.mjs (see the header)."
command -v jq > /dev/null || fail "jq is required."

# ── Step 1: An agent that can call the eliciting server ─────────────────────

log "Step 1: Creating an agent wired to ${MCP_URL}"

AGENT_ID=$(curl -fsS -X POST "${BASE_URL}/v1/agents" \
  -H "Content-Type: application/json" \
  -d "$(jq -n --arg url "$MCP_URL" '{
    name: "elicitation-example-agent",
    display_name: "Revenue Reports",
    description: "Runs revenue reports through a server that needs the user API key",
    system_prompt: "You run revenue reports. Call run_revenue_report when asked. If a tool says a person must finish something in their browser, say so briefly and wait.",
    mcp_servers: { example_analytics: { type: "http", url: $url, protocol_mode: "2026-07-28" } }
  }')" | jq -r '.public_id // .id')

info "agent: ${AGENT_ID}"

# ── Step 2: A session that says it can answer an elicitation ────────────────
#
# Without this hint the turn never pauses: the elicitation still reaches the
# model as a tool result, but there is no pending call to answer, so the tool
# can never complete.

log "Step 2: Creating a session with the url_elicitation hint"

SESSION_ID=$(curl -fsS -X POST "${BASE_URL}/v1/sessions" \
  -H "Content-Type: application/json" \
  -d "$(jq -n --arg agent "$AGENT_ID" '{
    agent_id: $agent,
    hints: { url_elicitation: true }
  }')" | jq -r '.id')

info "session: ${SESSION_ID}"

# ── Step 3: Ask for something that needs the tool ───────────────────────────

log "Step 3: Sending the message"

curl -fsS -X POST "${BASE_URL}/v1/sessions/${SESSION_ID}/messages" \
  -H "Content-Type: application/json" \
  -d '{"message":{"role":"user","content":[{"type":"text","text":"Run the revenue report for August 2026."}]}}' \
  > /dev/null

# ── Step 4: Wait for the pause ──────────────────────────────────────────────

log "Step 4: Waiting for the confirm_url_elicitation call"

TOOL_CALL_ID=""
for _ in $(seq 1 60); do
  EVENTS=$(curl -fsS "${BASE_URL}/v1/sessions/${SESSION_ID}/events?limit=100")
  PENDING=$(echo "$EVENTS" | jq -c '
    [ .data[]? | select(.type == "tool.call_requested")
      | .data.tool_calls[]? | select(.name == "confirm_url_elicitation") ] | last // empty')
  if [ -n "$PENDING" ]; then
    TOOL_CALL_ID=$(echo "$PENDING" | jq -r '.id')
    info "server:  $(echo "$PENDING" | jq -r '.arguments.server')"
    info "domain:  $(echo "$PENDING" | jq -r '.arguments.url_host')"
    info "reason:  $(echo "$PENDING" | jq -r '.arguments.message')"
    printf "\n    Open this URL and finish there:\n\n    %s\n\n" \
      "$(echo "$PENDING" | jq -r '.arguments.url')"
    break
  fi
  sleep 2
done

[ -n "$TOOL_CALL_ID" ] || fail "No elicitation arrived. Is the MCP server reachable from Everruns?"

# ── Step 5: Answer once the human is actually done ──────────────────────────
#
# Not when they open the link: the server checks whether the interaction
# completed, so consenting too early just makes it ask again. The pause is swept
# after TOOL_RESULT_TIMEOUT_SECS (default 300), so do not dawdle.

log "Step 5: Waiting for you"
read -r -p "    Press Enter once you have entered the key on that page… " _

curl -fsS -X POST "${BASE_URL}/v1/sessions/${SESSION_ID}/mcp-elicitation-consent" \
  -H "Content-Type: application/json" \
  -d "$(jq -n --arg id "$TOOL_CALL_ID" '{ tool_call_id: $id, action: "accept" }')" \
  | jq -c '.'

# ── Step 6: The retry runs on its own ───────────────────────────────────────

log "Step 6: Waiting for the answer"

for _ in $(seq 1 60); do
  STATUS=$(curl -fsS "${BASE_URL}/v1/sessions/${SESSION_ID}" | jq -r '.status')
  case "$STATUS" in
    idle|completed|failed) break ;;
  esac
  sleep 2
done

curl -fsS "${BASE_URL}/v1/sessions/${SESSION_ID}/events?limit=200" \
  | jq -r '[ .data[]? | select(.type == "output.message.completed")
             | .data.message.content[]? | select(.type == "text") | .text ] | last // "no answer"'

log "The secret never entered the session"
curl -fsS "${BASE_URL}/v1/sessions/${SESSION_ID}/events?limit=200" \
  | jq -r 'if (tostring | test("sk-live")) then "  FOUND a key in the events — that should not happen"
           else "  no API key anywhere in the session events" end'
