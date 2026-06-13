# TC005: Cancel Task - Subagent Cancellation

## Description

Verify that `cancel_task` delivers a cooperative cancellation request to a subagent task and that the task transitions to a canceled state. This test was previously titled "Subagent Cancel" using the retired `message_subagent(cancel=true)` mechanism; it now uses the generic `cancel_task` replacement.

## Preconditions

- Control-plane running (dev or full mode)
- LLM API keys configured via environment variables

## Test Data

| Field | Value |
|-------|-------|
| Agent Name | Cancel Tester |
| Capabilities | `subagents`, `session_tasks` |
| System Prompt | You are an orchestrator. Spawn subagents and cancel them using cancel_task when instructed. Use the task_id returned by spawn_subagent. |
| First Message | Spawn a subagent named "Worker" with task "Write a long story about a dragon." Record the task_id. |
| Second Message | Cancel the Worker subagent using cancel_task with Worker's task_id. |

## Steps

1. Create agent:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{
       "name": "Cancel Tester",
       "system_prompt": "You are an orchestrator. Spawn subagents and cancel them using cancel_task when instructed. Use the task_id returned by spawn_subagent.",
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
         "content": [{"type": "text", "text": "Spawn a subagent named \"Worker\" with task \"Write a long story about a dragon.\" Record the task_id."}]
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
         "content": [{"type": "text", "text": "Cancel the Worker subagent using cancel_task with Worker's task_id."}]
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
| Spawn completed | `tool.completed` for `spawn_subagent` with `task_id` in result |

### Cancel Phase Assertions

| Check | Expected |
|-------|----------|
| cancel_task called | `tool.called` with `tool_name: "cancel_task"` |
| Cancel succeeds | `tool.completed` for `cancel_task` without error |
| Task state reflects cancellation | `task.updated` event with `state: "canceled"` for the Worker task |

## Validation Commands

```bash
# Assert: cancel_task called
curl -s ".../events" | jq '[.data[] | select(.type == "tool.called" and .data.tool_name == "cancel_task")] | length > 0'

# Assert: task.updated event shows canceled state
curl -s ".../events" | jq '[.data[] | select(.type == "task.updated" and .data.task.state == "canceled")] | length > 0'
```
