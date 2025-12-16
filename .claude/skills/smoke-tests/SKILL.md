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
Expected: `{"status": "ok", "version": "...", "runner_mode": "..."}`

#### 2. Create Harness
```bash
HARNESS=$(curl -s -X POST http://localhost:9000/v1/harnesses \
  -H "Content-Type: application/json" \
  -d '{
    "slug": "test-harness",
    "display_name": "Test Harness",
    "system_prompt": "You are a helpful assistant created for smoke testing.",
    "description": "Created by smoke test"
  }')
HARNESS_ID=$(echo $HARNESS | jq -r '.id')
echo "Harness ID: $HARNESS_ID"
```
Expected: Valid UUID returned

#### 3. Get Harness
```bash
curl -s "http://localhost:9000/v1/harnesses/$HARNESS_ID" | jq
```
Expected: Harness object with matching ID

#### 4. Update Harness
```bash
curl -s -X PATCH "http://localhost:9000/v1/harnesses/$HARNESS_ID" \
  -H "Content-Type: application/json" \
  -d '{"display_name": "Updated Test Harness"}' | jq
```
Expected: Updated harness with new display_name

#### 5. List Harnesses
```bash
curl -s http://localhost:9000/v1/harnesses | jq '.data | length'
```
Expected: At least 1 harness

#### 6. Create Session
```bash
SESSION=$(curl -s -X POST "http://localhost:9000/v1/harnesses/$HARNESS_ID/sessions" \
  -H "Content-Type: application/json" \
  -d '{"title": "Test Session"}')
SESSION_ID=$(echo $SESSION | jq -r '.id')
echo "Session ID: $SESSION_ID"
```
Expected: Valid UUID returned

#### 7. Get Session
```bash
curl -s "http://localhost:9000/v1/harnesses/$HARNESS_ID/sessions/$SESSION_ID" | jq
```
Expected: Session object with matching ID

#### 8. Send User Message (Create Event)
```bash
EVENT=$(curl -s -X POST "http://localhost:9000/v1/harnesses/$HARNESS_ID/sessions/$SESSION_ID/events" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type": "message.user",
    "data": {
      "message": {
        "role": "user",
        "content": [{"type": "text", "text": "Hello, world!"}]
      }
    }
  }')
EVENT_ID=$(echo $EVENT | jq -r '.id')
echo "Event ID: $EVENT_ID"
```
Expected: Valid UUID returned, event_type "message.user"

#### 9. List Messages
```bash
curl -s "http://localhost:9000/v1/harnesses/$HARNESS_ID/sessions/$SESSION_ID/messages" | jq '.data | length'
```
Expected: At least 1 message

#### 10. List Sessions
```bash
curl -s "http://localhost:9000/v1/harnesses/$HARNESS_ID/sessions" | jq '.data | length'
```
Expected: At least 1 session

#### 11. OpenAPI Spec
```bash
curl -s http://localhost:9000/api-doc/openapi.json | jq '.info.title'
```
Expected: "Everruns API"

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

#### 3. Harnesses Page
```bash
curl -s -o /dev/null -w "%{http_code}" http://localhost:9100/harnesses
```
Expected: 200

#### 4. New Harness Page
```bash
curl -s -o /dev/null -w "%{http_code}" http://localhost:9100/harnesses/new
```
Expected: 200

#### 5. Harness Detail Page
```bash
curl -s -o /dev/null -w "%{http_code}" "http://localhost:9100/harnesses/$HARNESS_ID"
```
Expected: 200

#### 6. Session Detail Page
```bash
curl -s -o /dev/null -w "%{http_code}" "http://localhost:9100/harnesses/$HARNESS_ID/sessions/$SESSION_ID"
```
Expected: 200

## No-Docker Mode

For environments without Docker (Cloud Agent, CI):

```bash
./.claude/skills/smoke-tests/scripts/run-no-docker.sh
```

This script:
1. Installs PostgreSQL 18 from PGDG repository
2. Installs Temporal CLI
3. Starts local PostgreSQL cluster and Temporal dev server
4. Runs database migrations
5. Starts API server
6. Runs the test checklist above

### Scripts

| Script | Description |
|--------|-------------|
| `scripts/run-no-docker.sh` | Entry point for no-Docker environments |
| `scripts/_setup-postgres.sh` | PostgreSQL 18 cluster setup (internal) |
| `scripts/_setup-temporal.sh` | Temporal dev server setup (internal) |
| `scripts/_utils.sh` | Shared utilities and configuration (internal) |

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
