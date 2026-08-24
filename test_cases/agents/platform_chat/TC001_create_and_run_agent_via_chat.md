# TC001: Create and Run an Agent via Platform Chat

## Description

Verify that a user can use the Platform Chat global session to create a new agent end-to-end and then ask Platform Chat to invoke that agent — all from a single conversation. The chat must surface the new agent through clickable navigation links (not raw prefixed IDs), the run-agent flow must produce a coherent reply with structured tool blocks, and no harness-deletion error banners or raw tool-call leakage may appear.

This test exercises the Platform Chat harness's catalog-backed `platform`
capability path (`discover`, `query`, and `execute`) plus the chat UI's link
rendering, tool-call formatting, and error-banner suppression.

## Preconditions

- Control-plane running (`just start-dev` or `just start-all`)
- An LLM API key configured (OpenAI, Anthropic, or Gemini — Platform Chat works with any frontier chat model)
- The signed-in user has a fresh Platform Chat thread (or is willing to start one). The old singleton `/chat` page was retired with EVE-855; a Platform Chat conversation is now an ordinary chat thread bound to the built-in `platform-chat` harness
- Default org has the built-in `platform-chat`, `generic`, and `base` harnesses provisioned (handled by org init)

## Test Data

| Field | Value |
|-------|-------|
| Agent name to create | `weather-bot` |
| Agent display name | `Weather Bot` |
| Harness for the new agent | `generic` (built-in default) |
| Run prompt | `What's the weather like in Paris?` (agent will answer from training data — no live tool needed) |

## Steps

### Happy path — create and run

1. **Open Platform Chat** in the web UI: go to `/chats`, start a **New chat**, and pick the built-in **Platform Chat** harness. Note the thread URL (`/chats/{threadId}`).

2. **Send the create message:**

   ```
   Create an agent named "weather-bot" with display name "Weather Bot" that answers weather questions. Use the generic harness.
   ```

3. **Wait for Platform Chat to finish.** The reply should:
   - Confirm the agent was created.
   - Include a **clickable link to the agent** (e.g. the agent's display name rendered as an `<a>` tag pointing at `/agents/{id}`), not a bare `agent_...` prefixed ID copy-pasted into the prose.

4. **Click the agent link.** The agent detail page for `weather-bot` should load and show the display name (`Weather Bot`) plus any capabilities Platform Chat configured. Navigate back to chat. (Harness is per-session, not per-agent — verify the harness on the spawned session page in step 6 instead.)

5. **Send the run message** in the same Platform Chat session:

   ```
   Ask weather-bot: What's the weather like in Paris?
   ```

6. **Wait for Platform Chat to finish.** It should:
   - Create a new session against `weather-bot` (or reuse one via tag), send the prompt, wait for idle, and fetch the reply.
   - Surface a **clickable link to the spawned session**, not a raw `session_...` ID.
   - Echo a coherent answer to the Paris weather question (typical/seasonal answer is fine — model-dependent).
   - On the linked session page, the harness should be `generic`.

### Negative path A — empty agent name

7. **Send:**

   ```
   Create an agent with no name.
   ```

8. **Expected:** Platform Chat declines with a clear validation error explaining that a name is required (or asks the user to supply one). No 500 banner, no stack trace, no silent success.

### Negative path B — run a nonexistent agent

9. **Send:**

   ```
   Ask nonexistent-bot-xyz: hello?
   ```

10. **Expected:** Platform Chat reports that the agent was not found (text such as "agent not found" or "no agent named nonexistent-bot-xyz"). It must NOT spawn an empty session, must NOT crash, and must NOT show the harness-deletion error described below.

## Expected Result

### Capability Wiring

- Platform Chat session uses the focused `platform-chat` built-in harness (which inherits from `base` and adds `platform` plus runtime safeguards).
- The trace uses `discover` when command names or schemas are unknown, `query`
  for reads, and `execute` for the requested mutations.
- Underlying agent/session commands succeed without authorization,
  multitenancy, or quota errors.

### Happy Path Rendering

- The created-agent confirmation message contains an `<a href="/agents/...">Weather Bot</a>` (or equivalent route) — the rendered text must be the human-readable name, not the raw `agent_01...` ID.
- The run-agent confirmation message contains an `<a href="/sessions/...">` link — again, no bare `session_01...` prefix in the visible prose.
- The agent's reply ("Paris weather…") appears in the chat and reads as a normal assistant message.
- Tool calls in the chat transcript render as **structured tool blocks** (the standard tool-call UI element), NOT as literal `to=manage_agents` / `to=session_send_message` strings or raw JSON dumps.
- No red error banner is shown.
- The chat does NOT contain the string `Execution stopped because the assigned harness was deleted` anywhere in the transcript.

### Negative Path A — Empty Name

- The chat reply explains the missing name. No new agent appears in the agents list.
- No HTTP 500 toast, no internal-error banner.

### Negative Path B — Nonexistent Agent

- The chat reply clearly states the agent does not exist.
- No new session row is created against a deleted/missing harness.
- No `Execution stopped because the assigned harness was deleted` text.

### Persistence

- Refreshing `/chats/{threadId}` preserves the Platform Chat transcript, and the thread stays listed on `/chats`.
- The `weather-bot` agent persists in the agents list and is reachable via direct URL after browser reload.

## Failure Modes

| Failure | What to look for |
|---------|-----------------|
| Raw IDs in chat prose | Reply contains `agent_01...` or `session_01...` instead of clickable links — check command response decoration and Platform Chat link rendering |
| Literal `to=functions.X` text | Tool calls leak as raw assistant text — check `message-content.tsx` tool-call detection / streaming parser |
| `Execution stopped because the assigned harness was deleted` | Session was created against a stale/missing `harness_id`. Check org init, harness reconciliation, and that Platform Chat is resolving `generic` by name (not a stale UUID) |
| 500 / red banner on empty name | `create_agent` validation missing — server should return 4xx with a structured error that the chat surfaces as plain text |
| Nonexistent-agent run spawns a session anyway | `send_message` / `create_session` accepting an unknown agent name — should be rejected before spawn |
| Agent link 404 | Agent created but UI route missing — check agents detail route registration |
| Platform Chat thread lost on reload | Thread not listed by the Chats surface — check the `chat` tag written on thread creation and `selectChatThreads` |

## Notes

- Deterministic LLM output is not guaranteed; the model may phrase things differently or use different tool-call sequences. Check rendered semantics (links present, agent created, reply coherent, no error banners) rather than exact wording.
- Run on at least one OpenAI and one Anthropic model — Platform Chat's tool-calling reliability varies by provider.
- This test exercises the same flow used by the in-product onboarding tour; regressions here usually break the first-run experience.
- The "ask weather-bot" prompt does not require the agent to have any web/network capability — answering from training data is sufficient. To extend this test for live weather data, attach a `web_fetch` capability to `weather-bot` first.
