// Session service for business logic (M2)
//
// Design Decision: Capability mounts are applied at session creation time.
// This ensures mounted files are available immediately when the session starts.
// The service collects mounts from the agent's capabilities and applies them
// to the session filesystem.

use crate::api::common::Pagination;
use crate::services::session_file::SessionFileService;
use crate::storage::{
    StorageBackend,
    models::{CreateSessionRow, UpdateSession},
};
use anyhow::Result;
use everruns_core::{
    CapabilityRegistry, Session, SessionStatus, TokenUsage, capabilities::collect_capabilities,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::api::sessions::{CreateSessionRequest, UpdateSessionRequest};

pub struct SessionService {
    db: Arc<StorageBackend>,
    capability_registry: CapabilityRegistry,
    session_file_service: SessionFileService,
}

impl SessionService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self {
            capability_registry: CapabilityRegistry::with_builtins(),
            session_file_service: SessionFileService::new(db.clone()),
            db,
        }
    }

    /// Create a new SessionService with a custom capability registry.
    pub fn with_registry(db: Arc<StorageBackend>, registry: CapabilityRegistry) -> Self {
        Self {
            capability_registry: registry,
            session_file_service: SessionFileService::new(db.clone()),
            db,
        }
    }

    pub async fn create(
        &self,
        org_id: i64,
        agent_id: Uuid,
        req: CreateSessionRequest,
    ) -> Result<Session> {
        // If model_id not provided, use the agent's default_model_id
        let model_id = match req.model_id {
            Some(id) => Some(id.uuid()),
            None => {
                // Look up the agent to get its default_model_id
                let agent = self.db.get_agent(org_id, agent_id).await?;
                agent.and_then(|a| a.default_model_id)
            }
        };

        let input = CreateSessionRow {
            agent_id,
            title: req.title,
            tags: req.tags,
            model_id,
        };
        let row = self.db.create_session(input).await?;
        let session = Self::row_to_session(row);

        // Apply capability mounts to the session filesystem
        self.apply_capability_mounts(agent_id, session.id.uuid())
            .await?;

        Ok(session)
    }

    /// Apply capability mounts to a session's filesystem.
    ///
    /// This method:
    /// 1. Gets the agent's enabled capabilities
    /// 2. Collects mount points from those capabilities
    /// 3. Creates the mounted files/directories in the session filesystem
    async fn apply_capability_mounts(
        &self,
        agent_id: Uuid,
        session_id: impl Into<uuid::Uuid> + Copy,
    ) -> Result<()> {
        let session_id = session_id.into();
        // Get agent's capability IDs
        let capability_rows = self.db.get_agent_capabilities(agent_id).await?;
        let capability_ids: Vec<String> = capability_rows
            .iter()
            .map(|r| r.capability_id.clone())
            .collect();

        if capability_ids.is_empty() {
            return Ok(()); // No capabilities, nothing to mount
        }

        // Collect mounts from capabilities
        let collected = collect_capabilities(&capability_ids, &self.capability_registry);

        if collected.mounts.is_empty() {
            return Ok(()); // No mounts to apply
        }

        // Apply mounts to session filesystem
        let result = self
            .session_file_service
            .apply_capability_mounts(session_id, &collected.mounts)
            .await?;

        if !result.is_success() {
            tracing::warn!(
                session_id = %session_id,
                agent_id = %agent_id,
                errors = ?result.errors,
                "Some capability mounts failed to apply"
            );
        } else {
            tracing::debug!(
                session_id = %session_id,
                agent_id = %agent_id,
                files_created = result.files_created,
                directories_created = result.directories_created,
                mount_points = result.mount_points_applied,
                "Capability mounts applied successfully"
            );
        }

        Ok(())
    }

    pub async fn get(&self, org_id: i64, id: Uuid) -> Result<Option<Session>> {
        let row = self.db.get_session(org_id, id).await?;
        Ok(row.map(Self::row_to_session))
    }

    /// List sessions for an agent with pagination.
    /// Returns (sessions, total_count).
    /// Sessions include preview text from first user message and last assistant response.
    pub async fn list(
        &self,
        org_id: i64,
        agent_id: Uuid,
        pagination: Pagination,
    ) -> Result<(Vec<Session>, u32)> {
        let (rows, total) = self.db.list_sessions(org_id, agent_id, pagination).await?;
        let mut sessions: Vec<Session> = rows.into_iter().map(Self::row_to_session).collect();

        // Fetch previews for all sessions in batch queries
        let session_ids: Vec<Uuid> = sessions.iter().map(|s| s.id.uuid()).collect();
        let input_previews = self.db.get_session_previews(&session_ids).await?;
        let output_previews = self.db.get_session_output_previews(&session_ids).await?;

        // Populate previews for each session
        for session in &mut sessions {
            if let Some(preview) = input_previews.get(&session.id.uuid()) {
                session.preview = Some(preview.clone());
            }
            if let Some(preview) = output_previews.get(&session.id.uuid()) {
                session.output_preview = Some(preview.clone());
            }
        }

        Ok((sessions, total))
    }

    pub async fn update(
        &self,
        org_id: i64,
        id: Uuid,
        req: UpdateSessionRequest,
    ) -> Result<Option<Session>> {
        let input = UpdateSession {
            title: req.title,
            tags: req.tags,
            ..Default::default()
        };
        let row = self.db.update_session(org_id, id, input).await?;
        Ok(row.map(Self::row_to_session))
    }

    /// Update session status (used by worker via gRPC)
    pub async fn update_status(
        &self,
        org_id: i64,
        id: Uuid,
        status: String,
    ) -> Result<Option<Session>> {
        let input = UpdateSession {
            status: Some(status),
            ..Default::default()
        };
        let row = self.db.update_session(org_id, id, input).await?;
        Ok(row.map(Self::row_to_session))
    }

    pub async fn delete(&self, org_id: i64, id: Uuid) -> Result<bool> {
        self.db.delete_session(org_id, id).await
    }

    fn row_to_session(row: crate::storage::SessionRow) -> Session {
        // Convert database usage columns to TokenUsage
        let usage = if row.total_input_tokens > 0 || row.total_output_tokens > 0 {
            Some(TokenUsage::with_cache(
                row.total_input_tokens as u32,
                row.total_output_tokens as u32,
                if row.total_cache_read_tokens > 0 {
                    Some(row.total_cache_read_tokens as u32)
                } else {
                    None
                },
                if row.total_cache_creation_tokens > 0 {
                    Some(row.total_cache_creation_tokens as u32)
                } else {
                    None
                },
            ))
        } else {
            None
        };

        Session {
            id: row.id.into(),
            agent_id: row.agent_id.into(),
            title: row.title,
            preview: None,        // Populated separately in list()
            output_preview: None, // Populated separately in list()
            tags: row.tags,
            model_id: row.model_id.map(|id| id.into()),
            status: SessionStatus::from(row.status.as_str()),
            created_at: row.created_at,
            started_at: row.started_at,
            finished_at: row.finished_at,
            usage,
        }
    }
}
