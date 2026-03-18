# TC002: Send Message and Get Response

## Description

Verify that sending a user message to a session triggers a turn, the agent responds, and the session transitions through correct states.

## Preconditions

- API server running (`just start-dev`)
- LLM API keys configured

## Test Data

| Field | Value |
|-------|-------|
| Agent Name | Simple Chat Agent |
| Agent Prompt | You are a helpful assistant. Answer concisely. |
| User Message | What is 2 + 2? |

## Steps

1. Create agent:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{
       "name": "Simple Chat Agent",
       "system_prompt": "You are a helpful assistant. Answer concisely."
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

3. Send user message:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/sessions/{session_id}/messages" \
     -H "Content-Type: application/json" \
     -d '{
       "message": {
         "role": "user",
         "content": [{"type": "text", "text": "What is 2 + 2?"}]
       }
     }'
   ```

4. Wait for turn completion (~10-20 seconds).

5. Get session events:
   ```bash
   curl -s "http://localhost:9300/api/v1/sessions/{session_id}/events"
   ```

6. Get messages:
   ```bash
   curl -s "http://localhost:9300/api/v1/sessions/{session_id}/messages"
   ```

7. Check session status:
   ```bash
   curl -s "http://localhost:9300/api/v1/sessions/{session_id}"
   ```

## Expected Result

### Event Lifecycle

| Check | Expected |
|-------|----------|
| `input.message` event | Exists with user message content |
| `turn.started` event | Exists |
| `reason.completed` event | Exists with `success: true` |
| `output.message.completed` event | Exists with agent response |
| `turn.completed` event | Exists |
| `session.idled` event | Exists |

### Messages

| Check | Expected |
|-------|----------|
| Total messages | >= 2 (user + agent) |
| First message role | `"user"` |
| First message text | `"What is 2 + 2?"` |
| Agent message role | `"agent"` |
| Agent message text | Non-empty, mentions `4` |

### Session State

| Check | Expected |
|-------|----------|
| `status` | `"idle"` |
| `preview` | Contains `"What is 2 + 2?"` |
| `output_preview` | Non-empty |
| `usage.total_tokens` | > 0 |

## Validation Commands

```bash
# Assert: agent responded
curl -s ".../sessions/{session_id}/messages" | jq '[.data[] | select(.role == "agent")] | length > 0'

# Assert: turn completed
curl -s ".../sessions/{session_id}/events" | jq '[.data[] | select(.type == "turn.completed")] | length > 0'

# Assert: session is idle
curl -s ".../sessions/{session_id}" | jq '.status == "idle"'
```
