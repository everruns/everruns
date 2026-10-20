//! Key rotation over registered encrypted columns, preserving concurrent writes.
use super::{ENCRYPTED_COLUMNS, EncryptedColumn, EncryptionService};
use anyhow::{Context, Result};
use sqlx::PgPool;

/// Process a single table, re-encrypting any records with old keys
pub async fn process_table(
    pool: &PgPool,
    encryption: &EncryptionService,
    table: &EncryptedColumn,
    batch_size: i64,
    dry_run: bool,
) -> Result<(u64, u64)> {
    anyhow::ensure!(
        ENCRYPTED_COLUMNS.contains(table),
        "unregistered encrypted column"
    );
    anyhow::ensure!(batch_size > 0, "batch size must be positive");
    let mut processed = 0u64;
    let mut reencrypted = 0u64;
    let mut after = None;

    loop {
        // Stable keyset traversal does not skip rows when maintenance GC
        // deletes an earlier row between batches.
        let rows = fetch_batch(pool, table, after, batch_size).await?;

        if rows.is_empty() {
            break;
        }

        for (id, encrypted_data) in &rows {
            processed += 1;

            if let Some(data) = encrypted_data {
                // Check if needs re-encryption
                match encryption.is_current_key(data) {
                    Ok(true) => {
                        // Already using current key
                    }
                    Ok(false) => {
                        // Needs re-encryption
                        let key_id = EncryptionService::get_key_id(data).unwrap_or_default();
                        if dry_run {
                            tracing::info!(
                                "Would re-encrypt {}.{} (id={}, current_key={})",
                                table.table,
                                table.column,
                                id,
                                key_id
                            );
                        } else {
                            // Decrypt and re-encrypt
                            match encryption.reencrypt(data) {
                                Ok(Some(new_data)) => {
                                    // A runtime may replace maintenance state while
                                    // rotation is reading it. Never restore the stale value.
                                    if !replace_ciphertext(pool, table, *id, data, &new_data)
                                        .await?
                                    {
                                        continue;
                                    }

                                    tracing::info!(
                                        "Re-encrypted {}.{} (id={}, {} -> {})",
                                        table.table,
                                        table.column,
                                        id,
                                        key_id,
                                        encryption.primary_key_id()
                                    );
                                }
                                Ok(None) => {
                                    // Already current (shouldn't happen due to is_current_key check)
                                }
                                Err(e) => {
                                    tracing::error!(
                                        error = %e,
                                        table = table.table,
                                        column = table.column,
                                        id = %id,
                                        "Failed to re-encrypt record"
                                    );
                                    continue;
                                }
                            }
                        }
                        reencrypted += 1;
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            table = table.table,
                            column = table.column,
                            id = %id,
                            "Failed to check encryption key"
                        );
                    }
                }
            }
        }

        after = rows.last().map(|(id, _)| *id);

        // Progress log every 1000 records
        if processed.is_multiple_of(1000) {
            tracing::info!(
                "Progress: {} processed, {} need re-encryption",
                processed,
                reencrypted
            );
        }
    }

    Ok((processed, reencrypted))
}

async fn fetch_batch(
    pool: &PgPool,
    table: &EncryptedColumn,
    after: Option<uuid::Uuid>,
    batch_size: i64,
) -> Result<Vec<(uuid::Uuid, Option<Vec<u8>>)>> {
    let boundary = if after.is_some() {
        format!("{} > $1", table.id_column)
    } else {
        "$1::uuid IS NULL".to_owned()
    };
    let query = format!(
        "SELECT {}, {} FROM {} WHERE {} ORDER BY {} LIMIT $2",
        table.id_column, table.column, table.table, boundary, table.id_column
    );
    sqlx::query_as(sqlx::AssertSqlSafe(query.as_str()))
        .bind(after)
        .bind(batch_size)
        .fetch_all(pool)
        .await
        .context("Failed to fetch encrypted records")
}

async fn replace_ciphertext(
    pool: &PgPool,
    table: &EncryptedColumn,
    id: uuid::Uuid,
    old_data: &[u8],
    new_data: &[u8],
) -> Result<bool> {
    let query = format!(
        "UPDATE {} SET {} = $1 WHERE {} = $2 AND {} = $3",
        table.table, table.column, table.id_column, table.column
    );
    let result = sqlx::query(sqlx::AssertSqlSafe(query.as_str()))
        .bind(new_data)
        .bind(id)
        .bind(old_data)
        .execute(pool)
        .await
        .context("Failed to update encrypted record")?;
    Ok(result.rows_affected() == 1)
}

#[cfg(test)]
mod tests;
