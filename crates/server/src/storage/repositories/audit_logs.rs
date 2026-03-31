// PostgreSQL repository: Audit Logs

use super::super::models::*;
use super::Database;
use anyhow::Result;
use chrono::{DateTime, Utc};

impl Database {
    // Audit Logs (TM-OBS-007, EVE-226)

    pub async fn create_audit_log(&self, input: CreateAuditLogRow) -> Result<AuditLogRow> {
        let row = sqlx::query_as::<_, AuditLogRow>(
            r#"
            INSERT INTO audit_logs (org_id, actor_id, event_type, ip_address, metadata, domain, action, target_type, target_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            RETURNING *
            "#,
        )
        .bind(input.org_id)
        .bind(input.actor_id)
        .bind(&input.event_type)
        .bind(&input.ip_address)
        .bind(&input.metadata)
        .bind(&input.domain)
        .bind(&input.action)
        .bind(&input.target_type)
        .bind(&input.target_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_audit_logs(&self, query: AuditLogQuery<'_>) -> Result<Vec<AuditLogRow>> {
        let before_ts = query
            .before
            .unwrap_or_else(|| Utc::now() + chrono::Duration::seconds(1));

        let rows = sqlx::query_as::<_, AuditLogRow>(
            r#"
            SELECT * FROM audit_logs
            WHERE org_id = $1
              AND created_at < $2
              AND ($3::text IS NULL OR event_type LIKE $3)
              AND ($4::uuid IS NULL OR actor_id = $4)
              AND ($5::text IS NULL OR domain = $5)
              AND ($6::text IS NULL OR action = $6)
            ORDER BY created_at DESC
            LIMIT $7
            "#,
        )
        .bind(query.org_id)
        .bind(before_ts)
        .bind(query.event_type_prefix.map(|p| format!("{p}%")))
        .bind(query.actor_id)
        .bind(query.domain)
        .bind(query.action)
        .bind(query.limit)
        .fetch_all(&self.pool)
        .await?;

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
