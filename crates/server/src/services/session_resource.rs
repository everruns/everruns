// Session resource service.
//
// Aggregates all resource types active in a session — leased resources
// (sandboxes, browsers) and subagents — into a unified SessionResource list
// for the API/UI.

use anyhow::Result;
use everruns_core::session::SubagentStatus;
use everruns_core::{SessionId, SessionResource};
use std::sync::Arc;

use crate::storage::backend::StorageBackend;
use crate::storage::leased_resource_row_to_domain;

/// Read-side service for session resources (leased resources + subagents).
pub struct SessionResourceService {
    db: Arc<StorageBackend>,
}

impl SessionResourceService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }

    /// List all resources for a session after validating org ownership.
    /// Merges leased resources and child sessions (subagents), sorted by created_at.
    pub async fn list_for_session(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Result<Option<Vec<SessionResource>>> {
        let Some(_session) = self.db.get_session(org_id, session_id).await? else {
            return Ok(None);
        };

        // Fetch both sources concurrently.
        let (leased_rows, child_rows) = tokio::try_join!(
            self.db.list_session_leased_resources(session_id),
            self.db.list_child_sessions(session_id),
        )?;

        let mut resources: Vec<SessionResource> =
            Vec::with_capacity(leased_rows.len() + child_rows.len());

        // Map leased resources.
        for row in &leased_rows {
            let lr = leased_resource_row_to_domain(row)?;
            resources.push(SessionResource::Leased { resource: lr });
        }

        // Map subagent sessions.
        for row in child_rows {
            let status = row
                .subagent_status
                .as_deref()
                .map(SubagentStatus::from)
                .unwrap_or(SubagentStatus::Spawning);
            resources.push(SessionResource::Subagent {
                session_id: row.id,
                name: row
                    .subagent_name
                    .unwrap_or_else(|| row.title.unwrap_or_default()),
                task: row.subagent_task,
                status,
                blueprint_id: row.blueprint_id,
                created_at: row.created_at,
                updated_at: row.updated_at,
            });
        }

        // Sort by created_at ascending so the UI gets chronological order.
        resources.sort_by_key(|r| match r {
            SessionResource::Leased { resource } => resource.created_at,
            SessionResource::Subagent { created_at, .. } => *created_at,
        });

        Ok(Some(resources))
    }
}
