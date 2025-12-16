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
- The `reencrypt-secrets` CLI tool (included in the API crate)

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

Use the `reencrypt-secrets` CLI tool to migrate all data to the new key.

#### Step 1: Dry Run (Preview Changes)

First, run in dry-run mode to see what would be re-encrypted:

```bash
# From the project root
cargo run --bin reencrypt-secrets -- --dry-run

# Or with a release build
./target/release/reencrypt-secrets --dry-run
```

Example output:
```
2024-01-15T10:00:00Z INFO Encryption service initialized. Primary key: kek-v2
2024-01-15T10:00:00Z INFO Available keys: ["kek-v2", "kek-v1"]
2024-01-15T10:00:00Z INFO Connected to database
2024-01-15T10:00:00Z INFO Processing table: llm_providers
2024-01-15T10:00:01Z INFO Would re-encrypt llm_providers.api_key_encrypted (id=..., current_key=kek-v1)
2024-01-15T10:00:01Z INFO DRY RUN: Would re-encrypt 42 of 100 records
```

#### Step 2: Execute Re-encryption

Once satisfied with the dry run, execute the actual re-encryption:

```bash
# Re-encrypt all tables
cargo run --bin reencrypt-secrets

# Or with specific options
cargo run --bin reencrypt-secrets -- --batch-size 50 --table llm_providers
```

#### CLI Options

```
USAGE:
    reencrypt-secrets [OPTIONS]

OPTIONS:
    -n, --dry-run           Show what would be changed without making changes
    -b, --batch-size <N>    Process N records at a time (default: 100)
    -t, --table <NAME>      Only process specified table (default: all)
    -h, --help              Show this help message
```

### Phase 4: Verify Migration

Confirm all data has been migrated by running another dry run:

```bash
cargo run --bin reencrypt-secrets -- --dry-run
```

Expected output:
```
2024-01-15T11:00:00Z INFO DRY RUN: Would re-encrypt 0 of 100 records
```

You can also verify directly in the database:

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

No rollback needed - both keys work. Simply stop the re-encryption CLI if causing issues.

### After Phase 5 (Old Key Removed)

If old key was removed but some data wasn't migrated:

1. Re-add the old key as `SECRETS_ENCRYPTION_KEY_PREVIOUS`
2. Deploy
3. Run the re-encryption CLI again
4. Verify again before removing

## Monitoring

During rotation, monitor:

- **CLI Progress Output**: The tool logs progress every 1000 records
- **API Error Rates**: Watch for decryption failures in application logs
- **Database Load**: Ensure re-encryption isn't causing performance issues

## Emergency: Compromised Key

If a key is suspected compromised:

1. **Immediately** generate new key and deploy with both keys
2. Run re-encryption CLI with **highest priority**:
   ```bash
   cargo run --release --bin reencrypt-secrets
   ```
3. Remove compromised key as soon as all data is migrated
4. Rotate any credentials that may have been exposed

## Key Storage Best Practices

- Store keys in a secrets manager (AWS Secrets Manager, HashiCorp Vault, etc.)
- Enable audit logging for key access
- Rotate keys on a regular schedule (e.g., annually)
- Keep previous key archived for disaster recovery (separate secure storage)
- Never commit keys to source control

## Adding New Encrypted Tables

When adding encryption to a new table, update the `get_encrypted_tables()` function in `crates/everruns-api/src/bin/reencrypt_secrets.rs`:

```rust
fn get_encrypted_tables(filter: &Option<String>) -> Vec<EncryptedTable> {
    let all_tables = vec![
        EncryptedTable {
            name: "llm_providers",
            id_column: "id",
            encrypted_columns: &["api_key_encrypted"],
        },
        // Add new tables here
        EncryptedTable {
            name: "new_table",
            id_column: "id",
            encrypted_columns: &["secret_field"],
        },
    ];
    // ...
}
```
