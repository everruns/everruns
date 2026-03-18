# TC001: AGENTS.md Applied via Generic Harness

## Description

Verify that AGENTS.md instructions are automatically applied when using the Generic harness (which includes `agent_instructions` capability by default). The agent should follow instructions written to AGENTS.md on subsequent turns.

## Preconditions

- Control-plane running (DEV_MODE or full mode)
- LLM API key configured (non-simulated model required to verify instruction following)

## Test Data

| Field | Value |
|-------|-------|
| Harness | Generic (`harness_01933b5a000070008000000000000602`) |
| AGENTS.md Content | `# Instructions\n\nAlways end your response with "-- Agent X"` |
| User Message | `What is 2+2?` |

## Steps

1. Create session with Generic harness:
   ```bash
   curl -s -X POST "http://localhost:9000/v1/sessions" \
     -H "Content-Type: application/json" \
     -d '{"harness_id": "harness_01933b5a000070008000000000000602"}'
   ```
   Save `session_id` from response.

2. Write AGENTS.md to session filesystem:
   ```bash
   curl -s -X POST "http://localhost:9000/v1/sessions/{session_id}/fs/AGENTS.md" \
     -H "Content-Type: application/json" \
     -d '{"content": "# Instructions\n\nAlways end your response with \"-- Agent X\""}'
   ```

3. Send a message:
   ```bash
   curl -s -X POST "http://localhost:9000/v1/sessions/{session_id}/messages" \
     -H "Content-Type: application/json" \
     -d '{"message": {"content": [{"type": "text", "text": "What is 2+2?"}]}}'
   ```

4. Wait for response (10-30 seconds).

5. Retrieve messages:
   ```bash
   curl -s "http://localhost:9000/v1/sessions/{session_id}/messages"
   ```

## Expected Result

| Check | Expected |
|-------|----------|
| Agent responds | `data` contains message with `role: "agent"` |
| Instructions followed | Agent response ends with `-- Agent X` |
| AGENTS.md readable | `GET /v1/sessions/{id}/fs/AGENTS.md` returns 200 with correct content |
