// Session service for business logic (M2)
//
// Design Decision: Capability mounts are applied at session creation time.
// This ensures mounted files are available immediately when the session starts.
// The service collects mounts from the agent's capabilities and applies them
// to the session filesystem. Session capabilities are applied after agent capabilities
// (additive behavior).

use crate::api::common::Pagination;
use crate::services::session_file::SessionFileService;
use crate::storage::{
    StorageBackend,
    models::{CreateSessionRow, UpdateSession},
};
use anyhow::Result;
use everruns_core::{
    AgentCapabilityConfig, AgentId, CapabilityRegistry, HarnessId, ModelId, Session, SessionId,
    SessionStatus, TokenUsage, capabilities::collect_capabilities,
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
        org_public_id: &str,
        harness_id: Uuid,
        agent_internal_id: Option<Uuid>,
        agent_public_id: Option<AgentId>,
        req: CreateSessionRequest,
    ) -> Result<Session> {
        let harness_id = HarnessId::from_uuid(harness_id);
        let agent_id = agent_internal_id.map(AgentId::from_uuid);

        // Resolve model_id: session > agent > harness
        let model_id: Option<ModelId> = match req.model_id {
            Some(id) => Some(id),
            None => {
                // Try agent's default_model_id first
                let agent_model = if let Some(aid) = agent_id {
                    let agent = self.db.get_agent(org_id, aid).await?;
                    agent.and_then(|a| a.default_model_id)
                } else {
                    None
                };
                // Fall back to harness's default_model_id
                match agent_model {
                    Some(id) => Some(id),
                    None => {
                        let harness = self.db.get_harness(org_id, harness_id).await?;
                        harness.and_then(|h| h.default_model_id)
                    }
                }
            }
        };

        // Serialize capabilities to JSON for storage
        let capabilities_json = serde_json::to_value(&req.capabilities)?;

        let input = CreateSessionRow {
            org_id,
            harness_id: Some(harness_id),
            agent_id,
            title: req.title,
            tags: req.tags,
            model_id,
            capabilities: capabilities_json,
            tools: serde_json::to_value(&req.tools).unwrap_or_default(),
        };
        let row = self.db.create_session(input).await?;
        let mut session = Self::row_to_session(row, org_public_id);
        // Override agent_id with public_id (DB stores internal UUID as FK)
        session.agent_id = agent_public_id;

        // Apply capability mounts (harness + agent + session capabilities)
        self.apply_capability_mounts(
            harness_id.uuid(),
            agent_id.map(|a| a.uuid()),
            &req.capabilities,
            session.id.uuid(),
        )
        .await?;

        Ok(session)
    }

    /// Apply capability mounts to a session's filesystem.
    ///
    /// Collects mounts from harness + agent + session capabilities.
    async fn apply_capability_mounts(
        &self,
        harness_id: Uuid,
        agent_id: Option<Uuid>,
        session_capabilities: &[AgentCapabilityConfig],
        session_id: impl Into<uuid::Uuid> + Copy,
    ) -> Result<()> {
        let session_id = session_id.into();

        // Collect capability IDs: harness caps first, then agent, then session
        let harness_cap_rows = self.db.get_harness_capabilities(harness_id).await?;
        let mut capability_ids: Vec<String> = harness_cap_rows
            .iter()
            .map(|r| r.capability_id.clone())
            .collect();

        // Add agent's capability IDs
        if let Some(agent_id) = agent_id {
            let agent_cap_rows = self.db.get_agent_capabilities(agent_id).await?;
            for r in &agent_cap_rows {
                if !capability_ids.contains(&r.capability_id) {
                    capability_ids.push(r.capability_id.clone());
                }
            }
        }

        // Add session-level capabilities (additive)
        for cap in session_capabilities {
            let cap_id = cap.capability_id().to_string();
            if !capability_ids.contains(&cap_id) {
                capability_ids.push(cap_id);
            }
        }

        if capability_ids.is_empty() {
            return Ok(()); // No capabilities, nothing to mount
        }

        // Collect mounts from all capabilities
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
                agent_id = ?agent_id,
                errors = ?result.errors,
                "Some capability mounts failed to apply"
            );
        } else {
            tracing::debug!(
                session_id = %session_id,
                agent_id = ?agent_id,
                files_created = result.files_created,
                directories_created = result.directories_created,
                mount_points = result.mount_points_applied,
                "Capability mounts applied successfully"
            );
        }

        Ok(())
    }

    pub async fn get(&self, org_id: i64, org_public_id: &str, id: Uuid) -> Result<Option<Session>> {
        let row = self
            .db
            .get_session(org_id, SessionId::from_uuid(id))
            .await?;
        match row {
            Some(r) => {
                let mut session = Self::row_to_session(r, org_public_id);
                self.resolve_session_agent_id(org_id, &mut session).await?;
                Ok(Some(session))
            }
            None => Ok(None),
        }
    }

    /// List sessions for an organization with optional agent filter.
    /// Returns (sessions, total_count).
    /// Sessions include preview text from first user message and last assistant response.
    pub async fn list(
        &self,
        org_id: i64,
        org_public_id: &str,
        agent_id: Option<Uuid>,
        pagination: Pagination,
    ) -> Result<(Vec<Session>, u32)> {
        let agent_id = agent_id.map(AgentId::from_uuid);
        let (rows, total) = self.db.list_sessions(org_id, agent_id, pagination).await?;
        let mut sessions: Vec<Session> = rows
            .into_iter()
            .map(|r| Self::row_to_session(r, org_public_id))
            .collect();

        // Resolve agent internal UUIDs to public IDs
        for session in &mut sessions {
            self.resolve_session_agent_id(org_id, session).await?;
        }

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
        org_public_id: &str,
        id: Uuid,
        req: UpdateSessionRequest,
    ) -> Result<Option<Session>> {
        let input = UpdateSession {
            title: req.title,
            tags: req.tags,
            ..Default::default()
        };
        let row = self
            .db
            .update_session(org_id, SessionId::from_uuid(id), input)
            .await?;
        match row {
            Some(r) => {
                let mut session = Self::row_to_session(r, org_public_id);
                self.resolve_session_agent_id(org_id, &mut session).await?;
                Ok(Some(session))
            }
            None => Ok(None),
        }
    }

    /// Update session status (used by worker via gRPC)
    pub async fn update_status(
        &self,
        org_id: i64,
        org_public_id: &str,
        id: Uuid,
        status: String,
    ) -> Result<Option<Session>> {
        let input = UpdateSession {
            status: Some(status),
            ..Default::default()
        };
        let row = self
            .db
            .update_session(org_id, SessionId::from_uuid(id), input)
            .await?;
        match row {
            Some(r) => {
                let mut session = Self::row_to_session(r, org_public_id);
                self.resolve_session_agent_id(org_id, &mut session).await?;
                Ok(Some(session))
            }
            None => Ok(None),
        }
    }

    pub async fn delete(&self, org_id: i64, id: Uuid) -> Result<bool> {
        self.db
            .delete_session(org_id, SessionId::from_uuid(id))
            .await
    }

    /// Resolve a session's agent_id from internal UUID to the agent's public_id.
    /// The DB stores the internal UUID as FK; the API should return the public_id.
    async fn resolve_session_agent_id(&self, org_id: i64, session: &mut Session) -> Result<()> {
        if let Some(aid) = session.agent_id
            && let Some(public_id) = self.db.get_agent_public_id(org_id, aid).await?
            && let Ok(agent_id) = public_id.parse::<AgentId>()
        {
            session.agent_id = Some(agent_id);
        }
        Ok(())
    }

    fn row_to_session(row: crate::storage::SessionRow, org_public_id: &str) -> Session {
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

        // Parse capabilities from JSON
        let capabilities: Vec<AgentCapabilityConfig> =
            serde_json::from_value(row.capabilities).unwrap_or_default();

        Session {
            id: row.id,
            organization_id: org_public_id.to_string(),
            harness_id: row.harness_id.unwrap_or_else(|| HarnessId::from_seed(1)),
            agent_id: row.agent_id,
            title: row.title,
            preview: None,        // Populated separately in list()
            output_preview: None, // Populated separately in list()
            tags: row.tags,
            model_id: row.model_id,
            capabilities,
            tools: serde_json::from_value(row.tools).unwrap_or_default(),
            status: SessionStatus::from(row.status.as_str()),
            created_at: row.created_at,
            updated_at: row.updated_at,
            started_at: row.started_at,
            finished_at: row.finished_at,
            usage,
        }
    }
}
