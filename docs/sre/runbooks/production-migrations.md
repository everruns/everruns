---
title: Production Database Migrations
description: How to run database migrations in production environments
---

This runbook describes how to run database migrations in production environments.

## Overview

Database migrations should be run **before** deploying new application code that depends on schema changes. The admin container provides a safe way to run migrations as a one-off task.

## Prerequisites

- Access to run containers in your production environment
- Network access to the production database
- The `everruns-admin` container image

## Migration Strategy

### When to Run Migrations

1. **Pre-deployment**: Run migrations before deploying new code that requires schema changes
2. **Backward-compatible changes**: Prefer additive migrations (new tables, new columns with defaults)
3. **Multi-phase deployments**: For breaking changes, use multiple deployments:
   - Phase 1: Add new schema (backward compatible)
   - Phase 2: Deploy new code
   - Phase 3: Remove old schema (if needed)

### Migration Execution Flow

```
┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
│  Check Status   │────▶│ Run Migrations  │────▶│ Deploy New Code │
│ (migrate-info)  │     │    (migrate)    │     │                 │
└─────────────────┘     └─────────────────┘     └─────────────────┘
```

## Procedure

### Step 1: Check Current Migration Status

Before running migrations, check which migrations have been applied:

```bash
docker run --rm \
    -e DATABASE_URL="$DATABASE_URL" \
    everruns-admin migrate-info
```

Expected output shows applied and pending migrations:
```
20240101000000/installed: 001_initial_schema
20240115000000/installed: 002_llm_providers
20240201000000/pending: 003_new_feature (NEW)
```

### Step 2: Review Pending Migrations

Before applying, review the migration SQL:

```bash
ls -la crates/control-plane/migrations/
cat crates/control-plane/migrations/003_new_feature.sql
```

**Check for:**
- Backward compatibility with current code
- Long-running operations (large table alterations)
- Potential locking issues
- Data migrations that might fail

### Step 3: Run Migrations

Execute the pending migrations:

```bash
docker run --rm \
    -e DATABASE_URL="$DATABASE_URL" \
    everruns-admin migrate
```

The task will:
1. Connect to the database
2. Apply pending migrations in order
3. Record applied migrations in `_sqlx_migrations` table
4. Exit with code 0 on success

### Step 4: Verify Migration Success

Confirm migrations were applied:

```bash
docker run --rm \
    -e DATABASE_URL="$DATABASE_URL" \
    everruns-admin migrate-info
```

All migrations should show as `installed`.

### Step 5: Deploy Application

Once migrations are verified, proceed with the application deployment.

## Rollback Procedure

SQLx does not have built-in rollback. To rollback a migration:

### Option 1: Manual Rollback

1. Identify the changes made by the migration
2. Write and execute reverse SQL manually
3. Delete the migration record:
   ```sql
   DELETE FROM _sqlx_migrations WHERE version = 20240201000000;
   ```

### Option 2: Forward-Fix

Often safer than rollback:
1. Create a new migration that reverts the changes
2. Apply the new migration
3. This maintains a clear audit trail

## Troubleshooting

### Migration Fails Mid-Way

If a migration partially applies:

1. **Check database state**: Verify what was applied
2. **Fix manually**: Apply remaining changes or rollback
3. **Update migration table**: Ensure state is consistent

```sql
-- Check migration status
SELECT * FROM _sqlx_migrations ORDER BY version;

-- If needed, mark as applied after manual fix
INSERT INTO _sqlx_migrations (version, description, installed_on, success, checksum)
VALUES (20240201000000, '003_new_feature', NOW(), true, '...');
```

### Container Fails to Start

Common issues:

1. **Database not reachable**: Check network connectivity and credentials
2. **Invalid DATABASE_URL**: Verify connection string format
3. **Missing secrets**: Ensure environment variables are set

### Database Connection Timeout

If migrations timeout:

1. Check network rules allow database access
2. Verify database is accessible from the container environment
3. Check database credentials

## Emergency: Production Database Issues

If migrations cause production issues:

1. **Stop the deployment pipeline** immediately
2. **Assess the damage**: What queries are failing?
3. **Decide: rollback or forward-fix**
   - Rollback if changes are isolated
   - Forward-fix if rollback is risky
4. **Execute the fix** using admin container
5. **Verify application health**
6. **Post-incident review**

## Best Practices

1. **Always check status first** before running migrations
2. **Review migration SQL** before applying to production
3. **Test in staging** before production
4. **Prefer additive changes** (add columns, not modify)
5. **Use transactions** for multi-statement migrations
6. **Monitor database metrics** during migration
7. **Have a rollback plan** before starting
8. **Document breaking changes** in migration files
