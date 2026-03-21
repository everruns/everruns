// In-memory storage: Audit Logs

use super::super::models::*;
use super::InMemoryDatabase;
use anyhow::Result;
use chrono::{DateTime, Utc};
use uuid::Uuid;

impl InMemoryDatabase {
    // Audit Logs (TM-OBS-007)

    pub async fn create_audit_log(&self, input: CreateAuditLogRow) -> Result<AuditLogRow> {
        let row = AuditLogRow {
            id: Uuid::now_v7(),
            org_id: input.org_id,
            actor_id: input.actor_id,
            event_type: input.event_type,
            ip_address: input.ip_address,
            metadata: input.metadata,
            created_at: Self::now(),
        };
        self.audit_logs.write().push(row.clone());
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
        let logs = self.audit_logs.read();
        let before_ts = before.unwrap_or_else(|| Utc::now() + chrono::Duration::seconds(1));
        let mut filtered: Vec<_> = logs
            .iter()
            .filter(|r| {
                r.org_id == org_id
                    && r.created_at < before_ts
                    && event_type_prefix.is_none_or(|p| r.event_type.starts_with(p))
                    && actor_id.is_none_or(|a| r.actor_id == Some(a))
            })
            .cloned()
            .collect();
        filtered.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        filtered.truncate(limit as usize);
        Ok(filtered)
    }

    pub async fn delete_audit_logs_before(&self, before: DateTime<Utc>) -> Result<u64> {
        let mut logs = self.audit_logs.write();
        let initial = logs.len();
        logs.retain(|r| r.created_at >= before);
        Ok((initial - logs.len()) as u64)
    }
}
