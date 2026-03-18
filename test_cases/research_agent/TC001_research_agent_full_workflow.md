# TC001: Research Agent - Full Workflow

## Description

Verify that the research agent completes a full research workflow: receives a question, uses tools (web_fetch, file creation), and returns a comprehensive response with saved research files.

## Preconditions

- Control-plane running in DEV_MODE
- LLM API keys configured via environment variables
- Agent has all research capabilities enabled

## Test Data

| Field | Value |
|-------|-------|
| Agent Name | Research Agent |
| Capabilities | `web_fetch`, `stateless_todo_list`, `session_file_system` |
| User Message | Research Axum web framework. Fetch info from https://docs.rs/axum/latest/axum/ and save your findings to /research/report.md |

## Steps

1. Create research agent:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{
       "name": "Research Agent",
       "system_prompt": "You are an expert research analyst. Use web_fetch to gather information and save findings to files using the filesystem.",
       "capabilities": [{"ref": "web_fetch"}, {"ref": "stateless_todo_list"}, {"ref": "session_file_system"}]
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

3. Send research request:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/sessions/{session_id}/messages" \
     -H "Content-Type: application/json" \
     -d '{
       "message": {
         "role": "user",
         "content": [{"type": "text", "text": "Research Axum web framework. Fetch info from https://docs.rs/axum/latest/axum/ and save your findings to /research/report.md"}]
       }
     }'
   ```

4. Wait for completion (60-90 seconds for multi-step workflow).

5. Retrieve all data for assertions:
   ```bash
   # Events
   curl -s "http://localhost:9300/api/v1/sessions/{session_id}/events"

   # Files
   curl -s "http://localhost:9300/api/v1/sessions/{session_id}/fs?recursive=true"

   # Report content
   curl -s "http://localhost:9300/api/v1/sessions/{session_id}/fs/research/report.md"
   ```

## Expected Result

### Response Assertions

| Check | Expected |
|-------|----------|
| Agent message exists | `events` contains `type: "output.message.completed"` |
| Response has content | `message.content[0].text` is non-empty |
| Response is relevant | Text mentions "Axum" |
| Model metadata present | `message.metadata.model` exists |

### Event Lifecycle Assertions

| Check | Expected |
|-------|----------|
| Turn started | `turn.started` event exists |
| Reasoning completed | `reason.completed` with `success: true` |
| Turn completed | `turn.completed` event exists |
| Session returned to idle | `session.idled` event exists |

### Tool Usage Assertions

| Check | Expected |
|-------|----------|
| Tools were called | `reason.completed.has_tool_calls: true` |
| Web fetch used | `tool.called` event with `tool_name: "web_fetch"` |
| File write used | `tool.called` event with `tool_name` containing "file" or "write" |

### File System Assertions

| Check | Expected |
|-------|----------|
| Files created | `GET /fs?recursive=true` returns non-empty list |
| Report exists | `GET /fs/research/report.md` returns status 200 |
| Report has content | Response `content` field is non-empty |
| Report is relevant | Content mentions "Axum" |

## Validation Commands

```bash
# Assert: Agent responded
curl -s ".../events" | jq '[.data[] | select(.type == "output.message.completed")] | length > 0'

# Assert: Turn completed successfully
curl -s ".../events" | jq '[.data[] | select(.type == "turn.completed")] | length > 0'

# Assert: Tools were used
curl -s ".../events" | jq '[.data[] | select(.type == "tool.called")] | length > 0'

# Assert: Files exist
curl -s ".../fs?recursive=true" | jq '.files | length > 0'

# Assert: Report exists and has content
curl -s ".../fs/research/report.md" | jq '.content | length > 0'
```
