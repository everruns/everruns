# TC003: Message Task - Send Follow-Up to Subagent

## Description

Verify that `message_task` can send a follow-up message to a subagent's task channel and that the subagent processes the message. This test was previously titled "Message Subagent" using the retired `message_subagent` tool; it now uses the generic `message_task` replacement.

## Preconditions

- Control-plane running (dev or full mode)
- LLM API keys configured via environment variables

## Test Data

| Field | Value |
|-------|-------|
| Agent Name | Messenger Orchestrator |
| Capabilities | `subagents`, `session_tasks` |
| System Prompt | You are an orchestrator. Spawn subagents and message them as instructed. Use message_task with the task_id from spawn_agent to send follow-up messages. |
| First Message | Spawn a subagent named "Helper" with task "Introduce yourself briefly." Record the task_id. |
| Second Message | Now send a message to Helper's task saying "What is 2 + 2?" using message_task with Helper's task_id. |

## Steps

1. Create agent:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{
       "name": "Messenger Orchestrator",
       "system_prompt": "You are an orchestrator. Spawn subagents and message them as instructed. Use message_task with the task_id from spawn_agent to send follow-up messages.",
       "capabilities": ["subagents", "session_tasks"]
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
         "content": [{"type": "text", "text": "Spawn a subagent named \"Helper\" with task \"Introduce yourself briefly.\" Record the task_id."}]
       }
     }'
   ```

4. Wait for first turn to complete (60-120 seconds).

5. Send second message to message the subagent via task:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/sessions/{session_id}/messages" \
     -H "Content-Type: application/json" \
     -d '{
       "message": {
         "role": "user",
         "content": [{"type": "text", "text": "Now send a message to Helper’s task saying \"What is 2 + 2?\" using message_task with Helper’s task_id."}]
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
| spawn_agent called | `tool.called` event with `tool_name: "spawn_agent"` and `arguments.name: "Helper"` |
| Spawn completed | `tool.completed` for `spawn_agent` with `task_id` in result |

### Message Phase Assertions

| Check | Expected |
|-------|----------|
| message_task called | `tool.called` event with `tool_name: "message_task"` |
| Message recorded and delivered | `tool.completed` for `message_task` whose result has `recorded: true` and `delivery: "delivered"` |
| Two turns completed | Two `turn.completed` events |

## Validation Commands

```bash
# Assert: spawn_agent was called
curl -s ".../events" | jq '[.data[] | select(.type == "tool.called" and .data.tool_name == "spawn_agent")] | length > 0'

# Assert: message_task was called
curl -s ".../events" | jq '[.data[] | select(.type == "tool.called" and .data.tool_name == "message_task")] | length > 0'

# Assert: message_task recorded and delivered the message
curl -s ".../events" | jq '[.data[] | select(.type == "tool.completed" and .data.tool_name == "message_task" and (.data.result.recorded == true) and (.data.result.delivery == "delivered"))] | length > 0'

# Assert: two turns completed
curl -s ".../events" | jq '[.data[] | select(.type == "turn.completed")] | length == 2'
```
