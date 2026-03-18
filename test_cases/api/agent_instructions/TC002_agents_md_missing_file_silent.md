# TC002: Missing AGENTS.md Silently Ignored

## Description

Verify that when no AGENTS.md file exists in the session filesystem, the agent operates normally without errors.

## Preconditions

- Control-plane running (DEV_MODE or full mode)

## Test Data

| Field | Value |
|-------|-------|
| Harness | Generic (`harness_01933b5a000070008000000000000602`) |
| AGENTS.md | Not created |
| User Message | `Say hello` |

## Steps

1. Create session with Generic harness:
   ```bash
   curl -s -X POST "http://localhost:9000/v1/sessions" \
     -H "Content-Type: application/json" \
     -d '{"harness_id": "harness_01933b5a000070008000000000000602"}'
   ```

2. Do NOT create an AGENTS.md file.

3. Send a message:
   ```bash
   curl -s -X POST "http://localhost:9000/v1/sessions/{session_id}/messages" \
     -H "Content-Type: application/json" \
     -d '{"message": {"content": [{"type": "text", "text": "Say hello"}]}}'
   ```

4. Wait for response.

5. Retrieve messages.

## Expected Result

| Check | Expected |
|-------|----------|
| Agent responds | `data` contains message with `role: "agent"` |
| No errors | No error events in session events |
| Turn completed | `turn.completed` event exists |
