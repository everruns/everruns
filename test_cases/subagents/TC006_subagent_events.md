# TC006: Subagent SSE Events - Lifecycle Events

## Description

Verify that the SSE event stream emits `subagent.spawned` and `subagent.completed` events during subagent lifecycle, with correct event data containing `subagent_session_id`, `subagent_name`, `task`, and `status`.

## Preconditions

- Control-plane running (dev or full mode)
- LLM API keys configured via environment variables

## Test Data

| Field | Value |
|-------|-------|
| Agent Name | Events Tester |
| Capabilities | `subagents` |
| System Prompt | You are an orchestrator. When asked, spawn a subagent as instructed. |
| User Message | Spawn a subagent named "Ping" with task "Reply with pong." |

## Steps

1. Create agent:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{
       "name": "Events Tester",
       "system_prompt": "You are an orchestrator. When asked, spawn a subagent as instructed.",
       "capabilities": ["subagents"]
     }'
   ```
   Save `agent_id` from response.

2. Create session:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/sessions" \
     -H "Content-Type: application/json" \
     -d '{"agent_id": "{agent_id}"}'
   ```
   Save `session_id` from response.

3. Send message to spawn subagent:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/sessions/{session_id}/messages" \
     -H "Content-Type: application/json" \
     -d '{
       "message": {
         "role": "user",
         "content": [{"type": "text", "text": "Spawn a subagent named \"Ping\" with task \"Reply with pong.\""}]
       }
     }'
   ```

4. Wait for completion (60-120 seconds).

5. Retrieve all events:
   ```bash
   curl -s "http://localhost:9300/api/v1/sessions/{session_id}/events"
   ```

## Expected Result

### Subagent Spawned Event

| Check | Expected |
|-------|----------|
| Event exists | Event with `type: "subagent.spawned"` exists |
| subagent_session_id | `data.subagent_session_id` is a valid session ID (non-empty) |
| subagent_name | `data.subagent_name` equals `"Ping"` |
| task | `data.task` contains the task description |
| status | `data.status` is `"spawning"` or `"running"` |

### Subagent Completed Event

| Check | Expected |
|-------|----------|
| Event exists | Event with `type: "subagent.completed"` exists |
| subagent_session_id | `data.subagent_session_id` matches the spawned session ID |
| subagent_name | `data.subagent_name` equals `"Ping"` |
| task | `data.task` contains the task description |
| status | `data.status` is `"completed"` |

### Event Ordering

| Check | Expected |
|-------|----------|
| Spawn before complete | `subagent.spawned` event sequence number < `subagent.completed` sequence number |
| Both within turn | Events occur between `turn.started` and `turn.completed` |

### Standard Lifecycle Events

| Check | Expected |
|-------|----------|
| Turn started | `turn.started` event exists |
| Turn completed | `turn.completed` event exists |
| Session idled | `session.idled` event exists |

## Validation Commands

```bash
# Assert: subagent.spawned event exists
curl -s ".../events" | jq '[.data[] | select(.type == "subagent.spawned")] | length > 0'

# Assert: subagent.completed event exists
curl -s ".../events" | jq '[.data[] | select(.type == "subagent.completed")] | length > 0'

# Assert: spawned event has correct name
curl -s ".../events" | jq '[.data[] | select(.type == "subagent.spawned")] | .[0].data.subagent_name == "Ping"'

# Assert: completed event has correct name
curl -s ".../events" | jq '[.data[] | select(.type == "subagent.completed")] | .[0].data.subagent_name == "Ping"'

# Assert: spawned event has subagent_session_id
curl -s ".../events" | jq '[.data[] | select(.type == "subagent.spawned")] | .[0].data.subagent_session_id | length > 0'

# Assert: completed event has status "completed"
curl -s ".../events" | jq '[.data[] | select(.type == "subagent.completed")] | .[0].data.status == "completed"'

# Assert: spawned before completed (by sequence)
curl -s ".../events" | jq '([.data[] | select(.type == "subagent.spawned")] | .[0].sequence) < ([.data[] | select(.type == "subagent.completed")] | .[0].sequence)'
```
