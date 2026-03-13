# TC005: Subagent Cancel

## Description

Verify that `message_subagent` with `cancel=true` delivers the message and returns `cancel_requested: true` in the response.

## Preconditions

- Control-plane running (dev or full mode)
- LLM API keys configured via environment variables

## Test Data

| Field | Value |
|-------|-------|
| Agent Name | Cancel Tester |
| Capabilities | `subagents` |
| System Prompt | You are an orchestrator. Spawn subagents and cancel them as instructed. Always use the cancel parameter when asked to cancel. |
| First Message | Spawn a subagent named "Worker" with task "Write a long story about a dragon." |
| Second Message | Cancel the Worker subagent with the message "Stop now, we no longer need this." |

## Steps

1. Create agent:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{
       "name": "Cancel Tester",
       "system_prompt": "You are an orchestrator. Spawn subagents and cancel them as instructed. Always use the cancel parameter when asked to cancel.",
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
         "content": [{"type": "text", "text": "Spawn a subagent named \"Worker\" with task \"Write a long story about a dragon.\""}]
       }
     }'
   ```

4. Wait for first turn to complete (60-120 seconds).

5. Send cancel message:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/sessions/{session_id}/messages" \
     -H "Content-Type: application/json" \
     -d '{
       "message": {
         "role": "user",
         "content": [{"type": "text", "text": "Cancel the Worker subagent with the message \"Stop now, we no longer need this.\""}]
       }
     }'
   ```

6. Wait for second turn to complete (30-60 seconds).

7. Retrieve events:
   ```bash
   curl -s "http://localhost:9300/api/v1/sessions/{session_id}/events"
   ```

## Expected Result

### Spawn Phase Assertions

| Check | Expected |
|-------|----------|
| Worker spawned | `tool.called` with `tool_name: "spawn_subagent"` and `arguments.name: "Worker"` |
| Spawn completed | `tool.completed` for `spawn_subagent` with `subagent_id` in result |

### Cancel Phase Assertions

| Check | Expected |
|-------|----------|
| message_subagent called | `tool.called` with `tool_name: "message_subagent"` |
| cancel flag set | `arguments.cancel` is `true` |
| Target is Worker | `arguments.name_or_id` is `"Worker"` |
| Message delivered | Result `delivered` is `true` |
| Cancel requested | Result `cancel_requested` is `true` |
| Note present | Result `note` contains "Cancellation will take effect" |

## Validation Commands

```bash
# Assert: message_subagent called with cancel=true
curl -s ".../events" | jq '[.data[] | select(.type == "tool.called" and .data.tool_name == "message_subagent" and .data.arguments.cancel == true)] | length > 0'

# Assert: cancel_requested in result
curl -s ".../events" | jq '[.data[] | select(.type == "tool.completed" and .data.tool_name == "message_subagent")] | .[0].data.result | fromjson | .cancel_requested == true'

# Assert: delivered
curl -s ".../events" | jq '[.data[] | select(.type == "tool.completed" and .data.tool_name == "message_subagent")] | .[0].data.result | fromjson | .delivered == true'
```
