// Database-backed session resource registry.
//
// Decision: Implements the generic SessionResourceRegistry trait from core.
// Any capability can register resources here for agent and infra visibility.

use async_trait::async_trait;
use everruns_core::session_resource::{
    RegisterSessionResource, SessionResourceEntry, SessionResourceFilter, SessionResourceStatus,
};
use everruns_core::traits::SessionResourceRegistry;
use everruns_core::{AgentLoopError, Result, SessionId};

use super::backend::StorageBackend;
use super::models::UpsertSessionResourceRow;
use std::sync::Arc;

/// Convert a storage row into the public domain type.
fn row_to_entry(row: &super::models::SessionResourceRow) -> Result<SessionResourceEntry> {
    Ok(SessionResourceEntry {
        resource_id: row.resource_id.clone(),
        session_id: row.session_id,
        kind: row.kind.clone(),
        display_name: row.display_name.clone(),
        status: SessionResourceStatus::from(row.status.as_str()),
        metadata: row.metadata.clone(),
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// Database-backed session resource registry.
#[derive(Clone)]
pub struct DbSessionResourceRegistry {
    db: Arc<StorageBackend>,
}

impl DbSessionResourceRegistry {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl SessionResourceRegistry for DbSessionResourceRegistry {
    async fn register(&self, entry: RegisterSessionResource) -> Result<SessionResourceEntry> {
        let row = self
            .db
            .upsert_session_resource(UpsertSessionResourceRow {
                session_id: entry.session_id,
                resource_id: entry.resource_id,
                kind: entry.kind,
                display_name: entry.display_name,
                status: entry.status.to_string(),
                metadata: entry.metadata,
            })
            .await
            .map_err(|e| {
                AgentLoopError::store(format!("Failed to register session resource: {e}"))
            })?;

        row_to_entry(&row)
    }

    async fn update_status(
        &self,
        session_id: SessionId,
        resource_id: &str,
        status: SessionResourceStatus,
    ) -> Result<Option<SessionResourceEntry>> {
        let row = self
            .db
            .update_session_resource_status(session_id, resource_id, &status.to_string())
            .await
            .map_err(|e| {
                AgentLoopError::store(format!("Failed to update session resource status: {e}"))
            })?;

        row.as_ref().map(row_to_entry).transpose()
    }

    async fn get(
        &self,
        session_id: SessionId,
        resource_id: &str,
    ) -> Result<Option<SessionResourceEntry>> {
        let row = self
            .db
            .get_session_resource(session_id, resource_id)
            .await
            .map_err(|e| AgentLoopError::store(format!("Failed to get session resource: {e}")))?;

        row.as_ref().map(row_to_entry).transpose()
    }

    async fn list(
        &self,
        session_id: SessionId,
        filter: Option<&SessionResourceFilter>,
    ) -> Result<Vec<SessionResourceEntry>> {
        let kind = filter.and_then(|f| f.kind.as_deref());
        let status = filter.and_then(|f| f.status.map(|s| s.to_string()));
        let rows = self
            .db
            .list_session_resources(session_id, kind, status.as_deref())
            .await
            .map_err(|e| AgentLoopError::store(format!("Failed to list session resources: {e}")))?;

        rows.iter().map(row_to_entry).collect()
    }

    async fn deregister(&self, session_id: SessionId, resource_id: &str) -> Result<bool> {
        self.db
            .delete_session_resource(session_id, resource_id)
            .await
            .map_err(|e| {
                AgentLoopError::store(format!("Failed to deregister session resource: {e}"))
            })
    }
}
