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
    AgentCapabilityConfig, AgentId, Caller, CapabilityRegistry, HarnessId, ModelId, Permission,
    Policy, Rule, Session, SessionId, SessionStatus, SubagentStatus, TokenUsage,
    capabilities::{SystemPromptContext, collect_capabilities, compute_features},
};
use everruns_macros::policy;
use std::sync::Arc;
use uuid::Uuid;

use crate::api::sessions::{CreateSessionRequest, UpdateSessionRequest};

/// Policy: View sessions (read-only).
pub const SESSION_VIEW: Policy = Policy {
    id: "session.view",
    rules: &[Rule::UserHasPermission(Permission::OrgSessionsManage)],
};

/// Policy: Manage sessions (create, update, delete).
pub const SESSION_MANAGE: Policy = Policy {
    id: "session.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgSessionsManage)],
};

/// Session counts grouped by status.
#[derive(Debug, Clone, Default)]
pub struct SessionStats {
    pub total: u32,
    pub active: u32,
    pub idle: u32,
    pub started: u32,
    pub waiting_for_tool_results: u32,
}

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

    #[policy(SESSION_MANAGE)]
    pub async fn create(
        &self,
        caller: &Caller,
        harness_id: Uuid,
        agent_internal_id: Option<Uuid>,
        agent_public_id: Option<AgentId>,
        req: CreateSessionRequest,
    ) -> Result<Session> {
        let org_id = caller.org_id;
        let org_public_id = &caller.org_public_id;
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

        // Populate features before overriding agent_id (needs internal UUID)
        self.populate_features(&mut session).await?;

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

        let capability_ids = self
            .collect_session_capability_ids(harness_id, agent_id, session_capabilities)
            .await?;

        if capability_ids.is_empty() {
            return Ok(()); // No capabilities, nothing to mount
        }

        // Collect mounts from all capabilities
        let ctx = SystemPromptContext::without_file_store(SessionId::from_uuid(session_id));
        let collected =
            collect_capabilities(&capability_ids, &self.capability_registry, &ctx).await;

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

    #[policy(SESSION_VIEW)]
    pub async fn get(
        &self,
        caller: &Caller,
        id: Uuid,
        user_id: Option<Uuid>,
    ) -> Result<Option<Session>> {
        let row = self
            .db
            .get_session(caller.org_id, SessionId::from_uuid(id))
            .await?;
        match row {
            Some(r) => {
                let mut session = Self::row_to_session(r, &caller.org_public_id);
                // Populate features before resolving agent_id (needs internal UUID)
                self.populate_features(&mut session).await?;
                self.resolve_session_agent_id(caller.org_id, &mut session)
                    .await?;
                // Populate is_pinned if user context available
                if let Some(uid) = user_id {
                    let pinned = self.db.list_pinned_session_ids(uid, caller.org_id).await?;
                    session.is_pinned = Some(pinned.iter().any(|s| s.uuid() == id));
                }
                Ok(Some(session))
            }
            None => Ok(None),
        }
    }

    /// Get session counts grouped by status for an organization.
    #[policy(SESSION_VIEW)]
    pub async fn stats(&self, caller: &Caller) -> Result<SessionStats> {
        let counts = self.db.count_sessions_by_status(caller.org_id).await?;
        let mut stats = SessionStats::default();
        for (status, count) in counts {
            let count = count as u32;
            stats.total += count;
            match status.as_str() {
                "active" => stats.active = count,
                "idle" => stats.idle = count,
                "started" => stats.started = count,
                "waiting_for_tool_results" => stats.waiting_for_tool_results = count,
                _ => {} // ignore unknown statuses
            }
        }
        Ok(stats)
    }

    /// List sessions for an organization with optional agent filter.
    /// Returns (sessions, total_count).
    /// Sessions include preview text from first user message and last assistant response.
    #[policy(SESSION_VIEW)]
    pub async fn list(
        &self,
        caller: &Caller,
        agent_id: Option<Uuid>,
        user_id: Option<Uuid>,
        search: Option<&str>,
        pagination: Pagination,
    ) -> Result<(Vec<Session>, u32)> {
        let org_id = caller.org_id;
        let org_public_id = &caller.org_public_id;
        let agent_id = agent_id.map(AgentId::from_uuid);
        let (rows, total) = self
            .db
            .list_sessions(org_id, agent_id, search, pagination)
            .await?;
        let mut sessions: Vec<Session> = rows
            .into_iter()
            .map(|r| Self::row_to_session(r, org_public_id))
            .collect();

        // Populate features before resolving agent IDs (needs internal UUIDs)
        for session in &mut sessions {
            self.populate_features(session).await?;
        }

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

        // Populate is_pinned if user context available
        if let Some(uid) = user_id {
            let pinned_ids = self.db.list_pinned_session_ids(uid, org_id).await?;
            let pinned_set: std::collections::HashSet<Uuid> =
                pinned_ids.iter().map(|id| id.uuid()).collect();
            for session in &mut sessions {
                session.is_pinned = Some(pinned_set.contains(&session.id.uuid()));
            }
        }

        Ok((sessions, total))
    }

    #[policy(SESSION_MANAGE)]
    pub async fn update(
        &self,
        caller: &Caller,
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
            .update_session(caller.org_id, SessionId::from_uuid(id), input)
            .await?;
        match row {
            Some(r) => {
                let mut session = Self::row_to_session(r, &caller.org_public_id);
                self.resolve_session_agent_id(caller.org_id, &mut session)
                    .await?;
                Ok(Some(session))
            }
            None => Ok(None),
        }
    }

    /// Update session status (used by worker via gRPC)
    #[policy(SESSION_MANAGE)]
    pub async fn update_status(
        &self,
        caller: &Caller,
        id: Uuid,
        status: String,
    ) -> Result<Option<Session>> {
        let input = UpdateSession {
            status: Some(status),
            ..Default::default()
        };
        let row = self
            .db
            .update_session(caller.org_id, SessionId::from_uuid(id), input)
            .await?;
        match row {
            Some(r) => {
                let mut session = Self::row_to_session(r, &caller.org_public_id);
                self.resolve_session_agent_id(caller.org_id, &mut session)
                    .await?;
                Ok(Some(session))
            }
            None => Ok(None),
        }
    }

    /// Get or create the global chat session for a user.
    /// Uses tags for per-user singleton: `["global-chat", "user:{user_id}"]`.
    /// Creates with the Platform Chat harness if no existing session is found.
    #[policy(SESSION_MANAGE)]
    pub async fn get_or_create_chat_session(
        &self,
        caller: &Caller,
        user_id: Uuid,
        harness_id: Uuid,
    ) -> Result<Session> {
        let org_id = caller.org_id;
        let org_public_id = &caller.org_public_id;
        let user_tag = format!("user:{}", user_id);
        let tags = vec!["global-chat".to_string(), user_tag.clone()];

        // Look for existing chat session
        if let Some(row) = self.db.find_session_by_tags(org_id, &tags).await? {
            let mut session = Self::row_to_session(row, org_public_id);
            self.populate_features(&mut session).await?;
            self.resolve_session_agent_id(org_id, &mut session).await?;
            return Ok(session);
        }

        // Create a new chat session
        let harness_id_typed = HarnessId::from_uuid(harness_id);
        let input = CreateSessionRow {
            org_id,
            harness_id: Some(harness_id_typed),
            agent_id: None,
            title: Some("Platform Chat".to_string()),
            tags: vec!["global-chat".to_string(), user_tag],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
        };
        let row = self.db.create_session(input).await?;
        let session_id = row.id.uuid();
        let mut session = Self::row_to_session(row, org_public_id);
        self.populate_features(&mut session).await?;

        // Apply capability mounts
        self.apply_capability_mounts(harness_id, None, &[], session_id)
            .await?;

        Ok(session)
    }

    #[policy(SESSION_MANAGE)]
    pub async fn delete(&self, caller: &Caller, id: Uuid) -> Result<bool> {
        self.db
            .delete_session(caller.org_id, SessionId::from_uuid(id))
            .await
    }

    /// Pin a session for a user
    #[policy(SESSION_MANAGE)]
    pub async fn pin(&self, caller: &Caller, user_id: Uuid, session_id: Uuid) -> Result<()> {
        self.db
            .pin_session(user_id, SessionId::from_uuid(session_id), caller.org_id)
            .await
    }

    /// Unpin a session for a user
    #[policy(SESSION_MANAGE)]
    pub async fn unpin(&self, caller: &Caller, user_id: Uuid, session_id: Uuid) -> Result<bool> {
        self.db
            .unpin_session(user_id, SessionId::from_uuid(session_id))
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

    /// Collect all capability IDs for a session (harness + agent + session-level).
    /// Deduplicates while preserving order: harness first, then agent, then session.
    async fn collect_session_capability_ids(
        &self,
        harness_id: Uuid,
        agent_id: Option<Uuid>,
        session_capabilities: &[AgentCapabilityConfig],
    ) -> Result<Vec<String>> {
        let harness_cap_rows = self.db.get_harness_capabilities(harness_id).await?;
        let mut capability_ids: Vec<String> = harness_cap_rows
            .iter()
            .map(|r| r.capability_id.clone())
            .collect();

        if let Some(agent_id) = agent_id {
            let agent_cap_rows = self.db.get_agent_capabilities(agent_id).await?;
            for r in &agent_cap_rows {
                if !capability_ids.contains(&r.capability_id) {
                    capability_ids.push(r.capability_id.clone());
                }
            }
        }

        for cap in session_capabilities {
            let cap_id = cap.capability_id().to_string();
            if !capability_ids.contains(&cap_id) {
                capability_ids.push(cap_id);
            }
        }

        Ok(capability_ids)
    }

    /// Populate the `features` field on a session by aggregating features from
    /// all active capabilities (harness + agent + session-level).
    ///
    /// Must be called BEFORE `resolve_session_agent_id()` because the session's
    /// agent_id at that point is still the internal UUID needed for DB lookups.
    async fn populate_features(&self, session: &mut Session) -> Result<()> {
        let harness_id = session.harness_id.uuid();
        let agent_internal_id = session.agent_id.map(|a| a.uuid());

        let capability_ids = self
            .collect_session_capability_ids(harness_id, agent_internal_id, &session.capabilities)
            .await?;

        session.features = compute_features(&capability_ids, &self.capability_registry);
        Ok(())
    }

    pub fn row_to_session(row: crate::storage::SessionRow, org_public_id: &str) -> Session {
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
            is_pinned: None,             // Populated by caller with user context
            active_schedule_count: None, // Populated by caller
            features: vec![],            // Populated by caller via populate_features()
            parent_session_id: row.parent_session_id,
            subagent_name: row.subagent_name,
            subagent_task: row.subagent_task,
            subagent_status: row
                .subagent_status
                .map(|s| SubagentStatus::from(s.as_str())),
        }
    }
}
