// PostgreSQL storage: HTTP signing keys CRUD

use super::super::Database;
use super::super::models::*;
use anyhow::Result;

impl Database {
    pub async fn upsert_http_signing_key(
        &self,
        org_id: i64,
        input: UpsertHttpSigningKey,
    ) -> Result<HttpSigningKeyRow> {
        let row = sqlx::query_as::<_, HttpSigningKeyRow>(
            r#"
            INSERT INTO http_signing_keys (org_id, key_id, public_key, label, expires_at)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (org_id, key_id) DO UPDATE
                SET public_key = EXCLUDED.public_key,
                    label = EXCLUDED.label,
                    expires_at = EXCLUDED.expires_at
            RETURNING id, org_id, key_id, public_key, label, created_at, expires_at
            "#,
        )
        .bind(org_id)
        .bind(&input.key_id)
        .bind(&input.public_key)
        .bind(&input.label)
        .bind(input.expires_at)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_http_signing_keys(&self, org_id: i64) -> Result<Vec<HttpSigningKeyRow>> {
        let rows = sqlx::query_as::<_, HttpSigningKeyRow>(
            r#"
            SELECT id, org_id, key_id, public_key, label, created_at, expires_at
            FROM http_signing_keys
            WHERE org_id = $1
              AND (expires_at IS NULL OR expires_at > now())
            ORDER BY created_at ASC
            "#,
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn delete_http_signing_key(&self, org_id: i64, key_id: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM http_signing_keys WHERE org_id = $1 AND key_id = $2")
            .bind(org_id)
            .bind(key_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn list_all_http_signing_keys(&self) -> Result<Vec<HttpSigningKeyRow>> {
        let rows = sqlx::query_as::<_, HttpSigningKeyRow>(
            r#"
            SELECT id, org_id, key_id, public_key, label, created_at, expires_at
            FROM http_signing_keys
            WHERE expires_at IS NULL OR expires_at > now()
            ORDER BY created_at ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}
