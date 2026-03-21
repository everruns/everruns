// PostgreSQL repository: Audit Logs

use super::super::models::*;
use super::Database;
use anyhow::Result;
use chrono::{DateTime, Utc};
use uuid::Uuid;

impl Database {
    // Audit Logs (TM-OBS-007)

    pub async fn create_audit_log(&self, input: CreateAuditLogRow) -> Result<AuditLogRow> {
        let row = sqlx::query_as::<_, AuditLogRow>(
            r#"
            INSERT INTO audit_logs (org_id, actor_id, event_type, ip_address, metadata)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING *
            "#,
        )
        .bind(input.org_id)
        .bind(input.actor_id)
        .bind(&input.event_type)
        .bind(&input.ip_address)
        .bind(&input.metadata)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_audit_logs(
        &self,
        org_id: i64,
        limit: i64,
        before: Option<DateTime<Utc>>,
        event_type_prefix: Option<&str>,
        actor_id: Option<Uuid>,
    ) -> Result<Vec<AuditLogRow>> {
        // Build query dynamically based on filters
        let before_ts = before.unwrap_or_else(|| Utc::now() + chrono::Duration::seconds(1));
        let rows = if let Some(prefix) = event_type_prefix {
            if let Some(aid) = actor_id {
                sqlx::query_as::<_, AuditLogRow>(
                    r#"
                    SELECT * FROM audit_logs
                    WHERE org_id = $1 AND created_at < $2 AND event_type LIKE $3 AND actor_id = $4
                    ORDER BY created_at DESC
                    LIMIT $5
                    "#,
                )
                .bind(org_id)
                .bind(before_ts)
                .bind(format!("{}%", prefix))
                .bind(aid)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            } else {
                sqlx::query_as::<_, AuditLogRow>(
                    r#"
                    SELECT * FROM audit_logs
                    WHERE org_id = $1 AND created_at < $2 AND event_type LIKE $3
                    ORDER BY created_at DESC
                    LIMIT $4
                    "#,
                )
                .bind(org_id)
                .bind(before_ts)
                .bind(format!("{}%", prefix))
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
        } else if let Some(aid) = actor_id {
            sqlx::query_as::<_, AuditLogRow>(
                r#"
                SELECT * FROM audit_logs
                WHERE org_id = $1 AND created_at < $2 AND actor_id = $3
                ORDER BY created_at DESC
                LIMIT $4
                "#,
            )
            .bind(org_id)
            .bind(before_ts)
            .bind(aid)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, AuditLogRow>(
                r#"
                SELECT * FROM audit_logs
                WHERE org_id = $1 AND created_at < $2
                ORDER BY created_at DESC
                LIMIT $3
                "#,
            )
            .bind(org_id)
            .bind(before_ts)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        };

        Ok(rows)
    }

    pub async fn delete_audit_logs_before(&self, before: DateTime<Utc>) -> Result<u64> {
        let result = sqlx::query("DELETE FROM audit_logs WHERE created_at < $1")
            .bind(before)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }
}
