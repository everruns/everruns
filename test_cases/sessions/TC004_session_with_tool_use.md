# TC004: Session with Tool Use

## Description

Verify that an agent with capabilities uses tools during a session and produces tool call events.

## Preconditions

- API server running (`just start-dev`)
- LLM API keys configured

## Test Data

| Field | Value |
|-------|-------|
| Agent Name | Tool Agent |
| Agent Prompt | You are an assistant. Use available tools to help the user. |
| Capabilities | `current_time` |
| User Message | What is the current time? |

## Steps

1. Create agent with `current_time` capability:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{
       "name": "Tool Agent",
       "system_prompt": "You are an assistant. Always use the current_time tool when asked about time.",
       "capabilities": [{"ref": "current_time"}]
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

3. Send message requesting time:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/sessions/{session_id}/messages" \
     -H "Content-Type: application/json" \
     -d '{
       "message": {
         "role": "user",
         "content": [{"type": "text", "text": "What is the current time?"}]
       }
     }'
   ```

4. Wait for turn completion (~15-30 seconds).

5. Get events:
   ```bash
   curl -s "http://localhost:9300/api/v1/sessions/{session_id}/events"
   ```

## Expected Result

### Tool Execution

| Check | Expected |
|-------|----------|
| `tool.called` event | Exists with `tool_name: "current_time"` |
| `tool.completed` event | Exists with successful result |
| `reason.completed` | `has_tool_calls: true` |

### Agent Response

| Check | Expected |
|-------|----------|
| Agent message exists | `output.message.completed` event |
| Response content | Contains a time value |
| Turn completed | `turn.completed` event exists |

## Validation Commands

```bash
# Assert: tool was called
curl -s ".../sessions/{session_id}/events" \
  | jq '[.data[] | select(.type == "tool.called" and .data.tool_name == "current_time")] | length > 0'

# Assert: turn completed
curl -s ".../sessions/{session_id}/events" \
  | jq '[.data[] | select(.type == "turn.completed")] | length > 0'
```
