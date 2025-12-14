name: API & System Smoke Testing
description: Comprehensive smoke testing for API, database, Docker, and system integration
----
# Smoke Test Guide

This document provides step-by-step instructions for smoke testing the Everruns system to verify all components work correctly.

## Prerequisites

- Docker and Docker Compose installed
- Rust toolchain (see [rust-toolchain.toml](rust-toolchain.toml))
- `jq` installed for JSON parsing (`brew install jq` on macOS)
- OpenAI API key (optional, for full LLM testing)

## Quick Smoke Test (Automated)

Run the automated API smoke test script:

```bash
# Start services
./scripts/dev.sh start

# Run migrations
./scripts/dev.sh migrate

# Start API (in one terminal)
./scripts/dev.sh api

# Run smoke tests (in another terminal)
./scripts/smoke-test.sh
# Or use dev.sh
./scripts/dev.sh smoke-test
```

Expected output: All tests should pass with ✅ indicators.

### With UI Testing

To also verify the Management UI works:

```bash
# Install UI dependencies (first time)
./scripts/dev.sh ui-install

# Start UI (in another terminal)
./scripts/dev.sh ui

# Run smoke tests with UI
./scripts/smoke-test.sh --with-ui
# Or use dev.sh
./scripts/dev.sh smoke-test --with-ui
```

Expected output: All API and UI tests should pass with ✅ indicators.

---

## Manual Smoke Test (Step-by-Step)

### 1. Environment Setup

```bash
# Clean start
./scripts/dev.sh clean

# Start infrastructure
./scripts/dev.sh start

# Verify services are running
docker ps
```

Expected containers:
- `everruns-postgres` (port 5432)
- `everruns-temporal` (port 7233)
- `everruns-temporal-ui` (port 8080)

### 2. Database Migrations

```bash
# Run migrations
./scripts/dev.sh migrate

# Verify migrations applied
docker exec everruns-postgres psql -U everruns -d everruns -c "\dt"
```

Expected tables:
- `agents`
- `agent_versions`
- `threads`
- `messages`
- `runs`
- `actions`
- `run_events`
- `_sqlx_migrations`

### 3. Build & Test

```bash
# Build all crates
cargo build --all

# Run unit tests
cargo test --lib

# Run clippy
cargo clippy --all-targets -- -D warnings

# Check formatting
cargo fmt --all -- --check
```

Expected:
- ✅ Build succeeds
- ✅ All 15+ unit tests pass
- ✅ No clippy warnings
- ✅ Code is properly formatted

### 4. Start API Server

```bash
# Terminal 1: Start API
./scripts/dev.sh api

# Or run directly
DATABASE_URL=postgres://everruns:everruns@localhost:5432/everruns cargo run -p everruns-api
```

Expected output:
```
INFO everruns_api: Starting Everruns API server...
INFO everruns_api: Server listening on 0.0.0.0:9000
```

### 5. Health Check

```bash
# Test health endpoint
curl http://localhost:9000/health | jq
```

Expected response:
```json
{
  "status": "ok",
  "version": "0.1.0"
}
```

### 6. OpenAPI Documentation

Open in browser:
- Swagger UI: http://localhost:9000/swagger-ui/
- OpenAPI spec: http://localhost:9000/api-doc/openapi.json

Verify:
- ✅ All endpoints documented
- ✅ Schemas visible
- ✅ Can interact with endpoints

### 7. Agent CRUD Operations

```bash
# Create agent
AGENT_RESPONSE=$(curl -s -X POST http://localhost:9000/v1/agents \
  -H 'Content-Type: application/json' \
  -d '{
    "name": "Smoke Test Agent",
    "description": "Testing agent creation",
    "default_model_id": "gpt-5.1"
  }')

AGENT_ID=$(echo "$AGENT_RESPONSE" | jq -r '.id')
echo "Created agent: $AGENT_ID"

# Get agent
curl -s "http://localhost:9000/v1/agents/$AGENT_ID" | jq

# List agents
curl -s http://localhost:9000/v1/agents | jq

# Update agent
curl -s -X PATCH "http://localhost:9000/v1/agents/$AGENT_ID" \
  -H 'Content-Type: application/json' \
  -d '{"name": "Updated Smoke Test Agent"}' | jq

# Create agent version
VERSION_RESPONSE=$(curl -s -X POST "http://localhost:9000/v1/agents/$AGENT_ID/versions" \
  -H 'Content-Type: application/json' \
  -d '{
    "definition": {
      "system": "You are a helpful assistant for testing.",
      "llm": {
        "temperature": 0.7,
        "max_tokens": 100
      },
      "tools": []
    }
  }')

echo "$VERSION_RESPONSE" | jq
```

Expected:
- ✅ Agent created with valid UUID
- ✅ Agent retrieved successfully
- ✅ Agent appears in list
- ✅ Agent updated successfully
- ✅ Agent version created (version: 1)

### 8. Thread Operations

```bash
# Create thread
THREAD_RESPONSE=$(curl -s -X POST http://localhost:9000/v1/threads \
  -H 'Content-Type: application/json' \
  -d '{}')

THREAD_ID=$(echo "$THREAD_RESPONSE" | jq -r '.id')
echo "Created thread: $THREAD_ID"

# Get thread
curl -s "http://localhost:9000/v1/threads/$THREAD_ID" | jq

# Add message to thread
curl -s -X POST "http://localhost:9000/v1/threads/$THREAD_ID/messages" \
  -H 'Content-Type: application/json' \
  -d '{
    "role": "user",
    "content": "Hello, this is a test message."
  }' | jq
```

Expected:
- ✅ Thread created with valid UUID
- ✅ Thread retrieved successfully
- ✅ Message added to thread

### 9. Run Execution (Without LLM)

```bash
# Create run (will execute without actual LLM if OPENAI_API_KEY not set)
RUN_RESPONSE=$(curl -s -X POST http://localhost:9000/v1/runs \
  -H 'Content-Type: application/json' \
  -d "{
    \"agent_id\": \"$AGENT_ID\",
    \"agent_version\": 1,
    \"thread_id\": \"$THREAD_ID\"
  }")

RUN_ID=$(echo "$RUN_RESPONSE" | jq -r '.id')
echo "Created run: $RUN_ID"

# Wait for workflow execution
sleep 3

# Check run status
curl -s "http://localhost:9000/v1/runs/$RUN_ID" | jq

# Get run events
curl -s "http://localhost:9000/v1/runs/$RUN_ID/events" \
  -H 'Accept: text/event-stream' | head -20
```

Expected:
- ✅ Run created with valid UUID
- ✅ Run status transitions: pending → running → completed (or error if no API key)
- ✅ Events emitted: `RUN_STARTED`, `RUN_FINISHED` or `RUN_ERROR`

### 10. Run Cancellation

```bash
# Create another run
RUN2_RESPONSE=$(curl -s -X POST http://localhost:9000/v1/runs \
  -H 'Content-Type: application/json' \
  -d "{
    \"agent_id\": \"$AGENT_ID\",
    \"agent_version\": 1,
    \"thread_id\": \"$THREAD_ID\"
  }")

RUN2_ID=$(echo "$RUN2_RESPONSE" | jq -r '.id')

# Cancel immediately
curl -s -X POST "http://localhost:9000/v1/runs/$RUN2_ID/cancel" | jq

# Check status
curl -s "http://localhost:9000/v1/runs/$RUN2_ID" | jq
```

Expected:
- ✅ Run cancellation succeeds
- ✅ Run status shows cancelled

### 11. Integration Tests

```bash
# Run integration tests (requires API running)
cargo test --test integration_test -- --ignored
```

Expected:
- ✅ All integration tests pass

### 12. Example Programs

```bash
# Run example: create agent
cargo run --example create_agent
```

Expected:
- ✅ Example runs successfully
- ✅ Agent created and printed

---

## Full LLM Smoke Test (With OpenAI)

**Prerequisites**: Set `OPENAI_API_KEY` environment variable

```bash
# Export API key
export OPENAI_API_KEY=sk-...

# Restart API with key
./scripts/dev.sh api
```

### Test LLM Call

```bash
# Create agent with LLM config
AGENT_RESPONSE=$(curl -s -X POST http://localhost:9000/v1/agents \
  -H 'Content-Type: application/json' \
  -d '{
    "name": "LLM Test Agent",
    "default_model_id": "gpt-5.1"
  }')

AGENT_ID=$(echo "$AGENT_RESPONSE" | jq -r '.id')

# Create version with system prompt
curl -s -X POST "http://localhost:9000/v1/agents/$AGENT_ID/versions" \
  -H 'Content-Type: application/json' \
  -d '{
    "definition": {
      "system": "You are a helpful assistant. Always respond in one short sentence.",
      "llm": {
        "temperature": 0.7,
        "max_tokens": 50
      }
    }
  }' | jq

# Create thread with message
THREAD_RESPONSE=$(curl -s -X POST http://localhost:9000/v1/threads \
  -H 'Content-Type: application/json' -d '{}')
THREAD_ID=$(echo "$THREAD_RESPONSE" | jq -r '.id')

curl -s -X POST "http://localhost:9000/v1/threads/$THREAD_ID/messages" \
  -H 'Content-Type: application/json' \
  -d '{
    "role": "user",
    "content": "What is 2+2?"
  }' | jq

# Create run
RUN_RESPONSE=$(curl -s -X POST http://localhost:9000/v1/runs \
  -H 'Content-Type: application/json' \
  -d "{
    \"agent_id\": \"$AGENT_ID\",
    \"agent_version\": 1,
    \"thread_id\": \"$THREAD_ID\"
  }")

RUN_ID=$(echo "$RUN_RESPONSE" | jq -r '.id')

# Watch SSE stream (in separate terminal)
curl -N "http://localhost:9000/v1/runs/$RUN_ID/events" \
  -H 'Accept: text/event-stream'

# Wait and check final status
sleep 5
curl -s "http://localhost:9000/v1/runs/$RUN_ID" | jq
```

Expected:
- ✅ Run completes successfully
- ✅ SSE stream shows: `RUN_STARTED`, `TEXT_MESSAGE_START`, `TEXT_MESSAGE_DELTA` (multiple), `TEXT_MESSAGE_END`, `RUN_FINISHED`
- ✅ LLM response is coherent (e.g., "2+2 equals 4")

### Test Tool Calling

```bash
# Create agent with webhook tool
AGENT_RESPONSE=$(curl -s -X POST http://localhost:9000/v1/agents \
  -H 'Content-Type: application/json' \
  -d '{
    "name": "Tool Test Agent",
    "default_model_id": "gpt-5.1"
  }')

AGENT_ID=$(echo "$AGENT_RESPONSE" | jq -r '.id')

# Create version with tool
curl -s -X POST "http://localhost:9000/v1/agents/$AGENT_ID/versions" \
  -H 'Content-Type: application/json' \
  -d '{
    "definition": {
      "system": "You are a helpful assistant with access to tools.",
      "tools": [
        {
          "type": "webhook",
          "name": "get_weather",
          "description": "Get current weather for a location",
          "parameters": {
            "type": "object",
            "properties": {
              "location": {
                "type": "string",
                "description": "City name"
              }
            },
            "required": ["location"]
          },
          "url": "https://httpbin.org/post",
          "method": "POST",
          "headers": {},
          "timeout_secs": 30,
          "max_retries": 3,
          "policy": "auto"
        }
      ]
    }
  }' | jq

# Create run asking for weather
THREAD_RESPONSE=$(curl -s -X POST http://localhost:9000/v1/threads -H 'Content-Type: application/json' -d '{}')
THREAD_ID=$(echo "$THREAD_RESPONSE" | jq -r '.id')

curl -s -X POST "http://localhost:9000/v1/threads/$THREAD_ID/messages" \
  -H 'Content-Type: application/json' \
  -d '{"role": "user", "content": "What is the weather in San Francisco?"}' | jq

RUN_RESPONSE=$(curl -s -X POST http://localhost:9000/v1/runs \
  -H 'Content-Type: application/json' \
  -d "{\"agent_id\": \"$AGENT_ID\", \"agent_version\": 1, \"thread_id\": \"$THREAD_ID\"}")

RUN_ID=$(echo "$RUN_RESPONSE" | jq -r '.id')

# Watch events
curl -N "http://localhost:9000/v1/runs/$RUN_ID/events" -H 'Accept: text/event-stream'
```

Expected:
- ✅ LLM decides to call `get_weather` tool
- ✅ Tool call executed (webhook to httpbin.org)
- ✅ Events show: `TOOL_CALL_START`, `TOOL_CALL_RESULT`
- ✅ LLM receives tool result and responds

---

## Docker Smoke Test

### Build Docker Images

```bash
# Build API image
docker build -f crates/everruns-api/Dockerfile -t everruns-api:test .

# Build Worker image
docker build -f crates/everruns-worker/Dockerfile -t everruns-worker:test .

# Verify images
docker images | grep everruns
```

Expected:
- ✅ Both images build successfully
- ✅ Images are reasonably sized (< 500MB each)

### Run with Docker Compose

```bash
# Stop local services
./scripts/dev.sh stop

# Start everything with Docker Compose
cd harness
docker compose up --build

# In another terminal, test health
curl http://localhost:9000/health | jq

# Create an agent
curl -X POST http://localhost:9000/v1/agents \
  -H 'Content-Type: application/json' \
  -d '{"name": "Docker Test", "default_model_id": "gpt-5.1"}' | jq

# Stop
docker compose down
```

Expected:
- ✅ All services start successfully
- ✅ API responds to health check
- ✅ Agent creation works

---

## CI/CD Smoke Test

```bash
# Run local CI checks
./scripts/dev.sh check
```

Expected:
- ✅ Format check passes
- ✅ Clippy passes
- ✅ Tests pass
- ✅ Build succeeds

---

## Temporal UI Verification

Open http://localhost:8080 in browser.

Verify:
- ✅ Temporal UI loads
- ✅ Workflows visible for completed runs
- ✅ Workflow history shows activities

---

## Troubleshooting

### API won't start

```bash
# Check if port 9000 is in use
lsof -i :9000

# Check database connection
docker exec everruns-postgres psql -U everruns -d everruns -c "SELECT 1;"

# Check logs
./scripts/dev.sh api 2>&1 | tee api.log
```

### Migrations fail

```bash
# Reset database
docker compose -f harness/docker-compose.yml down -v
docker compose -f harness/docker-compose.yml up -d postgres

# Retry migrations
./scripts/dev.sh migrate
```

### Tests fail

```bash
# Clean build
cargo clean

# Rebuild
cargo build --all

# Run with output
cargo test --lib -- --nocapture
```

### Docker build fails

```bash
# Check Rust version in Dockerfile matches rust-toolchain.toml
cat rust-toolchain.toml

# Build with verbose output
docker build -f crates/everruns-api/Dockerfile --progress=plain .
```

---

## UI Smoke Test

### Prerequisites

- Node.js 18+ installed
- npm installed

### Setup

```bash
# Install UI dependencies
cd apps/ui
npm install

# Or use dev script
./scripts/dev.sh ui-install
```

### Start UI

```bash
# Start UI development server
./scripts/dev.sh ui

# Or directly
cd apps/ui && npm run dev
```

Expected output:
```
▲ Next.js 16.x
- Local: http://localhost:3000
```

### Verify UI Pages

With API running, open in browser:

| Page | URL | Expected |
|------|-----|----------|
| Dashboard | http://localhost:3000/dashboard | Stats, recent runs, agents list |
| Agents | http://localhost:3000/agents | Agent cards grid |
| Create Agent | http://localhost:3000/agents/new | Form to create agent |
| Runs | http://localhost:3000/runs | Run table with filters |
| Chat | http://localhost:3000/chat | Agent selector, chat interface |

### End-to-End UI Test

1. Open http://localhost:3000
2. Navigate to Agents → Create new agent
3. Fill in name, select model, click Create
4. Click "New Version" → Add system prompt → Create
5. Navigate to Chat
6. Select the agent you created
7. Type a message and send
8. Verify:
   - ✅ Thread created
   - ✅ Run started
   - ✅ Response streams in real-time
   - ✅ Tool calls displayed (if any)

### Automated UI Test

```bash
# With API and UI running
./scripts/smoke-test.sh --with-ui
# Or use dev.sh
./scripts/dev.sh smoke-test --with-ui
```

This tests:
- ✅ UI availability (root redirect)
- ✅ Dashboard page loads
- ✅ Agents page loads
- ✅ Runs page loads
- ✅ Chat page loads
- ✅ Agent detail page loads
- ✅ Run detail page loads
- ✅ Thread detail page loads

---

## Success Criteria

All smoke tests pass when:

- ✅ All Docker services start and stay healthy
- ✅ Database migrations apply cleanly
- ✅ Cargo build/test/clippy all pass
- ✅ API health endpoint responds
- ✅ Agent CRUD operations work
- ✅ Thread CRUD operations work
- ✅ Run creation and execution work
- ✅ Events are persisted and retrievable via SSE
- ✅ Integration tests pass
- ✅ Docker images build successfully
- ✅ UI pages load and render correctly
- ✅ UI can create agents, threads, and runs
- ✅ UI shows real-time SSE events
- ✅ (Optional) LLM calls work with OpenAI API key
- ✅ (Optional) Tool calling works end-to-end

---

## Cleanup

```bash
# Stop all services
./scripts/dev.sh stop

# Remove volumes (WARNING: deletes all data)
./scripts/dev.sh clean

# Or manually
docker compose -f harness/docker-compose.yml down -v
```
