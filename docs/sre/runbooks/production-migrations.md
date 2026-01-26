---
title: Production Database Migrations
description: How database migrations work in production environments
---

This runbook describes how database migrations work in production environments.

## Overview

**Migrations are applied automatically on server startup.** The `everruns-server` runs pending migrations when it starts, using PostgreSQL advisory locks to ensure only one instance applies migrations even in multi-instance deployments.

## How Auto-Migration Works

1. Server connects to PostgreSQL
2. Acquires advisory lock (blocks if another instance is migrating)
3. Checks `_sqlx_migrations` table for pending migrations
4. Applies pending migrations in order
5. Releases lock
6. Continues startup

### Multi-Instance Safety

PostgreSQL advisory locks ensure safe concurrent startup:

```
Instance A                Instance B
    |                         |
    v                         v
 Connect                   Connect
    |                         |
    v                         |
Acquire lock                  |
    |                         v
Run migrations         Wait for lock...
    |                         |
    v                         |
Release lock                  v
    |                    Acquire lock
    v                         |
Start serving           No pending migrations
                              |
                              v
                        Release lock
                              |
                              v
                        Start serving
```

## Disabling Auto-Migration

To skip automatic migrations (e.g., for debugging or special deployments):

```bash
everruns-server --no-migrations
```

## Admin Container

The admin container is still useful for:

### Checking Migration Status

Before deploying, verify migration status:

```bash
docker run --rm \
    -e DATABASE_URL="$DATABASE_URL" \
    everruns-admin migrate-info
```

### Running Migrations Separately

If you need to run migrations without starting the server:

```bash
docker run --rm \
    -e DATABASE_URL="$DATABASE_URL" \
    everruns-admin migrate
```

### Debugging

For interactive debugging:

```bash
docker run --rm -it \
    -e DATABASE_URL="$DATABASE_URL" \
    everruns-admin shell
```

## Migration Strategy

### Backward-Compatible Changes (Preferred)

For most changes, use additive migrations that work with both old and new code:

1. Deploy new code (auto-applies migration on startup)
2. Migration adds new tables/columns with defaults
3. Old instances continue working (ignore new schema)

### Breaking Changes

For breaking schema changes, use multi-phase deployment:

1. **Phase 1**: Deploy migration that adds new schema (backward compatible)
2. **Phase 2**: Deploy code that uses new schema
3. **Phase 3**: Deploy migration that removes old schema (optional cleanup)

## Rollback Procedure

SQLx does not have built-in rollback. To rollback a migration:

### Option 1: Manual Rollback

1. Identify the changes made by the migration
2. Write and execute reverse SQL manually
3. Delete the migration record:
   ```sql
   DELETE FROM _sqlx_migrations WHERE version = 20240201000000;
   ```

### Option 2: Forward-Fix (Safer)

1. Create a new migration that reverts the changes
2. Deploy with the fix migration
3. Maintains clear audit trail

## Troubleshooting

### Migration Fails on Startup

If server fails to start due to migration error:

1. Check server logs for specific SQL error
2. Fix the migration file or database state
3. If needed, use `--no-migrations` to start server for debugging
4. Use admin container to run migrations manually after fixing

### Migration Takes Too Long

For large data migrations:

1. Consider running migration during maintenance window
2. Use admin container to run migration separately
3. Start servers with `--no-migrations` first
4. Run migration via admin container
5. Restart servers normally

### Deadlock During Migration

If multiple instances appear stuck:

1. Check PostgreSQL for advisory lock holders:
   ```sql
   SELECT * FROM pg_locks WHERE locktype = 'advisory';
   ```
2. Kill stuck connections if necessary
3. Restart affected instances

## Best Practices

1. **Test in staging** before production deployment
2. **Prefer additive changes** (new columns with defaults)
3. **Use transactions** for multi-statement migrations
4. **Monitor startup time** after adding new migrations
5. **Check migration status** with admin container before deployment
6. **Have a rollback plan** for complex migrations
