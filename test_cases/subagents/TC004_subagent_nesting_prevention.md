# TC004: Subagent Nesting Prevention

## Description

Verify that a subagent (a session with `parent_session_id` set) cannot spawn another subagent. The `spawn_subagent` tool must return an error: "nesting not allowed".

## Preconditions

- Control-plane running (dev or full mode)
- LLM API keys configured via environment variables

## Test Data

| Field | Value |
|-------|-------|
| Agent Name | Nesting Tester |
| Capabilities | `subagents` |
| System Prompt | You are an orchestrator. When asked, spawn a subagent whose task instructs it to try spawning another subagent. |
| User Message | Spawn a subagent named "Child" with task: "Try to spawn a subagent named Inner with task 'say hello'. Report what happens." |

## Steps

1. Create agent:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{
       "name": "Nesting Tester",
       "system_prompt": "You are an orchestrator. When asked, spawn a subagent whose task instructs it to try spawning another subagent.",
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

3. Send message that triggers a nested spawn attempt:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/sessions/{session_id}/messages" \
     -H "Content-Type: application/json" \
     -d '{
       "message": {
         "role": "user",
         "content": [{"type": "text", "text": "Spawn a subagent named \"Child\" with task: \"Try to spawn a subagent named Inner with task say hello. Report what happens.\""}]
       }
     }'
   ```

4. Wait for completion (60-120 seconds).

5. Retrieve events for both parent and child sessions:
   ```bash
   # Parent events
   curl -s "http://localhost:9300/api/v1/sessions/{session_id}/events"

   # Find child session
   curl -s "http://localhost:9300/api/v1/sessions" | jq '.data[] | select(.parent_session_id == "{session_id}")'

   # Child events (using child session_id)
   curl -s "http://localhost:9300/api/v1/sessions/{child_session_id}/events"
   ```

## Expected Result

### Parent Session Assertions

| Check | Expected |
|-------|----------|
| Parent spawned Child | `tool.called` with `tool_name: "spawn_subagent"` and `arguments.name: "Child"` |
| Spawn completed | `tool.completed` for `spawn_subagent` with result |

### Child Session Assertions

| Check | Expected |
|-------|----------|
| Child attempted spawn | Child session events contain `tool.called` with `tool_name: "spawn_subagent"` |
| Nesting rejected | Child session events contain `tool.completed` with error containing "nesting not allowed" |
| No grandchild session | No session exists with `parent_session_id` matching the child session ID |

### Nesting Prevention

| Check | Expected |
|-------|----------|
| Error message correct | Tool error text contains "nesting not allowed" |
| Only two sessions exist | Parent session + one child session (no grandchild) |

## Validation Commands

```bash
# Assert: parent spawned a child
curl -s ".../sessions/{session_id}/events" | jq '[.data[] | select(.type == "tool.called" and .data.tool_name == "spawn_subagent")] | length > 0'

# Assert: child tried and failed to spawn
curl -s ".../sessions/{child_session_id}/events" | jq '[.data[] | select(.type == "tool.completed")] | map(select(.data.result | contains("nesting not allowed"))) | length > 0'

# Assert: no grandchild session exists
curl -s ".../sessions" | jq '[.data[] | select(.parent_session_id == "{child_session_id}")] | length == 0'
```
