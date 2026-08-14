use anyhow::Result;
use everruns_provider::typed_id::SessionId;

use super::Database;
use crate::storage::models::{CompactionCheckpointRow, InstallCompactionCheckpointRow};

impl Database {
    pub async fn get_compaction_checkpoint(
        &self,
        session_id: SessionId,
        provider_type: &str,
        model: &str,
        format_version: i32,
    ) -> Result<Option<CompactionCheckpointRow>> {
        Ok(sqlx::query_as(
            r#"
            SELECT id, session_id, source_sequence, provider_type, model,
                   format_version, payload_encrypted
            FROM session_compaction_checkpoints
            WHERE session_id = $1 AND provider_type = $2 AND model = $3
              AND format_version = $4
            "#,
        )
        .bind(session_id)
        .bind(provider_type)
        .bind(model)
        .bind(format_version)
        .fetch_optional(self.pool())
        .await?)
    }

    pub async fn install_compaction_checkpoint(
        &self,
        input: InstallCompactionCheckpointRow,
    ) -> Result<bool> {
        let result = sqlx::query(
            r#"
            INSERT INTO session_compaction_checkpoints
                (id, session_id, source_sequence, provider_type, model,
                 format_version, payload_encrypted)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT (session_id, provider_type, model, format_version)
            DO UPDATE SET
                id = EXCLUDED.id,
                source_sequence = EXCLUDED.source_sequence,
                payload_encrypted = EXCLUDED.payload_encrypted,
                updated_at = NOW()
            WHERE session_compaction_checkpoints.source_sequence
                < EXCLUDED.source_sequence
            "#,
        )
        .bind(input.id)
        .bind(input.session_id)
        .bind(input.source_sequence)
        .bind(input.provider_type)
        .bind(input.model)
        .bind(input.format_version)
        .bind(input.payload_encrypted)
        .execute(self.pool())
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn copy_compaction_checkpoints(
        &self,
        source_session_id: SessionId,
        target_session_id: SessionId,
        through_sequence: i32,
    ) -> Result<u64> {
        let result = sqlx::query(
            r#"
            INSERT INTO session_compaction_checkpoints
                (id, session_id, source_sequence, provider_type, model,
                 format_version, payload_encrypted)
            SELECT gen_random_uuid(), $2, source_sequence, provider_type, model,
                   format_version, payload_encrypted
            FROM session_compaction_checkpoints
            WHERE session_id = $1 AND source_sequence <= $3
            ON CONFLICT (session_id, provider_type, model, format_version)
            DO NOTHING
            "#,
        )
        .bind(source_session_id)
        .bind(target_session_id)
        .bind(through_sequence)
        .execute(self.pool())
        .await?;
        Ok(result.rows_affected())
    }
}
