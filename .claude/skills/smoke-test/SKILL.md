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

## Quick Start

### Docker Mode (Recommended)

```bash
# From repo root
./scripts/dev.sh start-all
```

### No-Docker Mode

For environments without Docker (cloud agents, CI systems), use the dedicated skill:

```bash
# See .claude/skills/no-docker-setup/SKILL.md
sudo -E .claude/skills/no-docker-setup/scripts/start.sh
```

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
- **Skill scripts** (e.g., `tool-calling-tests.sh`) - Located in `.claude/skills/smoke-test/scripts/`

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

#### 9.5. Verify Workflow Execution
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

### Durable Execution Engine Tests

The `everruns-durable` crate provides a custom workflow orchestration engine. Run these tests to verify the durable execution layer.

**Note:** Phases 5-7 (Observability, Scale Testing, Integration) are TODO followups. See `specs/durable-execution-engine.md`.

#### 1. Unit Tests (No External Dependencies)
```bash
cargo test -p everruns-durable --lib
```
Expected: 91+ tests passing (workflow, activity, reliability, worker modules)

#### 2. Integration Tests (Requires PostgreSQL)
```bash
# Ensure PostgreSQL is running with test database
sudo service postgresql start || pg_ctl -D /tmp/pgdata start

# Create test database if needed
psql -U postgres -c "CREATE DATABASE everruns_test;" 2>/dev/null || true

# Run migrations on test database
DATABASE_URL="postgres://postgres:postgres@localhost/everruns_test" \
  sqlx migrate run --source crates/control-plane/migrations

# Clean test data and run integration tests
psql -U postgres -d everruns_test -c "TRUNCATE durable_workflow_instances CASCADE;"
cargo test -p everruns-durable --test postgres_integration_test -- --test-threads=1
```
Expected: 17 tests passing (workflow lifecycle, task queue, signals, workers, DLQ)

#### 3. Clippy Lints
```bash
cargo clippy -p everruns-durable -- -D warnings
```
Expected: No warnings or errors

#### Quick Durable Test Script
```bash
# One-liner to run all durable tests
cargo test -p everruns-durable --lib && \
cargo clippy -p everruns-durable -- -D warnings && \
echo "Durable unit tests and clippy passed"
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

## DEV_MODE (UI-Only Changes)

For changes that only impact the UI (no backend/API changes), DEV_MODE provides a faster testing workflow:

```bash
# Start in DEV_MODE - no Docker/PostgreSQL required
./scripts/dev.sh start-dev
```

**When to use DEV_MODE:**
- UI component changes (styling, layout, interactions)
- Frontend-only bug fixes
- UI development and iteration
- Quick visual testing

**Limitations:**
- Data is not persisted (lost on restart)
- No worker (LLM execution happens in-process)
- Not suitable for testing backend changes or full integration

For full end-to-end testing including backend changes, use the standard smoke tests below.

## Skill Scripts

| Script | Description |
|--------|-------------|
| `scripts/tool-calling-tests.sh` | Automated tool calling scenario tests |

## Repo Root Scripts

| Script | Description |
|--------|-------------|
| `./scripts/dev.sh` | Development environment manager (Docker-based) |

**Note:** Seed agents (Dad Jokes Agent, Research Agent) are seeded automatically on API startup.

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

## Smoke Test Results Template

When reporting smoke test results, use this format:

```
## Smoke Test Results

### Environment
- Mode: [Docker / No-Docker]
- API Key: [OpenAI / Anthropic]
- Date: [YYYY-MM-DD]

### API Test Results

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

### Durable Execution Engine Results

| Test | Status | Notes |
|------|--------|-------|
| Unit Tests (91+) | PASS/FAIL | |
| Integration Tests (17) | PASS/FAIL | Requires PostgreSQL |
| Clippy Lints | PASS/FAIL | |

### Summary
- API Tests: X/15 passing
- Durable Tests: X/3 passing (91 unit + 17 integration + clippy)
- Blocking issues: [None / List issues]
- Action items: [None / List items]
```
