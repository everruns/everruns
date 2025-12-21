---
name: smoke-tests
description: Run API and UI smoke tests to verify the Everruns system works correctly. Use this skill when you need to test system functionality after changes, verify deployments, or troubleshoot issues. Supports both Docker-based and no-Docker environments.
---

# Smoke Tests

Comprehensive smoke testing for API, UI, database, and system integration.

## Prerequisites

Start the development environment before running tests:

```bash
./scripts/dev.sh start-all
```

## Test Checklist

Run these tests in order. Each test builds on the previous one.

### API Tests

#### 1. Health Check
```bash
curl -s http://localhost:9000/health | jq
```
Expected: `{"status": "ok", "version": "...", "runner_mode": "...", "auth_mode": "..."}`

#### 1.5. Authentication Config
```bash
curl -s http://localhost:9000/v1/auth/config | jq
```
Expected: `{"mode": "...", "passwordEnabled": ..., "oauthProviders": [...], "signupEnabled": ...}`

#### 1.6. Authentication Flow (when AUTH_MODE=admin or AUTH_MODE=full)
```bash
# Login (skip if AUTH_MODE=none)
LOGIN_RESPONSE=$(curl -s -X POST http://localhost:9000/v1/auth/login \
  -H "Content-Type: application/json" \
  -d '{"email": "'$AUTH_ADMIN_EMAIL'", "password": "'$AUTH_ADMIN_PASSWORD'"}')
ACCESS_TOKEN=$(echo $LOGIN_RESPONSE | jq -r '.access_token')
echo "Login successful: token starts with $(echo $ACCESS_TOKEN | cut -c1-20)..."

# Get current user
curl -s http://localhost:9000/v1/auth/me \
  -H "Authorization: Bearer $ACCESS_TOKEN" | jq
```
Expected: User object with email and name

#### 1.7. API Key Authentication (when AUTH_MODE != none)
```bash
# Create API key
API_KEY_RESPONSE=$(curl -s -X POST http://localhost:9000/v1/auth/api-keys \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name": "smoke-test-key"}')
API_KEY=$(echo $API_KEY_RESPONSE | jq -r '.key')
echo "API Key created: $(echo $API_KEY | cut -c1-12)..."

# Use API key for authentication
curl -s http://localhost:9000/v1/auth/me \
  -H "Authorization: $API_KEY" | jq
```
Expected: Same user object as with JWT

#### 2. Create Agent
```bash
AGENT=$(curl -s -X POST http://localhost:9000/v1/agents \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Test Agent",
    "system_prompt": "You are a helpful assistant created for smoke testing.",
    "description": "Created by smoke test"
  }')
AGENT_ID=$(echo $AGENT | jq -r '.id')
echo "Agent ID: $AGENT_ID"
```
Expected: Valid UUID returned

#### 3. Get Agent
```bash
curl -s "http://localhost:9000/v1/agents/$AGENT_ID" | jq
```
Expected: Agent object with matching ID

#### 4. Update Agent
```bash
curl -s -X PATCH "http://localhost:9000/v1/agents/$AGENT_ID" \
  -H "Content-Type: application/json" \
  -d '{"name": "Updated Test Agent"}' | jq
```
Expected: Updated agent with new name

#### 5. List Agents
```bash
curl -s http://localhost:9000/v1/agents | jq '.data | length'
```
Expected: At least 1 agent

#### 6. Create Session
```bash
SESSION=$(curl -s -X POST "http://localhost:9000/v1/agents/$AGENT_ID/sessions" \
  -H "Content-Type: application/json" \
  -d '{"title": "Test Session"}')
SESSION_ID=$(echo $SESSION | jq -r '.id')
echo "Session ID: $SESSION_ID"
```
Expected: Valid UUID returned

#### 7. Get Session
```bash
curl -s "http://localhost:9000/v1/agents/$AGENT_ID/sessions/$SESSION_ID" | jq
```
Expected: Session object with matching ID

#### 8. Send User Message (Create Message)
```bash
MESSAGE=$(curl -s -X POST "http://localhost:9000/v1/agents/$AGENT_ID/sessions/$SESSION_ID/messages" \
  -H "Content-Type: application/json" \
  -d '{
    "role": "user",
    "content": {"text": "Hello, world!"}
  }')
MESSAGE_ID=$(echo $MESSAGE | jq -r '.id')
echo "Message ID: $MESSAGE_ID"
```
Expected: Valid UUID returned, role "user"

#### 9. List Messages
```bash
curl -s "http://localhost:9000/v1/agents/$AGENT_ID/sessions/$SESSION_ID/messages" | jq '.data | length'
```
Expected: At least 1 message

#### 9.5. Verify Workflow Execution (Temporal)
After sending a user message, verify the agent workflow executed correctly:
```bash
# Wait for workflow to complete (5-10 seconds)
sleep 10

# Check session status (should be 'pending' after workflow completes)
curl -s "http://localhost:9000/v1/agents/$AGENT_ID/sessions/$SESSION_ID" | jq '.status'
```
Expected: `"pending"` (workflow completed)

```bash
# Check for assistant response
curl -s "http://localhost:9000/v1/agents/$AGENT_ID/sessions/$SESSION_ID/messages" | jq '.data[] | select(.role == "assistant") | .content.text'
```
Expected: Non-empty assistant response text

```bash
# Verify workflow type in worker logs (if running locally)
grep "agent_workflow" /tmp/worker.log | head -3
```
Expected: Logs showing `workflow_type: "agent_workflow"` and activities like `load-agent`, `call-model`

#### 10. List Sessions
```bash
curl -s "http://localhost:9000/v1/agents/$AGENT_ID/sessions" | jq '.data | length'
```
Expected: At least 1 session

#### 11. OpenAPI Spec
```bash
curl -s http://localhost:9000/api-doc/openapi.json | jq '.info.title'
```
Expected: "Everruns API"

### Tool Calling Tests

These tests verify the agent tool calling functionality via the API. They require the API and worker to be running.

#### 12. Single Tool Test (TestMath - Add)
Create an agent with test_math capability and test a single tool call:
```bash
# Create agent with test_math capability
MATH_AGENT=$(curl -s -X POST http://localhost:9000/v1/agents \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Math Agent",
    "system_prompt": "You are a math assistant. Use the add tool to add numbers.",
    "description": "Tests single tool calling"
  }')
MATH_AGENT_ID=$(echo $MATH_AGENT | jq -r '.id')

# Set test_math capability
curl -s -X PUT "http://localhost:9000/v1/agents/$MATH_AGENT_ID/capabilities" \
  -H "Content-Type: application/json" \
  -d '{"capabilities": ["test_math"]}' | jq

# Create session
MATH_SESSION=$(curl -s -X POST "http://localhost:9000/v1/agents/$MATH_AGENT_ID/sessions" \
  -H "Content-Type: application/json" \
  -d '{"title": "Single Tool Test"}')
MATH_SESSION_ID=$(echo $MATH_SESSION | jq -r '.id')

# Send message requiring tool use
curl -s -X POST "http://localhost:9000/v1/agents/$MATH_AGENT_ID/sessions/$MATH_SESSION_ID/messages" \
  -H "Content-Type: application/json" \
  -d '{"role": "user", "content": {"text": "What is 5 plus 3?"}}'

# Wait for workflow completion
sleep 15

# Check for tool results in messages
curl -s "http://localhost:9000/v1/agents/$MATH_AGENT_ID/sessions/$MATH_SESSION_ID/messages" | \
  jq '.data[] | select(.tool_results != null) | .tool_results'
```
Expected: Tool result containing `"result": 8`

#### 13. Multiple Tools Test (Math - Multiple Operations)
Test agent using multiple different tools in one conversation:
```bash
# Use the same math agent
curl -s -X POST "http://localhost:9000/v1/agents/$MATH_AGENT_ID/sessions/$MATH_SESSION_ID/messages" \
  -H "Content-Type: application/json" \
  -d '{"role": "user", "content": {"text": "Calculate 10 minus 4, then multiply the result by 2"}}'

# Wait for workflow
sleep 20

# Check for multiple tool calls
curl -s "http://localhost:9000/v1/agents/$MATH_AGENT_ID/sessions/$MATH_SESSION_ID/messages" | \
  jq '[.data[] | select(.tool_calls != null) | .tool_calls[]] | length'
```
Expected: Multiple tool calls (subtract and multiply)

#### 14. Multi-Step Agent Test (TestWeather Forecast)
Test agent that makes multiple tool calls across iterations:
```bash
# Create agent with test_weather capability
WEATHER_AGENT=$(curl -s -X POST http://localhost:9000/v1/agents \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Weather Agent",
    "system_prompt": "You are a weather assistant. Get weather and forecasts for locations.",
    "description": "Tests multi-step tool calling"
  }')
WEATHER_AGENT_ID=$(echo $WEATHER_AGENT | jq -r '.id')

# Set test_weather capability
curl -s -X PUT "http://localhost:9000/v1/agents/$WEATHER_AGENT_ID/capabilities" \
  -H "Content-Type: application/json" \
  -d '{"capabilities": ["test_weather"]}'

# Create session
WEATHER_SESSION=$(curl -s -X POST "http://localhost:9000/v1/agents/$WEATHER_AGENT_ID/sessions" \
  -H "Content-Type: application/json" \
  -d '{"title": "Multi-Step Test"}')
WEATHER_SESSION_ID=$(echo $WEATHER_SESSION | jq -r '.id')

# Request that requires weather + forecast
curl -s -X POST "http://localhost:9000/v1/agents/$WEATHER_AGENT_ID/sessions/$WEATHER_SESSION_ID/messages" \
  -H "Content-Type: application/json" \
  -d '{"role": "user", "content": {"text": "Get the current weather in New York and also the 5-day forecast"}}'

# Wait for workflow
sleep 20

# Check for both tool calls
curl -s "http://localhost:9000/v1/agents/$WEATHER_AGENT_ID/sessions/$WEATHER_SESSION_ID/messages" | \
  jq '.data[] | select(.tool_calls != null) | .tool_calls[] | .name'
```
Expected: Both `get_weather` and `get_forecast` tool calls

#### 15. Parallel Tool Execution Test (Multiple Cities)
Test agent executing multiple tool calls in parallel:
```bash
# Request weather for multiple cities (should trigger parallel execution)
curl -s -X POST "http://localhost:9000/v1/agents/$WEATHER_AGENT_ID/sessions/$WEATHER_SESSION_ID/messages" \
  -H "Content-Type: application/json" \
  -d '{"role": "user", "content": {"text": "Get the current weather for New York, London, and Tokyo at the same time"}}'

# Wait for workflow
sleep 25

# Check for parallel tool calls (same assistant message with multiple tool calls)
curl -s "http://localhost:9000/v1/agents/$WEATHER_AGENT_ID/sessions/$WEATHER_SESSION_ID/messages" | \
  jq '.data[] | select(.tool_calls != null) | select(.tool_calls | length > 1) | {msg_id: .id, tool_count: (.tool_calls | length)}'
```
Expected: Assistant message with multiple tool calls (3 get_weather calls)

#### 16. Combined Capabilities Test (TestMath + TestWeather)
Test agent with multiple capabilities:
```bash
# Create agent with both capabilities
COMBO_AGENT=$(curl -s -X POST http://localhost:9000/v1/agents \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Combo Agent",
    "system_prompt": "You are a helpful assistant with math and weather tools.",
    "description": "Tests multiple capabilities"
  }')
COMBO_AGENT_ID=$(echo $COMBO_AGENT | jq -r '.id')

# Set both capabilities
curl -s -X PUT "http://localhost:9000/v1/agents/$COMBO_AGENT_ID/capabilities" \
  -H "Content-Type: application/json" \
  -d '{"capabilities": ["test_math", "test_weather"]}'

# Create session
COMBO_SESSION=$(curl -s -X POST "http://localhost:9000/v1/agents/$COMBO_AGENT_ID/sessions" \
  -H "Content-Type: application/json" \
  -d '{"title": "Combo Capability Test"}')
COMBO_SESSION_ID=$(echo $COMBO_SESSION | jq -r '.id')

# Request using both capability types
curl -s -X POST "http://localhost:9000/v1/agents/$COMBO_AGENT_ID/sessions/$COMBO_SESSION_ID/messages" \
  -H "Content-Type: application/json" \
  -d '{"role": "user", "content": {"text": "Get the temperature in Tokyo, then add 10 to it"}}'

# Wait for workflow
sleep 20

# Check for both tool types
curl -s "http://localhost:9000/v1/agents/$COMBO_AGENT_ID/sessions/$COMBO_SESSION_ID/messages" | \
  jq '.data[] | select(.tool_calls != null) | .tool_calls[] | .name' | sort | uniq
```
Expected: Both weather and math tool calls

#### 17. Tool Error Handling Test (Division by Zero)
Test that tool errors are handled correctly:
```bash
# Request division by zero
curl -s -X POST "http://localhost:9000/v1/agents/$MATH_AGENT_ID/sessions/$MATH_SESSION_ID/messages" \
  -H "Content-Type: application/json" \
  -d '{"role": "user", "content": {"text": "Divide 10 by 0"}}'

# Wait for workflow
sleep 15

# Check for tool error in results
curl -s "http://localhost:9000/v1/agents/$MATH_AGENT_ID/sessions/$MATH_SESSION_ID/messages" | \
  jq '.data[] | select(.tool_results != null) | .tool_results[] | select(.error != null) | .error'
```
Expected: Tool error message about division by zero

#### Automated Tool Calling Tests

Run all tool calling tests automatically:
```bash
./.claude/skills/smoke-tests/scripts/tool-calling-tests.sh
```

Options:
- `--api-url URL` - Custom API URL (default: http://localhost:9000)
- `--verbose` - Show detailed output
- `--skip-cleanup` - Don't delete test agents after tests

Example:
```bash
# Run with verbose output
./.claude/skills/smoke-tests/scripts/tool-calling-tests.sh --verbose

# Run against different API URL
./.claude/skills/smoke-tests/scripts/tool-calling-tests.sh --api-url http://localhost:9000
```

### UI Tests

Run these after API tests pass. Requires UI running (`./scripts/dev.sh ui`).

#### 1. UI Availability
```bash
curl -s -o /dev/null -w "%{http_code}" http://localhost:9100
```
Expected: 200 or 307

#### 2. Dashboard Page
```bash
curl -s -o /dev/null -w "%{http_code}" http://localhost:9100/dashboard
```
Expected: 200

#### 3. Agents Page
```bash
curl -s -o /dev/null -w "%{http_code}" http://localhost:9100/agents
```
Expected: 200

#### 4. New Agent Page
```bash
curl -s -o /dev/null -w "%{http_code}" http://localhost:9100/agents/new
```
Expected: 200

#### 5. Agent Detail Page
```bash
curl -s -o /dev/null -w "%{http_code}" "http://localhost:9100/agents/$AGENT_ID"
```
Expected: 200

#### 6. Session Detail Page
```bash
curl -s -o /dev/null -w "%{http_code}" "http://localhost:9100/agents/$AGENT_ID/sessions/$SESSION_ID"
```
Expected: 200

## No-Docker Mode

For environments without Docker (Cloud Agent, CI):

```bash
./.claude/skills/smoke-tests/scripts/run-no-docker.sh
```

This script:
1. Detects or installs PostgreSQL (supports pre-installed versions via `pg_ctlcluster`)
2. Installs Temporal CLI from GitHub releases
3. Starts local PostgreSQL cluster and Temporal dev server
4. Runs database migrations
5. Starts API server AND Temporal worker (both required for workflow execution)
6. Keeps services running until Ctrl+C

**Important**: The Temporal worker is required for workflow execution. Without it, sending messages won't trigger LLM responses.

### Scripts

| Script | Description |
|--------|-------------|
| `scripts/run-no-docker.sh` | Entry point for no-Docker environments |
| `scripts/_setup-postgres.sh` | PostgreSQL cluster setup - auto-detects system install (internal) |
| `scripts/_setup-temporal.sh` | Temporal CLI install from GitHub releases (internal) |
| `scripts/_utils.sh` | Shared utilities and configuration (internal) |

### Log Files

| Log | Location |
|-----|----------|
| API | `/tmp/api.log` |
| Worker | `/tmp/worker.log` |
| Temporal | `/tmp/temporal.log` |
| PostgreSQL | `/tmp/pgdata/pg.log` |

## Troubleshooting

### API Issues

```bash
# Check if port 9000 is in use
lsof -i :9000

# Check database connection
docker exec everruns-postgres psql -U everruns -d everruns -c "SELECT 1;"

# View API logs
./scripts/dev.sh api 2>&1 | tee api.log
```

### Docker Issues

```bash
# Reset and restart
./scripts/dev.sh clean
./scripts/dev.sh start
./scripts/dev.sh migrate
```

### No-Docker Issues

**"OPENAI_API_KEY not set"**: Export the key before running:
```bash
export OPENAI_API_KEY=your-key
```

**"must be run as root"**: The PostgreSQL setup requires root access:
```bash
sudo ./.claude/skills/smoke-tests/scripts/run-no-docker.sh
```

**Messages sent but no assistant response**: Ensure the Temporal worker is running:
```bash
# Check if worker is running
ps aux | grep everruns-worker

# Check worker logs for errors
tail -50 /tmp/worker.log

# Manually start worker if needed
export DATABASE_URL="postgres://everruns:everruns@localhost:5432/everruns"
export TEMPORAL_ADDRESS="localhost:7233"
cargo run -p everruns-worker
```

**Network/curl issues in restricted environments**: The Temporal CLI download uses `--insecure` flag. If you still have issues, manually download:
```bash
# Direct download from GitHub
curl -L --insecure https://github.com/temporalio/cli/releases/download/v1.1.2/temporal_cli_1.1.2_linux_amd64.tar.gz -o /tmp/temporal.tar.gz
mkdir -p /tmp/temporal_extract
tar -xzf /tmp/temporal.tar.gz -C /tmp/temporal_extract
mv /tmp/temporal_extract/temporal /usr/local/bin/temporal
chmod +x /usr/local/bin/temporal
```

**PostgreSQL already running**: The script auto-detects system PostgreSQL via `pg_ctlcluster`. If you have a custom setup:
```bash
# Check what's using port 5432
lsof -i :5432

# Use system PostgreSQL
pg_ctlcluster 16 main start  # or whatever version you have
```

### Workflow Verification

To verify the full workflow cycle works:
```bash
# 1. Create agent
AGENT=$(curl -s -X POST http://localhost:9000/v1/agents \
  -H "Content-Type: application/json" \
  -d '{"name": "Test", "system_prompt": "You are helpful."}')
AGENT_ID=$(echo $AGENT | jq -r '.id')

# 2. Create session
SESSION=$(curl -s -X POST "http://localhost:9000/v1/agents/$AGENT_ID/sessions" \
  -H "Content-Type: application/json" \
  -d '{"title": "Test"}')
SESSION_ID=$(echo $SESSION | jq -r '.id')

# 3. Send message (this triggers the agent_workflow)
curl -s -X POST "http://localhost:9000/v1/agents/$AGENT_ID/sessions/$SESSION_ID/messages" \
  -H "Content-Type: application/json" \
  -d '{"role": "user", "content": {"text": "Hello!"}}'

# 4. Wait and check for response
sleep 10
curl -s "http://localhost:9000/v1/agents/$AGENT_ID/sessions/$SESSION_ID/messages" | \
  jq '.data[] | select(.role == "assistant") | .content.text'
```

Expected: An assistant message with LLM-generated text
