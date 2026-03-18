# TC006: Cancel Active Turn

## Description

Verify that an active turn can be cancelled and the session returns to idle state.

## Preconditions

- API server running (`just start-dev`)
- LLM API keys configured

## Test Data

| Field | Value |
|-------|-------|
| Agent Name | Slow Agent |
| Agent Prompt | You are an assistant. Write a very long, detailed essay about the history of computing. |
| User Message | Write the essay now. |

## Steps

1. Create agent:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{
       "name": "Slow Agent",
       "system_prompt": "You are an assistant. Write a very long, detailed essay about the history of computing."
     }'
   ```
   Save `agent_id`.

2. Create session:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/sessions" \
     -H "Content-Type: application/json" \
     -d '{"agent_id": "{agent_id}"}'
   ```
   Save `session_id`.

3. Send message to trigger a long response:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/sessions/{session_id}/messages" \
     -H "Content-Type: application/json" \
     -d '{
       "message": {
         "role": "user",
         "content": [{"type": "text", "text": "Write the essay now."}]
       }
     }'
   ```

4. While session is active, cancel the turn:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/sessions/{session_id}/cancel"
   ```

5. Wait briefly, then check session status:
   ```bash
   curl -s "http://localhost:9300/api/v1/sessions/{session_id}"
   ```

## Expected Result

| Check | Expected |
|-------|----------|
| Step 4: HTTP status | 200 |
| Step 5: session `status` | `"idle"` |
| Events contain cancellation | `turn.cancelled` or `turn.completed` event |
