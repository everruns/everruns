---
name: smoke-test
description: Run API and UI smoke tests to verify the Everruns system works correctly. Use this skill when you need to test system functionality after changes, verify deployments, or troubleshoot issues. Supports both Docker-based and no-Docker environments.
---

# Smoke Tests

Comprehensive smoke testing for API, UI, database, and system integration.

## IMPORTANT: Mandatory Testing Requirements

**Smoke tests are MANDATORY before completing any task that modifies code.** Do not skip or give up on smoke testing.

### Rules for Agents

1. **Never give up.** If smoke tests fail, troubleshoot the issue. Check logs, fix problems, and retry.

2. **Persist through infrastructure issues.** If dependencies fail to install, try alternative methods. If services fail to start, check ports, logs, and configurations.

3. **Report results.** You MUST report smoke test results to the user, including:
   - Which tests passed
   - Which tests failed (with error details)
   - What troubleshooting steps were attempted
   - Final status (all passing / some failing / blocked)

4. **Use the checklist.** Run through ALL tests in the "Test Checklist" section below and report on each one.

5. **Don't mark tasks complete without testing.** A PR or code change is not complete until smoke tests have been run and results reported.

### Quick Start for No-Docker Environments

```bash
# Ensure API keys are available (required for LLM calls)
export OPENAI_API_KEY="your-key"  # or ANTHROPIC_API_KEY

# Run with sudo to allow PostgreSQL setup
sudo -E .claude/skills/smoke-test/scripts/run-no-docker.sh
```

If the script fails, manually start services and run the Test Checklist below.

## Prerequisites

Start the development environment before running tests:

```bash
# From repo root - uses Docker
./scripts/dev.sh start-all
```

**Service Ports:**
- **HTTP API**: `http://localhost:9000` - REST API for clients and UI
- **gRPC Service**: `localhost:9001` - Internal worker communication (not tested directly in smoke tests)

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
    "message": {
      "content": [{"type": "text", "text": "Hello, world!"}]
    }
  }')
MESSAGE_ID=$(echo $MESSAGE | jq -r '.id')
echo "Message ID: $MESSAGE_ID"
```
Expected: Valid UUID returned, role "user"

**Note:** The message format uses `Vec<ContentPart>` for content. The `role` field defaults to `"user"` and can be omitted.

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
# Check for assistant response (content is now an array of ContentPart)
curl -s "http://localhost:9000/v1/agents/$AGENT_ID/sessions/$SESSION_ID/messages" | jq '.data[] | select(.role == "assistant") | .content[] | select(.type == "text") | .text'
```
Expected: Non-empty assistant response text

```bash
# Verify workflow type in worker logs (if running locally)
grep "agent_workflow" /tmp/worker.log | head -3
```
Expected: Logs showing `workflow_type: "agent_workflow"` and activities like `load-agent`, `call-model`

#### 9.6. Verify Events (Messages/Events Sync)
After workflow completes, verify events are created alongside messages:
```bash
# List events for the session
curl -s "http://localhost:9000/v1/agents/$AGENT_ID/sessions/$SESSION_ID/events" | jq '.data | length'
```
Expected: At least 2 events (message.user and message.agent)

```bash
# Check for message.user event
curl -s "http://localhost:9000/v1/agents/$AGENT_ID/sessions/$SESSION_ID/events" | jq '.data[] | select(.event_type == "message.user")'
```
Expected: Event with `event_type: "message.user"` and `data` containing `message_id`, `content`

```bash
# Check for message.agent event
curl -s "http://localhost:9000/v1/agents/$AGENT_ID/sessions/$SESSION_ID/events" | jq '.data[] | select(.event_type == "message.agent")'
```
Expected: Event with `event_type: "message.agent"` and `data` containing `message_id`, `role`, `content`

**Note:** The UI uses the `/sse` endpoint for real-time streaming and the `/events` endpoint for polling/listing events.

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

#### 12. LLM Providers and Models
```bash
# List LLM providers
curl -s http://localhost:9000/v1/llm-providers | jq '.data | length'
```
Expected: At least 1 provider

```bash
# List all models with profile data
curl -s http://localhost:9000/v1/llm-models | jq '.data[0]'
```
Expected: Model object with `profile` field (null for unknown models, object for known models like gpt-4o)

```bash
# Verify profile contains expected fields for known models
curl -s http://localhost:9000/v1/llm-models | jq '.data[] | select(.profile != null) | {model_id: .model_id, profile_name: .profile.name, has_cost: (.profile.cost != null)}'
```
Expected: Models like gpt-4o, claude-3-5-sonnet show profile with name and cost data

### Scenario Tests

Additional test scenarios are available in the `scenarios/` folder:

- **[Tool Calling](scenarios/tool-calling.md)** - Tests for agent tool calling functionality (TestMath, TestWeather capabilities)
- **[Task List](scenarios/task-list.md)** - Tests for task management capability (TaskList capability with write_todos tool)
- **[File System](scenarios/file-system.md)** - Tests for session virtual filesystem (create, read, update, delete files/directories)

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

For environments without Docker (Cloud Agent, CI, containers):

```bash
# Run from repo root
.claude/skills/smoke-test/scripts/run-no-docker.sh
```

This script automatically handles:
1. **Dependencies** - Installs protoc (required for building) and jq if not present
2. **PostgreSQL detection** - Supports three modes:
   - System install with `pg_ctlcluster` (Debian/Ubuntu standard)
   - Direct binaries (containers without pg_ctlcluster)
   - Fresh install from PGDG repository (if nothing found)
3. **Temporal CLI** - Downloads from GitHub releases if not installed
4. **Database setup** - Initializes cluster, creates user/database, runs migrations
5. **Application** - Builds and starts API server and Temporal worker
6. **Cleanup** - Stops all services on Ctrl+C

**Requirements**:
- Root access (for PostgreSQL initialization)
- Either `OPENAI_API_KEY` or `ANTHROPIC_API_KEY` environment variable
- Internet access (for downloading dependencies)

**Important**: The Temporal worker is required for workflow execution. Without it, sending messages won't trigger LLM responses.

### Cloud Environment Compatibility

The no-Docker mode is specifically designed for cloud agent environments like Claude Code on the web:

- **Auto-detects PostgreSQL** even without `pg_ctlcluster` command
- **Installs protoc automatically** (required for Temporal SDK)
- **Works in containers** by using direct `pg_ctl` instead of systemd
- **Supports both API keys** (`OPENAI_API_KEY` or `ANTHROPIC_API_KEY`)

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

**"OPENAI_API_KEY not set"**: Export either key before running:
```bash
export OPENAI_API_KEY=your-key
# OR
export ANTHROPIC_API_KEY=your-key
```

**"must be run as root"**: The PostgreSQL setup requires root access:
```bash
sudo .claude/skills/smoke-test/scripts/run-no-docker.sh
```

**"protoc not found" during build**: The script auto-installs protoc, but if it fails:
```bash
# Debian/Ubuntu
apt-get update && apt-get install -y protobuf-compiler

# Verify
protoc --version
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

**PostgreSQL already running**: The script auto-detects system PostgreSQL. If port 5432 is in use:
```bash
# Check what's using port 5432
lsof -i :5432

# Kill existing postgres if needed
pkill -9 postgres
```

**PostgreSQL fails to start in containers**: The script supports containers without pg_ctlcluster by using direct pg_ctl. Check logs:
```bash
cat /tmp/pgdata/pg.log
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
  -d '{"message": {"content": [{"type": "text", "text": "Hello!"}]}}'

# 4. Wait and check for response
sleep 10
curl -s "http://localhost:9000/v1/agents/$AGENT_ID/sessions/$SESSION_ID/messages" | \
  jq '.data[] | select(.role == "assistant") | .content[] | select(.type == "text") | .text'
```

Expected: An assistant message with LLM-generated text

## Manual Service Startup (Fallback)

If the `run-no-docker.sh` script fails, start services manually:

```bash
# 1. Ensure PostgreSQL is running
export PATH="$PATH:/usr/lib/postgresql/16/bin"
pg_ctl -D /tmp/pgdata -l /tmp/pgdata/pg.log start

# 2. Start Temporal dev server
temporal server start-dev --db-filename /tmp/temporal.db &> /tmp/temporal.log &

# 3. Set environment variables
export DATABASE_URL="postgres://everruns:everruns@localhost:5432/everruns"
export TEMPORAL_ADDRESS="localhost:7233"
export SECRETS_ENCRYPTION_KEY=$(openssl rand -base64 32)

# 4. Run migrations
cd /home/user/everruns
sqlx database create --database-url "$DATABASE_URL" 2>/dev/null || true
sqlx migrate run --source crates/control-plane/migrations --database-url "$DATABASE_URL"

# 5. Start API server
cargo run -p everruns-control-plane &> /tmp/api.log &

# 6. Start worker
cargo run -p everruns-worker &> /tmp/worker.log &

# 7. Wait for services
sleep 10

# 8. Run health check
curl -s http://localhost:9000/health | jq
```

## Smoke Test Results Template

When reporting smoke test results, use this format:

```
## Smoke Test Results

### Environment
- Mode: [Docker / No-Docker]
- API Key: [OpenAI / Anthropic]
- Date: [YYYY-MM-DD]

### Test Results

| Test | Status | Notes |
|------|--------|-------|
| Health Check | PASS/FAIL | |
| Auth Config | PASS/FAIL | |
| Create Agent | PASS/FAIL | |
| Get Agent | PASS/FAIL | |
| Update Agent | PASS/FAIL | |
| List Agents | PASS/FAIL | |
| Create Session | PASS/FAIL | |
| Get Session | PASS/FAIL | |
| Send Message | PASS/FAIL | |
| List Messages | PASS/FAIL | |
| Workflow Execution | PASS/FAIL | |
| Events Sync | PASS/FAIL | |
| List Sessions | PASS/FAIL | |
| OpenAPI Spec | PASS/FAIL | |
| LLM Providers | PASS/FAIL | |

### Summary
- Total: X/15 tests passing
- Blocking issues: [None / List issues]
- Action items: [None / List items]
```
