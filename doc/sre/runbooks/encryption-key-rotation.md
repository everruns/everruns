# Encryption Key Rotation Runbook

This runbook describes how to rotate the secrets encryption key (KEK) used to encrypt sensitive data in the database.

## Overview

Everruns uses envelope encryption with versioned keys. Key rotation is a multi-phase process:

1. **Deploy new key** alongside old key
2. **Re-encrypt data** from old key to new key
3. **Remove old key** after all data is migrated

## Prerequisites

- Access to environment configuration (secrets manager or deployment config)
- Database read/write access (for re-encryption job)
- Ability to deploy application updates

## Rotation Procedure

### Phase 1: Generate New Key

Generate a new encryption key with an incremented version:

```bash
# Generate new key (increment version number from current)
python3 -c "import os, base64; print('kek-v2:' + base64.b64encode(os.urandom(32)).decode())"
```

Store the output securely. Example output:
```
kek-v2:xR7qW2mN9pL4kJ8vB3tY6fE1hG5sD0cA9uI7oP2nM6w=
```

### Phase 2: Deploy with Both Keys

Update environment configuration:

```bash
# Current key becomes the new one
SECRETS_ENCRYPTION_KEY=kek-v2:xR7qW2mN9pL4kJ8vB3tY6fE1hG5sD0cA9uI7oP2nM6w=

# Previous key is preserved for decryption
SECRETS_ENCRYPTION_KEY_PREVIOUS=kek-v1:8B3uCQ4Znx45hl5nB+PKVriRrj/KtEVM+wBZ2VGa9vY=
```

Deploy the application with both keys configured. At this point:
- **New encryptions** use `kek-v2`
- **Existing data** encrypted with `kek-v1` is still decryptable

### Phase 3: Re-encrypt Existing Data

Run the re-encryption job to migrate all data to the new key.

#### Option A: Manual SQL Script

```sql
-- Example: Re-encrypt llm_providers API keys
-- This is a template - actual implementation depends on your schema

-- 1. Find records with old key
SELECT id, api_key_encrypted
FROM llm_providers
WHERE api_key_encrypted::text LIKE '%"key_id":"kek-v1"%';

-- 2. Use application code to decrypt/re-encrypt each record
-- (Cannot be done in pure SQL - requires application decryption logic)
```

#### Option B: Background Job (Recommended)

Implement a background job that:

1. Queries for records where `key_id` in encrypted payload != current primary key
2. Decrypts with appropriate key
3. Re-encrypts with primary key
4. Updates record
5. Processes in batches to avoid load spikes

Example pseudocode:

```rust
async fn reencrypt_all_providers(
    pool: &PgPool,
    encryption: &EncryptionService,
) -> Result<u32> {
    let mut count = 0;

    // Fetch all encrypted records
    let records = sqlx::query!("SELECT id, api_key_encrypted FROM llm_providers")
        .fetch_all(pool)
        .await?;

    for record in records {
        if let Some(encrypted) = record.api_key_encrypted {
            // Check if needs re-encryption
            if let Some(new_encrypted) = encryption.reencrypt(&encrypted)? {
                sqlx::query!(
                    "UPDATE llm_providers SET api_key_encrypted = $1 WHERE id = $2",
                    new_encrypted,
                    record.id
                )
                .execute(pool)
                .await?;
                count += 1;
            }
        }
    }

    Ok(count)
}
```

### Phase 4: Verify Migration

Confirm all data has been migrated:

```sql
-- Check for any remaining records with old key
SELECT COUNT(*)
FROM llm_providers
WHERE api_key_encrypted::text LIKE '%"key_id":"kek-v1"%';
-- Should return 0
```

### Phase 5: Remove Old Key

Once verified, remove the old key from configuration:

```bash
# Remove the previous key
SECRETS_ENCRYPTION_KEY=kek-v2:xR7qW2mN9pL4kJ8vB3tY6fE1hG5sD0cA9uI7oP2nM6w=
# SECRETS_ENCRYPTION_KEY_PREVIOUS= (remove or leave empty)
```

Deploy the updated configuration.

**Important**: Keep the old key archived securely for disaster recovery. You may need it if backup restoration is required.

## Rollback Procedure

If issues occur during rotation:

### During Phase 2-3 (Both Keys Active)

No rollback needed - both keys work. Simply pause the re-encryption job if causing issues.

### After Phase 5 (Old Key Removed)

If old key was removed but some data wasn't migrated:

1. Re-add the old key as `SECRETS_ENCRYPTION_KEY_PREVIOUS`
2. Deploy
3. Complete the re-encryption job
4. Verify again before removing

## Monitoring

During rotation, monitor:

- **API Error Rates**: Watch for decryption failures
- **Re-encryption Progress**: Track percentage of records migrated
- **Database Load**: Ensure re-encryption isn't causing performance issues

## Emergency: Compromised Key

If a key is suspected compromised:

1. **Immediately** generate new key and deploy with both keys
2. Run re-encryption job with **highest priority**
3. Remove compromised key as soon as all data is migrated
4. Rotate any credentials that may have been exposed

## Key Storage Best Practices

- Store keys in a secrets manager (AWS Secrets Manager, HashiCorp Vault, etc.)
- Enable audit logging for key access
- Rotate keys on a regular schedule (e.g., annually)
- Keep previous key archived for disaster recovery (separate secure storage)
- Never commit keys to source control
