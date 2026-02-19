# TC003: AGENTS.md Dynamic Update Between Turns

## Description

Verify that changes to AGENTS.md are picked up on the next turn without restarting the session. The spec requires re-reading on every LLM turn.

## Preconditions

- Control-plane running with a real LLM (not LlmSim)

## Test Data

| Field | Value |
|-------|-------|
| Harness | Generic |
| Initial AGENTS.md | `Always respond in uppercase.` |
| Updated AGENTS.md | `Always respond in French.` |

## Steps

1. Create session with Generic harness.

2. Write initial AGENTS.md:
   ```bash
   curl -s -X POST "http://localhost:9000/v1/sessions/{session_id}/fs/AGENTS.md" \
     -H "Content-Type: application/json" \
     -d '{"content": "Always respond in uppercase."}'
   ```

3. Send first message: `What color is the sky?`

4. Wait for response. Verify response follows initial instructions.

5. Update AGENTS.md:
   ```bash
   curl -s -X PUT "http://localhost:9000/v1/sessions/{session_id}/fs/AGENTS.md" \
     -H "Content-Type: application/json" \
     -d '{"content": "Always respond in French."}'
   ```

6. Send second message: `What color is the sky?`

7. Wait for response. Verify response follows updated instructions.

## Expected Result

| Check | Expected |
|-------|----------|
| First response | Follows initial AGENTS.md instructions (uppercase) |
| Second response | Follows updated AGENTS.md instructions (French) |
| No restart needed | Both responses come from the same session |
