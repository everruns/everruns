# TC002: Get Subagents - List After Spawning

## Description

Verify that after spawning multiple subagents, the `get_subagents` tool (with no arguments) returns all child sessions with correct names, statuses, and count.

## Preconditions

- Control-plane running (dev or full mode)
- LLM API keys configured via environment variables

## Test Data

| Field | Value |
|-------|-------|
| Agent Name | Multi-Spawner |
| Capabilities | `subagents` |
| System Prompt | You are an orchestrator. When asked, spawn subagents as instructed, then call get_subagents to list them. |
| User Message | Spawn two subagents: one named "Alpha" with task "Count to 5", and one named "Beta" with task "List 3 colors". After both complete, call get_subagents to list all subagents. |

## Steps

1. Create agent:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{
       "name": "Multi-Spawner",
       "system_prompt": "You are an orchestrator. When asked, spawn subagents as instructed, then call get_subagents to list them.",
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

3. Send message requesting two subagents and a listing:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/sessions/{session_id}/messages" \
     -H "Content-Type: application/json" \
     -d '{
       "message": {
         "role": "user",
         "content": [{"type": "text", "text": "Spawn two subagents: one named \"Alpha\" with task \"Count to 5\", and one named \"Beta\" with task \"List 3 colors\". After both complete, call get_subagents to list all subagents."}]
       }
     }'
   ```

4. Wait for completion (120-180 seconds for two foreground subagents sequentially).

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

### Get Subagents Assertions

| Check | Expected |
|-------|----------|
| get_subagents called | `tool.called` event with `tool_name: "get_subagents"` |
| Result has subagents array | `tool.completed` result contains `subagents` array |
| Count is 2 | Result `count` equals `2` |
| Alpha in list | `subagents` array contains entry with `name: "Alpha"` |
| Beta in list | `subagents` array contains entry with `name: "Beta"` |
| Each has status | Each entry has `status` field (e.g. `"completed"` or `"idle"`) |
| Each has subagent_id | Each entry has non-empty `subagent_id` |

## Validation Commands

```bash
# Assert: spawn_subagent called twice
curl -s ".../events" | jq '[.data[] | select(.type == "tool.called" and .data.tool_name == "spawn_subagent")] | length == 2'

# Assert: get_subagents called
curl -s ".../events" | jq '[.data[] | select(.type == "tool.called" and .data.tool_name == "get_subagents")] | length > 0'

# Assert: get_subagents result has count 2
curl -s ".../events" | jq '[.data[] | select(.type == "tool.completed" and .data.tool_name == "get_subagents")] | .[0].data.result | fromjson | .count == 2'

# Assert: both names present in listing
curl -s ".../events" | jq '[.data[] | select(.type == "tool.completed" and .data.tool_name == "get_subagents")] | .[0].data.result | fromjson | .subagents | map(.name) | sort == ["Alpha", "Beta"]'
```
