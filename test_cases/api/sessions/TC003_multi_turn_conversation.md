# TC003: Multi-Turn Conversation

## Description

Verify that an agent maintains context across multiple turns in the same session.

## Preconditions

- API server running (`just start-dev`)
- LLM API keys configured

## Test Data

| Field | Value |
|-------|-------|
| Agent Name | Memory Agent |
| Agent Prompt | You are a helpful assistant. Remember context from previous messages. |
| Message 1 | My name is Alice. |
| Message 2 | What is my name? |

## Steps

1. Create agent:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{
       "name": "Memory Agent",
       "system_prompt": "You are a helpful assistant. Remember context from previous messages."
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

3. Send first message:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/sessions/{session_id}/messages" \
     -H "Content-Type: application/json" \
     -d '{
       "message": {
         "role": "user",
         "content": [{"type": "text", "text": "My name is Alice."}]
       }
     }'
   ```

4. Wait for turn completion (~10-20 seconds).

5. Send second message:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/sessions/{session_id}/messages" \
     -H "Content-Type: application/json" \
     -d '{
       "message": {
         "role": "user",
         "content": [{"type": "text", "text": "What is my name?"}]
       }
     }'
   ```

6. Wait for turn completion (~10-20 seconds).

7. Get all messages:
   ```bash
   curl -s "http://localhost:9300/api/v1/sessions/{session_id}/messages"
   ```

## Expected Result

| Check | Expected |
|-------|----------|
| Total messages | >= 4 (2 user + 2 agent) |
| Second agent response | Mentions "Alice" |
| Session `status` | `"idle"` |
| `usage.total_tokens` | > 0, reflects both turns |
| Turn events | Two `turn.completed` events |

## Validation Commands

```bash
# Assert: agent remembers name
curl -s ".../sessions/{session_id}/messages" \
  | jq '[.data[] | select(.role == "agent")] | last | .content[0].text | test("Alice"; "i")'

# Assert: two turns completed
curl -s ".../sessions/{session_id}/events" \
  | jq '[.data[] | select(.type == "turn.completed")] | length == 2'
```
