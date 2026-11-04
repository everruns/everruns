use super::*;
use crate::storage::{generate_encryption_key, test_database::create_test_database};

fn services() -> Result<(EncryptionService, EncryptionService, EncryptionService)> {
    let old = generate_encryption_key("old");
    let current = generate_encryption_key("current");
    Ok((
        EncryptionService::new(&old, &[])?,
        EncryptionService::new(&current, &[old.as_str()])?,
        EncryptionService::new(&current, &[])?,
    ))
}

fn column() -> Result<&'static EncryptedColumn> {
    ENCRYPTED_COLUMNS
        .iter()
        .find(|column| column.table == "dreaming_state")
        .context("dreaming state participates in key rotation")
}

#[tokio::test]
async fn maintenance_rotation_preserves_tenant_identity_and_cas_revision() -> Result<()> {
    let (pool, _database) = create_test_database();
    let (old, rotation, current_only) = services()?;
    let table = column()?;
    let mut identities = Vec::new();
    for (org, revision, due, value) in [(1_i64, 7_i64, 123_i64, "one"), (999, 9, 456, "two")] {
        let id: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO dreaming_state (org_id, key, revision, value_encrypted, due_at)
             VALUES ($1, 'same-key', $2, $3, $4) RETURNING id",
        )
        .bind(org)
        .bind(revision)
        .bind(old.encrypt(value.as_bytes())?)
        .bind(due)
        .fetch_one(&pool)
        .await?;
        identities.push(id);
    }
    assert_ne!(identities[0], identities[1]);
    assert_eq!(
        process_table(&pool, &rotation, table, 1, true).await?,
        (2, 2)
    );
    let unchanged: Vec<Vec<u8>> =
        sqlx::query_scalar("SELECT value_encrypted FROM dreaming_state ORDER BY org_id")
            .fetch_all(&pool)
            .await?;
    assert_eq!(old.decrypt_to_string(&unchanged[0])?, "one");
    assert_eq!(old.decrypt_to_string(&unchanged[1])?, "two");

    assert_eq!(
        process_table(&pool, &rotation, table, 1, false).await?,
        (2, 2)
    );
    let rows: Vec<(i64, uuid::Uuid, i64, i64, Vec<u8>)> = sqlx::query_as(
        "SELECT org_id, id, revision, due_at, value_encrypted
         FROM dreaming_state ORDER BY org_id",
    )
    .fetch_all(&pool)
    .await?;
    for (index, (org, id, revision, due, ciphertext)) in rows.iter().enumerate() {
        assert_eq!(*id, identities[index]);
        let (expected_org, expected_revision, expected_due, expected_value) =
            [(1, 7, 123, "one"), (999, 9, 456, "two")][index];
        assert_eq!(
            (*org, *revision, *due),
            (expected_org, expected_revision, expected_due)
        );
        // A service with no old key can read each row after rotation.
        assert_eq!(current_only.decrypt_to_string(ciphertext)?, expected_value);
        assert!(old.decrypt(ciphertext).is_err());
    }
    assert_eq!(
        process_table(&pool, &current_only, table, 1, false).await?,
        (2, 0)
    );
    Ok(())
}

#[tokio::test]
async fn stale_rotation_cannot_restore_old_maintenance_state() -> Result<()> {
    let (pool, _database) = create_test_database();
    let (old, rotation, current_only) = services()?;
    let table = column()?;
    let original = old.encrypt(b"old reservation")?;
    let id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO dreaming_state (org_id, key, revision, value_encrypted)
         VALUES (1, 'reservation', 1, $1) RETURNING id",
    )
    .bind(&original)
    .fetch_one(&pool)
    .await?;
    let rotated_original = rotation
        .reencrypt(&original)?
        .context("old key needs rotation")?;
    let newer = current_only.encrypt(b"new reservation")?;
    sqlx::query("UPDATE dreaming_state SET revision = 2, value_encrypted = $1 WHERE id = $2")
        .bind(&newer)
        .bind(id)
        .execute(&pool)
        .await?;

    assert!(!replace_ciphertext(&pool, table, id, &original, &rotated_original).await?);
    let (revision, retained): (i64, Vec<u8>) =
        sqlx::query_as("SELECT revision, value_encrypted FROM dreaming_state WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(revision, 2);
    assert_eq!(retained, newer);
    assert_eq!(
        current_only.decrypt_to_string(&retained)?,
        "new reservation"
    );
    Ok(())
}

#[tokio::test]
async fn maintenance_cleanup_between_rotation_batches_does_not_skip_rows() -> Result<()> {
    let (pool, _database) = create_test_database();
    let (old, _, _) = services()?;
    let table = column()?;
    for key in ["first", "second"] {
        sqlx::query(
            "INSERT INTO dreaming_state (org_id, key, revision, value_encrypted)
             VALUES (1, $1, 1, $2)",
        )
        .bind(key)
        .bind(old.encrypt(key.as_bytes())?)
        .execute(&pool)
        .await?;
    }
    let first = fetch_batch(&pool, table, None, 1).await?;
    assert_eq!(first.len(), 1);
    let first_id = first[0].0;
    sqlx::query("DELETE FROM dreaming_state WHERE id = $1")
        .bind(first_id)
        .execute(&pool)
        .await?;
    let next = fetch_batch(&pool, table, Some(first_id), 1).await?;
    assert_eq!(next.len(), 1);
    assert!(next[0].0 > first_id);
    Ok(())
}
