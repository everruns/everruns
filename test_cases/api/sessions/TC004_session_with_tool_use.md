# TC004: Session with Tool Use

## Description

Verify that an agent with capabilities uses tools during a session and produces tool call events.

## Preconditions

- API server running locally (`just start-dev`) or a deployed API is available
- Set `BASE_URL` to the API origin (for example, `http://localhost:9300`)
- For authenticated deployments, configure `curl` with the required authorization and organization headers
- LLM API keys configured

## Test Data

| Field | Value |
|-------|-------|
| Agent Name | tool-agent |
| Agent Prompt | You are an assistant. Use available tools to help the user. |
| Capabilities | `current_time` |
| User Message | Use the current-time tool to get the current UTC time, convert it to America/Chicago, and return it in 24-hour ISO 8601 format with the UTC offset. |

## Steps

1. Create agent with `current_time` capability:
   ```bash
   curl -s -X POST "${BASE_URL}/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{
       "name": "tool-agent",
       "system_prompt": "You are an assistant. Always use the current_time tool when asked about time.",
       "capabilities": [{"ref": "current_time"}]
     }'
   ```
   Save `agent_id`.

2. Create session:
   ```bash
   curl -s -X POST "${BASE_URL}/api/v1/sessions" \
     -H "Content-Type: application/json" \
     -d '{"agent_id": "{agent_id}"}'
   ```
   Save `session_id`.

3. Send message requesting time:
   ```bash
   curl -s -X POST "${BASE_URL}/api/v1/sessions/{session_id}/messages" \
     -H "Content-Type: application/json" \
     -d '{
       "message": {
         "role": "user",
         "content": [{"type": "text", "text": "Use the current-time tool to get the current UTC time, convert it to America/Chicago, and return it in 24-hour ISO 8601 format with the UTC offset."}]
       }
     }'
   ```

4. Wait for turn completion (~15-30 seconds).

5. Get events:
   ```bash
   curl -s "${BASE_URL}/api/v1/sessions/{session_id}/events"
   ```

## Expected Result

### Tool Execution

| Check | Expected |
|-------|----------|
| `act.started` event | Exists before tool execution |
| `tool.started` event | Exists for the current-time tool |
| `tool.completed` event | Exists with successful result |
| `act.completed` event | Exists after tool execution |

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
  | jq '[.data[] | select(.type == "tool.started" and (.data.tool_call.name == "current_time" or .data.tool_call.name == "get_current_time"))] | length > 0'

# Assert: tool completed successfully
curl -s ".../sessions/{session_id}/events" \
  | jq '[.data[] | select(.type == "tool.completed" and .data.success == true)] | length > 0'

# Assert: turn completed
curl -s ".../sessions/{session_id}/events" \
  | jq '[.data[] | select(.type == "turn.completed")] | length > 0'
```
