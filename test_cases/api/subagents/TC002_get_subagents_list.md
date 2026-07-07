# TC002: List Subagent Tasks After Spawning

## Description

Verify that after spawning multiple subagents, `list_tasks` (from the generic session_tasks capability) returns all subagent tasks with correct names, statuses, and count. This test was previously titled "Get Subagents - List After Spawning" using the retired `get_subagents` tool; it now uses the generic `list_tasks` replacement.

## Preconditions

- Control-plane running (dev or full mode)
- LLM API keys configured via environment variables

## Test Data

| Field | Value |
|-------|-------|
| Agent Name | Multi-Spawner |
| Capabilities | `subagents`, `session_tasks` |
| System Prompt | You are an orchestrator. When asked, spawn subagents as instructed, then call list_tasks with kind="subagent" to list them. |
| User Message | Spawn two subagents: one named "Alpha" with task "Count to 5", and one named "Beta" with task "List 3 colors". After both complete, call list_tasks with kind="subagent" to list all subagent tasks. |

## Steps

1. Create agent:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{
       "name": "Multi-Spawner",
       "system_prompt": "You are an orchestrator. When asked, spawn subagents as instructed, then call list_tasks with kind=\"subagent\" to list them.",
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

3. Send message requesting two subagents and a listing:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/sessions/{session_id}/messages" \
     -H "Content-Type: application/json" \
     -d '{
       "message": {
         "role": "user",
         "content": [{"type": "text", "text": "Spawn two subagents: one named \"Alpha\" with task \"Count to 5\", and one named \"Beta\" with task \"List 3 colors\". After both complete, call list_tasks with kind=\"subagent\" to list all subagent tasks."}]
       }
     }'
   ```

4. Wait for completion (background subagents run in parallel; allow 120-180 seconds for both to finish).

5. Retrieve events:
   ```bash
   curl -s "http://localhost:9300/api/v1/sessions/{session_id}/events"
   ```

## Expected Result

### Spawn Assertions

| Check | Expected |
|-------|----------|
| Alpha spawned | `tool.called` event with `tool_name: "spawn_subagent"` and `arguments.name: "Alpha"` |
| Beta spawned | `tool.called` event with `tool_name: "spawn_subagent"` and `arguments.name: "Beta"` |
| Both completed | Two `tool.completed` events for `spawn_subagent` with `status` in result |

### List Tasks Assertions

| Check | Expected |
|-------|----------|
| list_tasks called | `tool.called` event with `tool_name: "list_tasks"` |
| Result has tasks array | `tool.completed` result contains `tasks` array |
| Count is 2 | Result `count` equals `2` |
| Alpha in list | `tasks` array contains entry with `display_name: "Alpha"` |
| Beta in list | `tasks` array contains entry with `display_name: "Beta"` |
| Each has state | Each entry has `state` field (e.g. `"succeeded"`) |
| Each has task id | Each entry has non-empty `id` field |

## Validation Commands

```bash
# Assert: spawn_subagent called twice
curl -s ".../events" | jq '[.data[] | select(.type == "tool.called" and .data.tool_name == "spawn_subagent")] | length == 2'

# Assert: list_tasks called
curl -s ".../events" | jq '[.data[] | select(.type == "tool.called" and .data.tool_name == "list_tasks")] | length > 0'

# Assert: list_tasks result has count 2
curl -s ".../events" | jq '[.data[] | select(.type == "tool.completed" and .data.tool_name == "list_tasks")] | .[0].data.result | fromjson | .count == 2'

# Assert: both names present in listing
curl -s ".../events" | jq '[.data[] | select(.type == "tool.completed" and .data.tool_name == "list_tasks")] | .[0].data.result | fromjson | .tasks | map(.display_name) | sort == ["Alpha", "Beta"]'
```
