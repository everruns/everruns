//! Command idempotency keys in PostgreSQL. See
//! `crate::storage::command_idempotency`.

use anyhow::Result;

use super::Database;
use crate::storage::command_idempotency::{
    ClaimIdempotencyKey, EXPIRED_SWEEP_LIMIT, IdempotencyClaim, IdempotencyKeyScope,
    StoredIdempotencyKey,
};

impl Database {
    /// Claims the key in one statement: a new key is inserted, and an expired
    /// row, or an abandoned in-flight row for the same request, is taken
    /// over. Anything else is returned as it stands.
    pub async fn claim_command_idempotency_key(
        &self,
        input: &ClaimIdempotencyKey,
    ) -> Result<IdempotencyClaim> {
        sqlx::query(
            r#"
            DELETE FROM command_idempotency_keys
            WHERE ctid IN (
                SELECT ctid FROM command_idempotency_keys
                WHERE expires_at < NOW()
                LIMIT $1
            )
            "#,
        )
        .bind(EXPIRED_SWEEP_LIMIT)
        .execute(&self.pool)
        .await?;

        // A claim can lose to a concurrent release between the upsert and the
        // read; the next round then inserts.
        for _ in 0..3 {
            let claimed: Option<(i64,)> = sqlx::query_as(
                r#"
                INSERT INTO command_idempotency_keys AS k (
                    org_id, principal_id, idempotency_key, command, fingerprint,
                    response, locked_until, expires_at
                ) VALUES ($1, $2, $3, $4, $5, NULL, $6, $7)
                ON CONFLICT (org_id, principal_id, idempotency_key) DO UPDATE
                SET command = EXCLUDED.command,
                    fingerprint = EXCLUDED.fingerprint,
                    response = NULL,
                    locked_until = EXCLUDED.locked_until,
                    expires_at = EXCLUDED.expires_at,
                    created_at = NOW()
                WHERE k.expires_at < NOW()
                   OR (k.response IS NULL
                       AND k.locked_until < NOW()
                       AND k.fingerprint = EXCLUDED.fingerprint)
                RETURNING k.org_id
                "#,
            )
            .bind(input.scope.org_id)
            .bind(input.scope.principal_id)
            .bind(&input.scope.key)
            .bind(&input.command)
            .bind(&input.fingerprint)
            .bind(input.locked_until)
            .bind(input.expires_at)
            .fetch_optional(&self.pool)
            .await?;
            if claimed.is_some() {
                return Ok(IdempotencyClaim::Claimed);
            }

            let existing: Option<(String, String, Option<Vec<u8>>)> = sqlx::query_as(
                r#"
                SELECT command, fingerprint, response
                FROM command_idempotency_keys
                WHERE org_id = $1 AND principal_id = $2 AND idempotency_key = $3
                "#,
            )
            .bind(input.scope.org_id)
            .bind(input.scope.principal_id)
            .bind(&input.scope.key)
            .fetch_optional(&self.pool)
            .await?;
            if let Some((command, fingerprint, response)) = existing {
                return Ok(IdempotencyClaim::Existing(StoredIdempotencyKey {
                    command,
                    fingerprint,
                    response,
                }));
            }
        }
        anyhow::bail!("idempotency key claim kept racing with its release")
    }

    pub async fn complete_command_idempotency_key(
        &self,
        scope: &IdempotencyKeyScope,
        response: &[u8],
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE command_idempotency_keys
            SET response = $4
            WHERE org_id = $1 AND principal_id = $2 AND idempotency_key = $3
            "#,
        )
        .bind(scope.org_id)
        .bind(scope.principal_id)
        .bind(&scope.key)
        .bind(response)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Forgets an in-flight key so the request can be retried. A completed
    /// row is never released.
    pub async fn release_command_idempotency_key(&self, scope: &IdempotencyKeyScope) -> Result<()> {
        sqlx::query(
            r#"
            DELETE FROM command_idempotency_keys
            WHERE org_id = $1 AND principal_id = $2 AND idempotency_key = $3
              AND response IS NULL
            "#,
        )
        .bind(scope.org_id)
        .bind(scope.principal_id)
        .bind(&scope.key)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
