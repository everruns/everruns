---
title: Admin Container
description: Tools for checking migration status, key rotation, and other administrative tasks
---

The admin container provides tools for key rotation, migration status checks, and other administrative tasks in production environments.

> **Note**: Migrations are **auto-applied on server startup**. The admin container's `migrate` command is primarily for checking status or running migrations separately in special cases.

## Building

```bash
docker build --target admin -f docker/Dockerfile.unified -t everruns-admin .
```

## Commands

| Command | Description |
|---------|-------------|
| `migrate` | Run pending database migrations |
| `migrate-info` | Show migration status |
| `reencrypt` | Re-encrypt secrets with new key |
| `shell` | Interactive shell for debugging |
| `help` | Show usage information |

## Usage

### Check Migration Status

Use this before deployments to verify migration state:

```bash
docker run --rm \
    -e DATABASE_URL="postgres://user:pass@host:5432/db" \
    everruns-admin migrate-info
```

### Run Migrations Manually

Migrations auto-apply on server startup. Use this only for:
- Running migrations without starting the server
- Debugging migration issues (with `--no-migrations` on server)

```bash
docker run --rm \
    -e DATABASE_URL="postgres://user:pass@host:5432/db" \
    everruns-admin migrate
```

### Re-encrypt Secrets (Dry Run)

```bash
docker run --rm \
    -e DATABASE_URL="postgres://user:pass@host:5432/db" \
    -e SECRETS_ENCRYPTION_KEY="kek-v2:..." \
    -e SECRETS_ENCRYPTION_KEY_PREVIOUS="kek-v1:..." \
    everruns-admin reencrypt --dry-run
```

### Re-encrypt Secrets (Execute)

```bash
docker run --rm \
    -e DATABASE_URL="postgres://user:pass@host:5432/db" \
    -e SECRETS_ENCRYPTION_KEY="kek-v2:..." \
    -e SECRETS_ENCRYPTION_KEY_PREVIOUS="kek-v1:..." \
    everruns-admin reencrypt --batch-size 50
```

## Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `DATABASE_URL` | Yes | PostgreSQL connection string |
| `SECRETS_ENCRYPTION_KEY` | For reencrypt | Primary encryption key |
| `SECRETS_ENCRYPTION_KEY_PREVIOUS` | For rotation | Previous encryption key |
| `RUST_LOG` | No | Log level (default: info) |

## TLS/SSL Connections

The admin container supports TLS connections to PostgreSQL. Use the `sslmode` parameter in your connection string:

```bash
DATABASE_URL="postgres://user:pass@host:5432/db?sslmode=require"
```

## Migration Troubleshooting

### Migration Fails on Startup

If the server won't start due to a migration error:

1. Check server logs for the specific SQL error
2. Fix the migration file
3. Rebuild and redeploy

### Bad Migration Deployed

If a migration succeeded but caused issues, use **forward-fix**:

```bash
# Create a new migration that fixes the problem
sqlx migrate add -r fix_bad_migration
# Edit the migration, then redeploy
```

### Emergency: Manual Database Fix

For emergencies where you need to manually fix the database:

```bash
# Start server without auto-migrations
everruns-server --no-migrations

# Connect and fix manually
psql -h host -U everruns -d everruns
> -- Fix schema issues
> DELETE FROM _sqlx_migrations WHERE version = 006;  -- If needed

# Restart server normally
everruns-server
```

## Production Deployment

The admin container can be run as a one-off task in any container orchestration platform:

- **Kubernetes**: Use a Job or run via `kubectl run`
- **ECS**: Use `aws ecs run-task` with command override
- **Docker Compose**: Use `docker compose run`
- **Nomad**: Use a batch job
