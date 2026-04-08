# TC001: Create Session for Agent

## Description

Verify that a session can be created for an existing agent and starts in the correct initial state.

## Preconditions

- API server running (`just start-dev`)
- An agent exists (create one first)

## Test Data

| Field | Value |
|-------|-------|
| Agent Name | chat-agent |
| Agent Display Name | Chat Agent |
| Agent Prompt | You are a helpful chat assistant. |

## Steps

1. Create agent:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{
       "name": "chat-agent",
       "display_name": "Chat Agent",
       "system_prompt": "You are a helpful chat assistant."
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

3. Fetch session:
   ```bash
   curl -s "http://localhost:9300/api/v1/sessions/{session_id}"
   ```

## Expected Result

| Check | Expected |
|-------|----------|
| HTTP status (create) | 201 |
| `id` format | Starts with `session_` |
| `agent_id` | Matches the created agent |
| `status` | `"started"` |
| `preview` | `null` (no messages yet) |
| `output_preview` | `null` |
| `usage` | All counters at 0 |

## Validation Commands

```bash
# Assert: session created with correct agent
curl -s ".../sessions/{session_id}" | jq '.agent_id == "{agent_id}"'

# Assert: session in started state
curl -s ".../sessions/{session_id}" | jq '.status == "started"'
```
