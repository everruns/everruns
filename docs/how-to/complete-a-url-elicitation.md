---
title: Complete a URL elicitation over the API
description: Drive the pause-and-consent flow from your own client — declare the hint, read the confirm_url_elicitation event, and post the user's decision.
---

When an MCP server asks that a person finish something in their browser, Everruns
pauses the turn and waits for a decision (see
[URL mode elicitation](/features/mcp-url-elicitation/)). The Chat UI renders a
card for this. Any client can do the same over the REST API and the SSE stream.

## 1. Declare that you can ask

The pause only happens for clients that say they can answer it:

```bash
curl -X POST "$EVERRUNS/api/v1/sessions" \
  -H 'content-type: application/json' \
  -d '{
    "agent_id": "agent_01a063c3b55f79f2b5de55fb002e0ae3",
    "hints": { "url_elicitation": true }
  }'
```

Without the hint the turn never pauses and the flow cannot be completed — see
[Clients that cannot pause](#clients-that-cannot-pause).

## 2. Watch for the pause

Send a message as usual. When a tool call hits an elicitation, the session moves
to `waiting_for_tool_results` and a `tool.call_requested` event arrives on
`GET /api/v1/sessions/{session_id}/sse`:

```json
{
  "type": "tool.call_requested",
  "data": {
    "tool_calls": [
      {
        "id": "url_elicitation_01a063cf-c0bb-7891-926e-fd83aeb24d88",
        "name": "confirm_url_elicitation",
        "arguments": {
          "server": "acme_analytics",
          "tool": "run_revenue_report",
          "retry_tool": "mcp_acme_analytics__run_revenue_report",
          "message": "Acme Analytics needs your API key before it can run this report.",
          "url": "https://acme-analytics.example/connect?ref=rev-2026-08",
          "url_host": "acme-analytics.example",
          "url_is_punycode": false
        }
      }
    ]
  }
}
```

Everything needed to render your own surface is in `arguments`: the full URL, the
host to emphasise, whether that host is Punycode, and which server is asking.

Show the URL in full and open it only on an explicit action. Never fetch it on
the user's behalf.

## 3. Post the decision

Post **when the user says they have finished**, not when they open the link. The
server checks whether the out-of-band interaction completed, so consenting at
open time just makes it ask again.

```bash
curl -X POST "$EVERRUNS/api/v1/sessions/$SESSION_ID/mcp-elicitation-consent" \
  -H 'content-type: application/json' \
  -d '{
    "tool_call_id": "url_elicitation_01a063cf-c0bb-7891-926e-fd83aeb24d88",
    "action": "accept"
  }'
```

```json
{ "host": "acme-analytics.example", "status": "active" }
```

`"action": "decline"` records nothing and lets the agent continue without the
tool. Errors worth handling: `404` when the tool call is not a pending
elicitation, `409` when the session is not paused (already answered, or timed
out).

The request body carries only the decision. The server, tool and domain the
consent applies to are read from the event Everruns emitted, so a client cannot
record consent for something the user was never shown.

## 4. Nothing else to do

Everruns records the consent, adds the decision to the conversation as a user
turn, and resumes. Your stream then shows the tool being called again and the
result arriving — the retry answers the MCP server `accept` on your behalf.

## Timing and reuse

- **You have about five minutes.** A session left in `waiting_for_tool_results`
  is swept (`TOOL_RESULT_TIMEOUT_SECS`, default `300`), the pending call is
  completed as a timeout, and the turn resumes without consent.
- **One consent authorises one retry.** It is deleted when used, so a second
  elicitation asks again.
- **Consent is bound to the domain the user saw.** If the server elicits a
  different host on the retry, the consent is not reused and a new
  `confirm_url_elicitation` event arrives.

## Clients that cannot pause

Without the `url_elicitation` hint the turn continues and the model relays the
link, carrying this payload as the tool result:

```json
{
  "code": "url_elicitation_required",
  "url": "https://acme-analytics.example/connect?ref=rev-2026-08",
  "url_host": "acme-analytics.example",
  "url_is_punycode": false,
  "server": "acme_analytics",
  "tool": "run_revenue_report",
  "retry_tool": "mcp_acme_analytics__run_revenue_report",
  "message": "Acme Analytics needs your API key before it can run this report.",
  "declined": false
}
```

That is informational only: with no pause there is no pending call to answer, the
consent endpoint returns `409`, and the tool elicits again on every retry.
Declare the hint for any client that needs these tools to complete.

## Calling Everruns as an MCP server

The reverse direction is plain MCP. Declare the capability in `_meta`:

```json
{
  "_meta": {
    "io.modelcontextprotocol/clientCapabilities": { "elicitation": { "url": {} } }
  }
}
```

`session_set_secret` then answers with an `input_required` result instead of
taking a value:

```json
{
  "resultType": "input_required",
  "requestState": "eyJ1c2VyX2lkIjoi…",
  "inputRequests": {
    "secret": {
      "method": "elicitation/create",
      "params": {
        "mode": "url",
        "url": "https://app.example.com/api/mcp/elicitations/secret?token=eyJ1c2Vy…",
        "message": "Everruns needs the value of 'STRIPE_API_KEY' for session session_…"
      }
    }
  }
}
```

Send the user to that URL, then retry the same call with the state echoed and the
answer under the server's own key:

```json
{
  "name": "session_set_secret",
  "arguments": { "session_id": "session_…", "name": "STRIPE_API_KEY" },
  "requestState": "eyJ1c2VyX2lkIjoi…",
  "inputResponses": { "secret": { "action": "accept" } }
}
```

```json
{ "resultType": "complete", "structuredContent": { "name": "STRIPE_API_KEY", "stored": true } }
```

A client that never declared `elicitation.url` gets `-32021` with the missing
capability named, rather than being asked for the value.

## Related

- [URL mode elicitation](/features/mcp-url-elicitation/)
- [Consume events via SSE](/how-to/consume-events-via-sse/)
