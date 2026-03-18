# TC003: Message Subagent - Send Follow-Up Message

## Description

Verify that `message_subagent` can send a follow-up message to a completed subagent, resuming it and receiving a new response.

## Preconditions

- Control-plane running (dev or full mode)
- LLM API keys configured via environment variables

## Test Data

| Field | Value |
|-------|-------|
| Agent Name | Messenger Orchestrator |
| Capabilities | `subagents` |
| System Prompt | You are an orchestrator. Spawn subagents and message them as instructed. |
| First Message | Spawn a subagent named "Helper" with task "Introduce yourself briefly." |
| Second Message | Now send a message to Helper saying "What is 2 + 2?" |

## Steps

1. Create agent:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{
       "name": "Messenger Orchestrator",
       "system_prompt": "You are an orchestrator. Spawn subagents and message them as instructed.",
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

3. Send first message to spawn subagent:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/sessions/{session_id}/messages" \
     -H "Content-Type: application/json" \
     -d '{
       "message": {
         "role": "user",
         "content": [{"type": "text", "text": "Spawn a subagent named \"Helper\" with task \"Introduce yourself briefly.\""}]
       }
     }'
   ```

4. Wait for first turn to complete (60-120 seconds).

5. Send second message to message the subagent:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/sessions/{session_id}/messages" \
     -H "Content-Type: application/json" \
     -d '{
       "message": {
         "role": "user",
         "content": [{"type": "text", "text": "Now send a message to Helper saying \"What is 2 + 2?\""}]
       }
     }'
   ```

6. Wait for second turn to complete (60-120 seconds).

7. Retrieve events:
   ```bash
   curl -s "http://localhost:9300/api/v1/sessions/{session_id}/events"
   ```

## Expected Result

### Spawn Phase Assertions

| Check | Expected |
|-------|----------|
| spawn_subagent called | `tool.called` event with `tool_name: "spawn_subagent"` and `arguments.name: "Helper"` |
| Spawn completed | `tool.completed` for `spawn_subagent` with `subagent_id` in result |

### Message Phase Assertions

| Check | Expected |
|-------|----------|
| message_subagent called | `tool.called` event with `tool_name: "message_subagent"` |
| Target is Helper | `arguments.name_or_id` is `"Helper"` |
| Message delivered | `tool.completed` result contains `delivered: true` |
| Response received | Result contains `result` with non-empty text |
| Status present | Result contains `status` field |
| subagent_id in result | Result contains `subagent_id` matching the spawned session |

### Event Lifecycle Assertions

| Check | Expected |
|-------|----------|
| Two turns started | Two `turn.started` events |
| Two turns completed | Two `turn.completed` events |

## Validation Commands

```bash
# Assert: spawn_subagent was called
curl -s ".../events" | jq '[.data[] | select(.type == "tool.called" and .data.tool_name == "spawn_subagent")] | length > 0'

# Assert: message_subagent was called
curl -s ".../events" | jq '[.data[] | select(.type == "tool.called" and .data.tool_name == "message_subagent")] | length > 0'

# Assert: message delivered
curl -s ".../events" | jq '[.data[] | select(.type == "tool.completed" and .data.tool_name == "message_subagent")] | .[0].data.result | fromjson | .delivered == true'

# Assert: response received
curl -s ".../events" | jq '[.data[] | select(.type == "tool.completed" and .data.tool_name == "message_subagent")] | .[0].data.result | fromjson | .result | length > 0'

# Assert: two turns completed
curl -s ".../events" | jq '[.data[] | select(.type == "turn.completed")] | length == 2'
```
