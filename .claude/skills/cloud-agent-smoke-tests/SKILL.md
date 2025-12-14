# Cloud Agent Smoke Testing

This skill documents how to run smoke tests in Cloud Agent environments where Docker is not available.

## Overview

The Cloud Agent environment provides:
- PostgreSQL 16 binaries (but no running server)
- Rust toolchain
- psql client
- Standard networking tools (curl, wget, nc)
- GitHub API access (for CI integration)

Docker and container runtimes are **NOT available**.

## Quick Start

```bash
# All-in-one: sets up PostgreSQL + Temporal, runs migrations, starts API, runs smoke tests
./.claude/skills/cloud-agent-smoke-tests/scripts/run-smoke-tests.sh
```

This script handles everything automatically and cleans up on exit.

## Architecture

The smoke test setup runs three services:

1. **PostgreSQL 16** - Local cluster initialized in `/tmp/pgdata`
2. **Temporal Dev Server** - Uses Temporal CLI with in-memory SQLite
3. **Everruns API** - Rust server on port 9000

```
┌─────────────────────────────────────────────────────────────┐
│                   Cloud Agent Environment                    │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐  │
│  │  PostgreSQL  │    │   Temporal   │    │  Everruns    │  │
│  │    (16)      │    │  Dev Server  │    │    API       │  │
│  │              │    │              │    │              │  │
│  │ /tmp/pgdata  │    │ :7233        │    │ :9000        │  │
│  │   (socket)   │    │ (in-memory)  │    │              │  │
│  └──────────────┘    └──────────────┘    └──────────────┘  │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

## Options for Running Smoke Tests

### Option 1: Local Setup (Recommended)

Uses local PostgreSQL cluster and Temporal CLI dev server:

```bash
./.claude/skills/cloud-agent-smoke-tests/scripts/run-smoke-tests.sh
```

**What it does:**
1. Checks for PostgreSQL binaries
2. Downloads Temporal CLI if not present
3. Starts Temporal dev server (in-memory SQLite backend)
4. Initializes PostgreSQL cluster as `postgres` user
5. Creates database and installs UUIDv7 polyfill
6. Runs migrations
7. Builds and starts the API
8. Executes smoke tests
9. Cleans up all services on exit

### Option 2: Manual Setup

**Step 1: Install Temporal CLI**
```bash
curl -sL "https://temporal.download/cli/archive/latest?platform=linux&arch=amd64" -o /tmp/temporal.tar.gz
tar -xzf /tmp/temporal.tar.gz -C /tmp
mv /tmp/temporal /usr/local/bin/temporal
chmod +x /usr/local/bin/temporal
temporal --version
```

**Step 2: Start Temporal**
```bash
temporal server start-dev --headless &
# Verify: nc -z localhost 7233
```

**Step 3: Initialize PostgreSQL**
```bash
PGDATA="/tmp/pgdata"
rm -rf "$PGDATA"
mkdir -p "$PGDATA"
chown postgres:postgres "$PGDATA"

su - postgres -c "initdb -D $PGDATA --auth=trust"
su - postgres -c "echo \"unix_socket_directories = '$PGDATA'\" >> $PGDATA/postgresql.conf"
su - postgres -c "pg_ctl -D $PGDATA -l $PGDATA/pg.log start"
```

**Step 4: Create Database**
```bash
su - postgres -c "psql -h $PGDATA -c \"CREATE USER everruns WITH PASSWORD 'everruns';\""
su - postgres -c "psql -h $PGDATA -c \"CREATE DATABASE everruns OWNER everruns;\""
```

**Step 5: Install UUIDv7 Polyfill**
```sql
-- Connect to everruns database
CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE OR REPLACE FUNCTION uuidv7() RETURNS uuid AS $$
DECLARE
  unix_ts_ms BIGINT;
  uuid_bytes BYTEA;
BEGIN
  unix_ts_ms := (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::BIGINT;
  uuid_bytes := set_byte(set_byte(set_byte(set_byte(set_byte(set_byte(
    gen_random_bytes(16),
    0, ((unix_ts_ms >> 40) & 255)::INT),
    1, ((unix_ts_ms >> 32) & 255)::INT),
    2, ((unix_ts_ms >> 24) & 255)::INT),
    3, ((unix_ts_ms >> 16) & 255)::INT),
    4, ((unix_ts_ms >> 8) & 255)::INT),
    5, (unix_ts_ms & 255)::INT);
  uuid_bytes := set_byte(uuid_bytes, 6, (get_byte(uuid_bytes, 6) & 15) | 112);
  uuid_bytes := set_byte(uuid_bytes, 8, (get_byte(uuid_bytes, 8) & 63) | 128);
  RETURN encode(uuid_bytes, 'hex')::uuid;
END;
$$ LANGUAGE plpgsql VOLATILE;
```

**Step 6: Run Migrations & API**
```bash
export DATABASE_URL="postgres://everruns:everruns@%2Ftmp%2Fpgdata/everruns"
export TEMPORAL_ADDRESS="localhost:7233"
sqlx migrate run --source crates/everruns-storage/migrations
cargo run -p everruns-api &
```

**Step 7: Run Smoke Tests**
```bash
./scripts/smoke-test.sh
```

### Option 3: Remote Services via SSH Tunnel

If you have a VM with PostgreSQL and Temporal:

```bash
# Create SSH tunnels
ssh -L 5432:localhost:5432 -L 7233:localhost:7233 user@your-vm.example.com

# Use standard URLs
export DATABASE_URL="postgres://everruns:everruns@localhost:5432/everruns"
export TEMPORAL_ADDRESS="localhost:7233"
```

### Option 4: GitHub Actions CI

Trigger the existing CI workflow which includes smoke tests:

```bash
# Push to trigger CI
git push origin <branch>

# Or manually trigger (if workflow_dispatch enabled)
gh workflow run ci.yml
gh run watch
```

## Environment Requirements

### Required
- PostgreSQL 16+ binaries (`/usr/lib/postgresql/16/bin/`)
- `postgres` system user
- Root access (for `su - postgres`)
- Rust toolchain
- sqlx-cli
- curl, tar (for Temporal CLI download)
- nc (netcat, for port checking)

### Checking Environment
```bash
# Check PostgreSQL
dpkg -l | grep postgresql
/usr/lib/postgresql/16/bin/initdb --version

# Check Rust
cargo --version

# Check network tools
which curl nc tar

# Check sqlx
which sqlx || cargo install sqlx-cli --no-default-features --features postgres

# Check Temporal CLI
which temporal || echo "Will be downloaded automatically"
```

## Troubleshooting

### "cannot be run as root"
PostgreSQL's initdb refuses to run as root. Use:
```bash
su - postgres -c "initdb ..."
```

### "function uuidv7() does not exist"
PostgreSQL 16 doesn't have native UUIDv7. Install the polyfill function.

### "Permission denied" for log file
Create the log file with correct ownership:
```bash
touch /tmp/pgdata/pg.log
chown postgres:postgres /tmp/pgdata/pg.log
```

### Connection refused on localhost:5432
Use socket connection:
```bash
export DATABASE_URL="postgres://everruns:everruns@%2Ftmp%2Fpgdata/everruns"
# Note: %2F is URL-encoded /
```

### Temporal CLI download fails
Check network access or download manually from:
https://temporal.io/download

### Run status shows "failed"
This is expected without `OPENAI_API_KEY`. The workflow can't execute LLM calls.
The smoke test still validates all API operations work correctly.

### Temporal connection refused
Check if Temporal is running:
```bash
nc -z localhost 7233 && echo "Temporal is running" || echo "Temporal not running"
```

Start it with:
```bash
temporal server start-dev --headless &
```

## Comparison Matrix

| Option | PostgreSQL | Temporal | Setup Time | Network |
|--------|------------|----------|------------|---------|
| Local Setup | Local cluster | CLI dev server | ~2 min | No |
| SSH Tunnel | Remote | Remote | ~5 min | Yes |
| GitHub Actions | CI service | CI service | ~0 min | Yes |

## Skill Files

```
cloud-agent-smoke-tests/
├── SKILL.md                        # This documentation
└── scripts/
    ├── run-smoke-tests.sh          # Main entry point (orchestration)
    ├── common.sh                   # Shared utilities (logging, config)
    ├── setup-postgres.sh           # PostgreSQL setup functions
    └── setup-temporal.sh           # Temporal setup functions
```

Individual scripts can be sourced separately for custom setups.

## Runtime Files Created

- `/tmp/pgdata/` - PostgreSQL data directory
- `/tmp/pgdata/pg.log` - PostgreSQL log
- `/tmp/temporal.log` - Temporal server log
- `/tmp/api.log` - API server log
- `/usr/local/bin/temporal` - Temporal CLI binary

All runtime files are cleaned up automatically when the script exits (except Temporal CLI binary).

## Temporal Dev Server Details

The Temporal CLI includes a development server that:
- Uses in-memory SQLite (no external database needed)
- Runs on port 7233 (gRPC) by default
- Includes a built-in UI (disabled with `--headless`)
- Perfect for testing and development

To use with UI (for debugging):
```bash
temporal server start-dev
# UI available at http://localhost:8233
```
