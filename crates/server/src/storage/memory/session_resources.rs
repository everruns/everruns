// In-memory storage: Session resource registry

use super::super::models::{SessionResourceRow, UpsertSessionResourceRow};
use super::InMemoryDatabase;
use anyhow::Result;
use everruns_core::SessionId;
use uuid::Uuid;

impl InMemoryDatabase {
    pub async fn upsert_session_resource(
        &self,
        input: UpsertSessionResourceRow,
    ) -> Result<SessionResourceRow> {
        let now = Self::now();
        let mut resources = self.session_resources.write();
        let key = (input.session_id, input.resource_id.clone());

        if let Some(existing) = resources.get_mut(&key) {
            existing.kind = input.kind;
            existing.display_name = input.display_name;
            existing.status = input.status;
            existing.metadata = input.metadata;
            existing.updated_at = now;
            return Ok(existing.clone());
        }

        let row = SessionResourceRow {
            id: Uuid::now_v7(),
            session_id: input.session_id,
            resource_id: input.resource_id,
            kind: input.kind,
            display_name: input.display_name,
            status: input.status,
            metadata: input.metadata,
            created_at: now,
            updated_at: now,
        };
        resources.insert(key, row.clone());
        Ok(row)
    }

    pub async fn update_session_resource_status(
        &self,
        session_id: SessionId,
        resource_id: &str,
        status: &str,
    ) -> Result<Option<SessionResourceRow>> {
        let now = Self::now();
        let mut resources = self.session_resources.write();
        let key = (session_id, resource_id.to_string());
        if let Some(row) = resources.get_mut(&key) {
            row.status = status.to_string();
            row.updated_at = now;
            return Ok(Some(row.clone()));
        }
        Ok(None)
    }

    pub async fn get_session_resource(
        &self,
        session_id: SessionId,
        resource_id: &str,
    ) -> Result<Option<SessionResourceRow>> {
        let resources = self.session_resources.read();
        let key = (session_id, resource_id.to_string());
        Ok(resources.get(&key).cloned())
    }

    pub async fn list_session_resources(
        &self,
        session_id: SessionId,
        kind: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<SessionResourceRow>> {
        let resources = self.session_resources.read();
        let mut result: Vec<_> = resources
            .values()
            .filter(|r| {
                r.session_id == session_id
                    && kind.is_none_or(|k| r.kind == k)
                    && status.is_none_or(|s| r.status == s)
            })
            .cloned()
            .collect();
        result.sort_by_key(|resource| resource.created_at);
        Ok(result)
    }

    pub async fn delete_session_resource(
        &self,
        session_id: SessionId,
        resource_id: &str,
    ) -> Result<bool> {
        let mut resources = self.session_resources.write();
        let key = (session_id, resource_id.to_string());
        Ok(resources.remove(&key).is_some())
    }
}
