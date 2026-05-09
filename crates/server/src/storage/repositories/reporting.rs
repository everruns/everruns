use anyhow::Result;

use super::Database;

impl Database {
    pub async fn enqueue_reporting_outbox(
        &self,
        org_id: i64,
        source_type: &str,
        source_id: &str,
        source_version: Option<&str>,
        reason: &str,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO reporting_outbox (
                org_id, source_type, source_id, source_version, reason, status,
                next_attempt_at
            )
            VALUES ($1, $2, $3, $4, $5, 'pending', NOW())
            ON CONFLICT (org_id, source_type, source_id, source_version, reason)
            DO UPDATE SET
                status = CASE
                    WHEN reporting_outbox.status = 'completed' THEN 'pending'
                    ELSE reporting_outbox.status
                END,
                next_attempt_at = NOW(),
                updated_at = NOW()
            "#,
        )
        .bind(org_id)
        .bind(source_type)
        .bind(source_id)
        .bind(source_version.unwrap_or(""))
        .bind(reason)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
