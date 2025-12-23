---
name: smoke-test
description: Run API and UI smoke tests to verify the Everruns system works correctly. Use this skill when you need to test system functionality after changes, verify deployments, or troubleshoot issues. Supports both Docker-based and no-Docker environments.
---

# Smoke Tests

Comprehensive smoke testing for API, UI, database, and system integration.

## Prerequisites

Start the development environment before running tests:

```bash
# From repo root - uses Docker
./scripts/dev.sh start-all
```

**Note on paths:** This document references two types of scripts:
- **Repo root scripts** (e.g., `./scripts/dev.sh`) - Run from the repository root directory
- **Skill scripts** (e.g., `run-no-docker.sh`) - Located in `.claude/skills/smoke-test/scripts/`

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

### Scenario Tests

Additional test scenarios are available in the `scenarios/` folder:

- **[Tool Calling](scenarios/tool-calling.md)** - Tests for agent tool calling functionality (TestMath, TestWeather capabilities)
- **[Task List](scenarios/task-list.md)** - Tests for task management capability (TaskList capability with write_todos tool)

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
# Run from skill scripts directory
.claude/skills/smoke-test/scripts/run-no-docker.sh

# Or from the skill scripts directory
cd .claude/skills/smoke-test/scripts && ./run-no-docker.sh
```

This script:
1. Detects or installs PostgreSQL (supports pre-installed versions via `pg_ctlcluster`)
2. Installs Temporal CLI from GitHub releases
3. Starts local PostgreSQL cluster and Temporal dev server
4. Runs database migrations
5. Starts API server AND Temporal worker (both required for workflow execution)
6. Keeps services running until Ctrl+C

**Important**: The Temporal worker is required for workflow execution. Without it, sending messages won't trigger LLM responses.

### Skill Scripts (relative to `.claude/skills/smoke-test/scripts/`)

| Script | Description |
|--------|-------------|
| `run-no-docker.sh` | Entry point for no-Docker environments |
| `_setup-postgres.sh` | PostgreSQL cluster setup - auto-detects system install (internal) |
| `_setup-temporal.sh` | Temporal CLI install from GitHub releases (internal) |
| `_utils.sh` | Shared utilities and configuration (internal) |
| `tool-calling-tests.sh` | Automated tool calling scenario tests |

### Repo Root Scripts (relative to repository root)

| Script | Description |
|--------|-------------|
| `./scripts/dev.sh` | Development environment manager (Docker-based) |
| `./scripts/seed-agents.sh` | Seed database with sample agents |

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
sudo ./scripts/run-no-docker.sh
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
