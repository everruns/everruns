# Stateless Todo List Tests

Tests for the StatelessTodoList capability that enables agents to create and manage task lists.

## Prerequisites

- API running at `http://localhost:9000`
- Temporal worker running
- StatelessTodoList capability available: `stateless_todo_list`

## Test Capability

| Capability | Tools | Description |
|------------|-------|-------------|
| `stateless_todo_list` | write_todos | Create and manage task lists for tracking multi-step work (stateless, stored in conversation history) |

## Manual Tests

### 1. Create Agent with StatelessTodoList Capability

```bash
# Create agent with stateless_todo_list capability
AGENT=$(curl -s -X POST http://localhost:9000/v1/agents \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Task Manager Agent",
    "system_prompt": "You are a helpful assistant that uses task lists to track your work. When working on multi-step problems, create a todo list first.",
    "description": "Agent with task management capability",
    "capabilities": ["stateless_todo_list"]
  }')
AGENT_ID=$(echo $AGENT | jq -r '.id')
echo "Agent ID: $AGENT_ID"
```

Expected: Agent created with `stateless_todo_list` capability

### 2. Verify Capability is Assigned

```bash
# Get agent to verify stateless_todo_list is in capabilities
curl -s "http://localhost:9000/v1/agents/$AGENT_ID" | jq '.capabilities'
```

Expected: Shows `["stateless_todo_list"]` in agent capabilities

### 3. Create Session and Trigger Task List Usage

```bash
# Create session
SESSION=$(curl -s -X POST "http://localhost:9000/v1/agents/$AGENT_ID/sessions" \
  -H "Content-Type: application/json" \
  -d '{"title": "Task List Test"}')
SESSION_ID=$(echo $SESSION | jq -r '.id')
echo "Session ID: $SESSION_ID"

# Send a message that should trigger task list usage
curl -s -X POST "http://localhost:9000/v1/agents/$AGENT_ID/sessions/$SESSION_ID/messages" \
  -H "Content-Type: application/json" \
  -d '{
    "role": "user",
    "content": {"text": "Help me plan a project with these tasks: 1) Research the topic, 2) Write an outline, 3) Draft the content, 4) Review and edit"}
  }' | jq '.id'
```

Expected: Message created successfully

### 4. Wait for Workflow and Check Response

```bash
# Wait for workflow completion
sleep 15

# Check messages for write_todos tool calls
curl -s "http://localhost:9000/v1/agents/$AGENT_ID/sessions/$SESSION_ID/messages" | \
  jq '.data[] | select(.role == "assistant") | select(.content.tool_calls != null) | .content.tool_calls[] | select(.name == "write_todos")'
```

Expected: `write_todos` tool call with todos array containing:
- Tasks with `content` (imperative form)
- Tasks with `activeForm` (present continuous form)
- Tasks with `status` (pending/in_progress/completed)

### 5. Verify Tool Results

```bash
# Check for tool result messages
curl -s "http://localhost:9000/v1/agents/$AGENT_ID/sessions/$SESSION_ID/messages" | \
  jq '.data[] | select(.role == "tool") | .content'
```

Expected: Tool result showing:
- `success: true`
- `total_tasks: 4` (or similar)
- Count of pending/in_progress/completed tasks
- `todos` array with validated tasks

## Test Coverage

| Test | Description |
|------|-------------|
| Capability Assignment | StatelessTodoList capability can be assigned to agent |
| Write Todos Tool | Agent can call `write_todos` with valid task list |
| Task Validation | Tool validates content, activeForm, and status fields |
| Status Counting | Tool correctly counts pending/in_progress/completed |
| Warning Messages | Tool warns when multiple tasks are in_progress |

## Troubleshooting

### No write_todos calls in response

1. Check that the capability is set on the agent:
   ```bash
   curl -s "http://localhost:9000/v1/agents/$AGENT_ID" | jq '.capabilities'
   ```

2. Verify the system prompt includes task management instructions:
   ```bash
   curl -s "http://localhost:9000/v1/agents/$AGENT_ID" | jq '.system_prompt'
   ```

3. Check worker logs for tool execution:
   ```bash
   grep -i "write_todos" /tmp/worker.log | tail -10
   ```

### Tool validation errors

Check tool result messages for validation error details:
```bash
curl -s "http://localhost:9000/v1/agents/$AGENT_ID/sessions/$SESSION_ID/messages" | \
  jq '.data[] | select(.role == "tool") | select(.content.error != null)'
```

Common errors:
- Missing `content` field
- Missing `activeForm` field
- Invalid `status` value (must be pending/in_progress/completed)
