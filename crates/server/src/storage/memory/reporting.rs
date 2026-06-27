use anyhow::Result;
use chrono::Utc;
use uuid::Uuid;

use super::InMemoryDatabase;
use crate::storage::reporting::models::ReportingOutboxRow;

impl InMemoryDatabase {
    pub async fn enqueue_reporting_outbox(
        &self,
        org_id: i64,
        source_type: &str,
        source_id: &str,
        source_version: Option<&str>,
        reason: &str,
    ) -> Result<()> {
        let source_version = source_version.unwrap_or("").to_string();
        let key = (
            org_id,
            source_type.to_string(),
            source_id.to_string(),
            source_version.clone(),
            reason.to_string(),
        );
        let now = Utc::now();
        let mut outbox = self.reporting_outbox.write();

        if let Some(existing) = outbox.get_mut(&key) {
            if existing.status == "completed" {
                existing.status = "pending".to_string();
            }
            existing.next_attempt_at = now;
            existing.updated_at = now;
            return Ok(());
        }

        outbox.insert(
            key,
            ReportingOutboxRow {
                id: Uuid::now_v7(),
                org_id,
                source_type: source_type.to_string(),
                source_id: source_id.to_string(),
                source_version,
                reason: reason.to_string(),
                status: "pending".to_string(),
                attempts: 0,
                next_attempt_at: now,
                last_error: None,
                created_at: now,
                updated_at: now,
            },
        );

        Ok(())
    }

    pub async fn list_reporting_outbox(
        &self,
        org_id: i64,
        source_type: &str,
        source_id: &str,
        reason: &str,
    ) -> Result<Vec<ReportingOutboxRow>> {
        let mut rows: Vec<_> = self
            .reporting_outbox
            .read()
            .values()
            .filter(|row| {
                row.org_id == org_id
                    && row.source_type == source_type
                    && row.source_id == source_id
                    && row.reason == reason
            })
            .cloned()
            .collect();
        rows.sort_by_key(|row| row.created_at);
        Ok(rows)
    }
}
