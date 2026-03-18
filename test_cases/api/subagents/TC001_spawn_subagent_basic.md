# TC001: Spawn Subagent - Basic

## Description

Verify that an agent with the `subagents` capability can spawn a subagent via the `spawn_subagent` tool, creating a child session with `parent_session_id` set and returning `subagent_id`, `name`, and `status` in the tool result.

## Preconditions

- Control-plane running (dev or full mode)
- LLM API keys configured via environment variables

## Test Data

| Field | Value |
|-------|-------|
| Agent Name | Subagent Orchestrator |
| Capabilities | `subagents` |
| System Prompt | You are an orchestrator. When asked to delegate, use spawn_subagent with the given name and task. |
| User Message | Spawn a subagent named "Greeter" with the task: "Say hello and list 3 fun facts about cats." |

## Steps

1. Create agent with subagents capability:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{
       "name": "Subagent Orchestrator",
       "system_prompt": "You are an orchestrator. When asked to delegate, use spawn_subagent with the given name and task.",
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

3. Send message requesting subagent spawn:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/sessions/{session_id}/messages" \
     -H "Content-Type: application/json" \
     -d '{
       "message": {
         "role": "user",
         "content": [{"type": "text", "text": "Spawn a subagent named \"Greeter\" with the task: \"Say hello and list 3 fun facts about cats.\""}]
       }
     }'
   ```

4. Wait for completion (60-120 seconds for foreground subagent execution).

5. Retrieve events and session list for assertions:
   ```bash
   # Events
   curl -s "http://localhost:9300/api/v1/sessions/{session_id}/events"

   # All sessions (to find child)
   curl -s "http://localhost:9300/api/v1/sessions"
   ```

## Expected Result

### Tool Call Assertions

| Check | Expected |
|-------|----------|
| spawn_subagent called | `tool.called` event with `tool_name: "spawn_subagent"` |
| Tool args contain name | `arguments.name` is `"Greeter"` |
| Tool args contain task | `arguments.task` is non-empty |

### Tool Result Assertions

| Check | Expected |
|-------|----------|
| subagent_id present | `tool.completed` result contains `subagent_id` (non-empty string) |
| name present | Result `name` equals `"Greeter"` |
| status present | Result `status` is `"completed"` or `"idle"` |
| result present | Result `result` contains non-empty response text |

### Child Session Assertions

| Check | Expected |
|-------|----------|
| Child session exists | A session in the list has `parent_session_id` matching `{session_id}` |
| Subagent name set | Child session `subagent_name` is `"Greeter"` |

### Event Lifecycle Assertions

| Check | Expected |
|-------|----------|
| Turn started | `turn.started` event exists |
| Turn completed | `turn.completed` event exists |
| Session idled | `session.idled` event exists |

## Validation Commands

```bash
# Assert: spawn_subagent tool was called
curl -s ".../events" | jq '[.data[] | select(.type == "tool.called" and .data.tool_name == "spawn_subagent")] | length > 0'

# Assert: tool result contains subagent_id
curl -s ".../events" | jq '[.data[] | select(.type == "tool.completed")] | .[0].data.result | fromjson | has("subagent_id")'

# Assert: child session has parent_session_id
curl -s ".../sessions" | jq '[.data[] | select(.parent_session_id == "{session_id}")] | length > 0'

# Assert: turn completed
curl -s ".../events" | jq '[.data[] | select(.type == "turn.completed")] | length > 0'
```
