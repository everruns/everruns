---
name: no-docker-setup
description: Run smoke tests without Docker using deterministic PostgreSQL setup. Use this skill in cloud agent environments (Claude Code on web) or CI systems without Docker. Provides predictable environment setup with fixed paths and configurations.
---

# No-Docker Setup

Deterministic environment setup for systems without Docker. Uses fixed PostgreSQL paths and always creates a fresh database cluster.

## Prerequisites

Before running, ensure these are installed:

1. **PostgreSQL 17** - Database server
   ```bash
   apt-get install postgresql-17
   ```

2. **jq** - JSON processor for tests
   ```bash
   apt-get install jq
   ```

3. **API Key** - Either `OPENAI_API_KEY` or `ANTHROPIC_API_KEY` environment variable

4. **Root access** - Required for PostgreSQL cluster initialization

## Quick Start

```bash
# Ensure API key is available
export OPENAI_API_KEY="your-key"  # or ANTHROPIC_API_KEY

# Run with root access
sudo -E .claude/skills/no-docker-setup/scripts/run.sh
```

## What This Does

The script performs these steps in order:

1. **Prerequisites Check**
   - Verifies root access
   - Checks API key is set
   - Sets encryption key (fixed value for consistency)
   - Verifies jq is installed
   - Verifies PostgreSQL 17 binaries exist

2. **Infrastructure Setup**
   - Kills any existing PostgreSQL on port 5432
   - Creates fresh cluster at `/tmp/pgdata`
   - Starts PostgreSQL on localhost:5432
   - Creates `everruns` user and database

3. **Application Setup**
   - Runs database migrations
   - Builds and starts API server (port 9000)
   - Builds and starts durable worker (port 9001)

4. **Ready for Testing**
   - Services run until Ctrl+C
   - Cleanup happens automatically on exit

## Fixed Configuration

| Setting | Value |
|---------|-------|
| PostgreSQL Version | 17 |
| PostgreSQL Binaries | `/usr/lib/postgresql/17/bin` |
| Data Directory | `/tmp/pgdata` |
| Database Port | 5432 |
| Database URL | `postgres://everruns:everruns@localhost:5432/everruns` |
| API Port | 9000 |
| gRPC Port | 9001 |
| Encryption Key | `kek-v1:8B3uCQ4Znx45hl5nB+PKVriRrj/KtEVM+wBZ2VGa9vY=` |

## Log Files

| Service | Location |
|---------|----------|
| API | `/tmp/api.log` |
| Worker | `/tmp/worker.log` |
| PostgreSQL | `/tmp/pgdata/pg.log` |

## Running Tests

After environment is ready, run the test checklist from the main smoke-test skill:

```bash
# Tool calling automated tests
.claude/skills/smoke-test/scripts/tool-calling-tests.sh

# Manual API tests - see .claude/skills/smoke-test/SKILL.md
```

## Differences from Docker Mode

| Aspect | Docker Mode | No-Docker Mode |
|--------|-------------|----------------|
| Setup | Auto-detects system state | Fixed paths, fresh cluster |
| PostgreSQL | Docker container | Local install at `/usr/lib/postgresql/17/bin` |
| Data Directory | Docker volume | `/tmp/pgdata` (always fresh) |
| Port Conflicts | Docker networking | Kills existing processes |
| Cleanup | Docker container lifecycle | Trap on Ctrl+C |

## Troubleshooting

### "PostgreSQL binaries not found"
Install PostgreSQL 17:
```bash
# Add PGDG repository
curl -fsSL https://www.postgresql.org/media/keys/ACCC4CF8.asc | gpg --dearmor -o /usr/share/keyrings/postgresql-keyring.gpg
echo "deb [signed-by=/usr/share/keyrings/postgresql-keyring.gpg] http://apt.postgresql.org/pub/repos/apt $(lsb_release -cs)-pgdg main" > /etc/apt/sources.list.d/pgdg.list
apt-get update
apt-get install postgresql-17
```

### "Port 5432 in use"
The script automatically kills processes on port 5432. If issues persist:
```bash
lsof -i :5432
kill -9 <pid>
```

### "API failed to start"
Check logs:
```bash
cat /tmp/api.log
```

### "Worker failed to start"
Check logs:
```bash
cat /tmp/worker.log
```

## Scripts

| Script | Description |
|--------|-------------|
| `run.sh` | Main entry point - sets up everything |
| `_postgres.sh` | PostgreSQL cluster management |
| `_utils.sh` | Shared utilities and configuration |
