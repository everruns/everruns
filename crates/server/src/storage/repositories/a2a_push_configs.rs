use anyhow::Result;
use everruns_contracts::typed_id::SessionId;

use super::Database;
use crate::storage::{A2aPushConfigRow, UpsertA2aPushConfig};

impl Database {
    /// Create a push config, or replace the one with the same id on the task.
    pub async fn upsert_a2a_push_config(
        &self,
        input: UpsertA2aPushConfig,
    ) -> Result<A2aPushConfigRow> {
        Ok(sqlx::query_as(
            r#"
            INSERT INTO a2a_push_configs (
                id, org_id, session_id, config_id, url, auth_scheme,
                secrets_encrypted, wire_version
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (session_id, config_id) DO UPDATE SET
                url = EXCLUDED.url,
                auth_scheme = EXCLUDED.auth_scheme,
                secrets_encrypted = EXCLUDED.secrets_encrypted,
                wire_version = EXCLUDED.wire_version,
                updated_at = NOW()
            RETURNING *
            "#,
        )
        .bind(uuid::Uuid::now_v7())
        .bind(input.org_id)
        .bind(input.session_id)
        .bind(input.config_id)
        .bind(input.url)
        .bind(input.auth_scheme)
        .bind(input.secrets_encrypted)
        .bind(input.wire_version)
        .fetch_one(&self.pool)
        .await?)
    }

    /// A task's push configs, oldest first.
    pub async fn list_a2a_push_configs(
        &self,
        session_id: SessionId,
    ) -> Result<Vec<A2aPushConfigRow>> {
        Ok(sqlx::query_as(
            "SELECT * FROM a2a_push_configs WHERE session_id = $1 ORDER BY created_at, id",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn delete_a2a_push_config(
        &self,
        session_id: SessionId,
        config_id: &str,
    ) -> Result<bool> {
        let result =
            sqlx::query("DELETE FROM a2a_push_configs WHERE session_id = $1 AND config_id = $2")
                .bind(session_id)
                .bind(config_id)
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() > 0)
    }
}
