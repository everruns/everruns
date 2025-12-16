---
name: smoke-tests
description: Run API and UI smoke tests to verify the Everruns system works correctly. Use this skill when you need to test system functionality after changes, verify deployments, or troubleshoot issues. Supports both Docker-based and no-Docker environments.
---

# Smoke Tests

Comprehensive smoke testing for API, UI, database, and system integration.

## Quick Start

### With Docker (default)

```bash
# Start services, run migrations, start API
./scripts/dev.sh start-all

# In another terminal, run smoke tests
./scripts/dev.sh smoke-test           # API tests only
./scripts/dev.sh smoke-test --with-ui # API + UI tests
```

### Without Docker (Cloud Agent / CI environments)

```bash
# All-in-one: sets up PostgreSQL + Temporal locally, runs migrations, starts API, runs smoke tests
./.claude/skills/smoke-tests/scripts/run-no-docker.sh
```

## Test Checks

The smoke tests verify the following checks. Each check outputs `[ ]` (pending), `[x]` (passed), or `[!]` (failed with details).

### API Tests

| Check | Description |
|-------|-------------|
| Health endpoint | GET /health returns status "ok" |
| Create agent | POST /v1/agents creates agent with valid UUID |
| Get agent | GET /v1/agents/:id returns the agent |
| Update agent | PATCH /v1/agents/:id updates agent fields |
| List agents | GET /v1/agents returns agent list |
| Create thread | POST /v1/threads creates thread with valid UUID |
| Add message | POST /v1/threads/:id/messages adds message to thread |
| Create run | POST /v1/runs creates run and triggers workflow |
| Run status | GET /v1/runs/:id shows status transition (pending -> running -> completed/error) |
| OpenAPI spec | GET /api-doc/openapi.json returns valid spec |

### UI Tests (with --with-ui flag)

| Check | Description |
|-------|-------------|
| UI availability | Root URL responds with 200/307 |
| Dashboard page | /dashboard loads correctly |
| Agents page | /agents loads correctly |
| Runs page | /runs loads correctly |
| Chat page | /chat loads correctly |
| Agent detail | /agents/:id loads for created agent |
| Run detail | /runs/:id loads for created run |
| Thread detail | /threads/:id loads for created thread |

### Infrastructure Tests (no-Docker mode only)

| Check | Description |
|-------|-------------|
| PostgreSQL install | PostgreSQL 18 installed from PGDG |
| PostgreSQL cluster | Cluster initialized and started |
| Database setup | Database and user created |
| Temporal install | Temporal CLI installed |
| Temporal server | Dev server started on port 7233 |
| Migrations | SQLx migrations applied |
| API startup | API server started on port 9000 |

## Output Format

Smoke tests output results in a structured format:

```
[x] Health endpoint - status: ok, version: 0.1.0
[x] Create agent - id: 550e8400-e29b-41d4-a716-446655440000
[x] Get agent - name: Test Agent
[!] Run status - FAILED: expected completed, got error (timeout after 30s)
```

## Environment Requirements

### Docker Mode (default)
- Docker and Docker Compose
- Rust toolchain (see rust-toolchain.toml)
- jq for JSON parsing
- Node.js 18+ and npm (for UI tests)

### No-Docker Mode
- Root access (for PostgreSQL setup)
- Rust toolchain with sqlx-cli
- Internet access (for PostgreSQL and Temporal CLI installation)
- OPENAI_API_KEY environment variable

## Scripts

| Script | Description |
|--------|-------------|
| `scripts/run-no-docker.sh` | Main entry point for no-Docker smoke tests |
| `scripts/_setup-postgres.sh` | PostgreSQL 18 cluster setup (internal) |
| `scripts/_setup-temporal.sh` | Temporal dev server setup (internal) |
| `scripts/_utils.sh` | Shared utilities and configuration (internal) |

## Manual Testing

### API Endpoints

```bash
# Health check
curl http://localhost:9000/health | jq

# Create agent
curl -X POST http://localhost:9000/v1/agents \
  -H 'Content-Type: application/json' \
  -d '{"name": "Test Agent", "default_model_id": "gpt-5.1"}' | jq

# View Swagger UI
open http://localhost:9000/swagger-ui/
```

### UI Pages

| Page | URL |
|------|-----|
| Dashboard | http://localhost:9100/dashboard |
| Agents | http://localhost:9100/agents |
| Runs | http://localhost:9100/runs |
| Chat | http://localhost:9100/chat |

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

**PostgreSQL installation fails**: Ensure internet access to apt.postgresql.org.

## CI/CD Integration

For CI pipelines without Docker:

```yaml
- name: Run smoke tests
  run: |
    export OPENAI_API_KEY=${{ secrets.OPENAI_API_KEY }}
    sudo ./.claude/skills/smoke-tests/scripts/run-no-docker.sh
```

For Docker-based CI:

```yaml
- name: Start services
  run: ./scripts/dev.sh start-all

- name: Run smoke tests
  run: ./scripts/dev.sh smoke-test --with-ui
```
