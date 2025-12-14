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

## Options for Running Smoke Tests

### Option 1: Local PostgreSQL (Recommended)

The environment has PostgreSQL 16 installed. We can initialize and run a local cluster:

```bash
# Run the all-in-one script
./scripts/cloud-agent-smoke-test.sh
```

**Manual Steps:**

1. Initialize PostgreSQL cluster as postgres user:
```bash
PGDATA="/tmp/pgdata"
rm -rf "$PGDATA"
mkdir -p "$PGDATA"
chown postgres:postgres "$PGDATA"
su - postgres -c "initdb -D $PGDATA --auth=trust"
su - postgres -c "echo \"unix_socket_directories = '$PGDATA'\" >> $PGDATA/postgresql.conf"
```

2. Start PostgreSQL:
```bash
su - postgres -c "pg_ctl -D $PGDATA -l $PGDATA/pg.log start"
```

3. Create database:
```bash
su - postgres -c "psql -h $PGDATA -c \"CREATE USER everruns WITH PASSWORD 'everruns';\""
su - postgres -c "psql -h $PGDATA -c \"CREATE DATABASE everruns OWNER everruns;\""
```

4. Install UUIDv7 polyfill (PostgreSQL < 18):
```sql
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

5. Run migrations and API:
```bash
export DATABASE_URL="postgres://everruns:everruns@%2Ftmp%2Fpgdata/everruns"
sqlx migrate run --source crates/everruns-storage/migrations
cargo run -p everruns-api &
```

6. Run smoke tests:
```bash
./scripts/smoke-test.sh
```

### Option 2: Remote PostgreSQL via SSH Tunnel

If you have a VM with PostgreSQL:

```bash
# Create SSH tunnel (from another terminal or use -f for background)
ssh -L 5432:localhost:5432 user@your-vm.example.com

# Use standard DATABASE_URL
export DATABASE_URL="postgres://everruns:everruns@localhost:5432/everruns"
```

**Setup on VM:**
```bash
# On the VM
docker run -d --name pg \
  -e POSTGRES_USER=everruns \
  -e POSTGRES_PASSWORD=everruns \
  -e POSTGRES_DB=everruns \
  -p 5432:5432 \
  postgres:18-alpine
```

### Option 3: Cloud-Hosted PostgreSQL

Use managed PostgreSQL services:

- **Neon**: Free tier available, instant provisioning
- **Supabase**: Free tier with PostgreSQL 15
- **Railway**: Easy setup, pay-as-you-go
- **Render**: Free PostgreSQL with 90-day limit

Example with Neon:
```bash
export DATABASE_URL="postgres://user:password@ep-xxx.us-east-2.aws.neon.tech/everruns?sslmode=require"
```

**Note:** Network access may be restricted in some Cloud Agent environments.

### Option 4: GitHub Actions CI

Trigger smoke tests via GitHub Actions:

```bash
# Trigger workflow (requires gh CLI)
gh workflow run ci.yml

# Check status
gh run list --workflow=ci.yml
gh run watch <run-id>
```

The CI workflow already includes a `smoke-test` job that:
1. Spins up PostgreSQL as a service
2. Builds the API
3. Runs all smoke tests

### Option 5: Hybrid Testing

For comprehensive testing:

1. **Unit tests**: Run locally without database
```bash
cargo test --lib
```

2. **API tests**: Run with local PostgreSQL (Option 1)
```bash
./scripts/cloud-agent-smoke-test.sh
```

3. **Full integration**: Use GitHub Actions (Option 4)

## Environment Requirements

### Required (for local PostgreSQL)
- PostgreSQL 16+ binaries (`/usr/lib/postgresql/16/bin/`)
- `postgres` system user
- Root access (for `su - postgres`)
- Rust toolchain
- sqlx-cli

### Checking Environment
```bash
# Check PostgreSQL
dpkg -l | grep postgresql
/usr/lib/postgresql/16/bin/initdb --version

# Check Rust
cargo --version
rustc --version

# Check sqlx
which sqlx || cargo install sqlx-cli --no-default-features --features postgres
```

## Troubleshooting

### "cannot be run as root"
PostgreSQL's initdb refuses to run as root. Use:
```bash
su - postgres -c "initdb ..."
```

### "function uuidv7() does not exist"
PostgreSQL 16 doesn't have native UUIDv7. Install the polyfill function (see above).

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

### Run status shows "failed"
This is expected without `OPENAI_API_KEY`. The workflow can't execute LLM calls.
The smoke test still validates all API operations work correctly.

## Comparison Matrix

| Option | Setup Time | Reliability | Network Required | Best For |
|--------|------------|-------------|------------------|----------|
| Local PostgreSQL | 2 min | High | No | Quick validation |
| SSH Tunnel | 5 min | Medium | Yes | Existing VMs |
| Cloud PostgreSQL | 10 min | High | Yes | Persistent testing |
| GitHub Actions | 0 min | High | Yes | CI/CD integration |

## Quick Start

```bash
# Fastest path to running smoke tests in Cloud Agent:
./scripts/cloud-agent-smoke-test.sh
```

This script handles all setup automatically and cleans up on exit.
