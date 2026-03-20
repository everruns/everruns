// Session service for business logic (M2)
//
// Design Decision: Capability mounts are applied at session creation time.
// This ensures mounted files are available immediately when the session starts.
// The service collects mounts from the agent's capabilities and applies them
// to the session filesystem. Session capabilities are applied after agent capabilities
// (additive behavior).

use crate::api::common::Pagination;
use crate::errors::ResourceNotFoundError;
use crate::org_init::BASE_HARNESS_ID;
use crate::services::harness::resolve_effective_harness;
use crate::services::session_file::{CreateFileInput, SessionFileService};
use crate::storage::{
    StorageBackend,
    models::{CreateSessionRow, UpdateSession},
};
use anyhow::Result;
use everruns_core::{
    AgentCapabilityConfig, AgentId, Caller, CapabilityRegistry, HarnessId, InitialFile, ModelId,
    Permission, Policy, Rule, Session, SessionId, SessionStatus, SubagentStatus, TokenUsage,
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

        let harness = self
            .db
            .get_harness(org_id, harness_id)
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("Harness"))?;
        if harness.status != "active" {
            anyhow::bail!("Archived or deleted harnesses cannot be assigned");
        }
        let effective_harness = resolve_effective_harness(self.db.as_ref(), org_id, harness_id)
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("Harness"))?;
        let agent = if let Some(aid) = agent_id {
            let agent = self
                .db
                .get_agent(org_id, aid)
                .await?
                .ok_or_else(|| ResourceNotFoundError::new("Agent"))?;
            if agent.status != "active" {
                anyhow::bail!("Archived or deleted agents cannot be assigned");
            }
            Some(agent)
        } else {
            None
        };

        // Resolve model_id: session > agent > harness
        let model_id = self
            .validate_model_id(org_id, req.model_id)
            .await?
            .or_else(|| {
                // Try agent's default_model_id first, then fall back to harness's default_model_id.
                agent
                    .as_ref()
                    .and_then(|a| a.default_model_id)
                    .or(effective_harness.default_model_id)
            });

        // Serialize capabilities to JSON for storage
        let capabilities_json = serde_json::to_value(&req.capabilities)?;

        let hints_json = req
            .hints
            .as_ref()
            .map(|h| serde_json::to_value(h).unwrap_or_default());

        let input = CreateSessionRow {
            org_id,
            harness_id: Some(harness_id),
            agent_id,
            title: req.title,
            locale: req.locale.clone(),
            tags: req.tags,
            model_id,
            capabilities: capabilities_json,
            tools: serde_json::to_value(&req.tools).unwrap_or_default(),
            hints: hints_json,
        };
        let row = self.db.create_session(input).await?;
        let mut session = Self::row_to_session(row, org_public_id);

        // Populate features before overriding agent_id (needs internal UUID)
        self.populate_features(org_id, &mut session).await?;

        // Override agent_id with public_id (DB stores internal UUID as FK)
        session.agent_id = agent_public_id;

        // Apply capability mounts (harness + agent + session capabilities)
        self.apply_capability_mounts(
            org_id,
            harness_id.uuid(),
            agent_id.map(|a| a.uuid()),
            &req.capabilities,
            session.id.uuid(),
        )
        .await?;

        self.apply_initial_files(
            org_id,
            harness_id.uuid(),
            agent_id.map(|a| a.uuid()),
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
        org_id: i64,
        harness_id: Uuid,
        agent_id: Option<Uuid>,
        session_capabilities: &[AgentCapabilityConfig],
        session_id: impl Into<uuid::Uuid> + Copy,
    ) -> Result<()> {
        let session_id = session_id.into();

        let capability_ids = self
            .collect_session_capability_ids(org_id, harness_id, agent_id, session_capabilities)
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

    /// Copy harness/agent starter files into the session filesystem.
    async fn apply_initial_files(
        &self,
        org_id: i64,
        harness_id: Uuid,
        agent_id: Option<Uuid>,
        session_id: Uuid,
    ) -> Result<()> {
        for file in self
            .collect_initial_files(org_id, harness_id, agent_id)
            .await?
        {
            self.session_file_service
                .create_file(
                    session_id,
                    CreateFileInput {
                        path: normalize_initial_file_path(&file.path),
                        content: Some(file.content),
                        encoding: Some(file.encoding),
                        is_readonly: Some(file.is_readonly),
                    },
                )
                .await?;
        }
        Ok(())
    }

    async fn collect_initial_files(
        &self,
        org_id: i64,
        harness_id: Uuid,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<InitialFile>> {
        let mut files = self
            .resolve_effective_harness(org_id, HarnessId::from_uuid(harness_id))
            .await?
            .map(|harness| harness.initial_files)
            .unwrap_or_default();

        if let Some(agent_id) = agent_id
            && let Some(row) = self
                .db
                .get_agent(org_id, AgentId::from_uuid(agent_id))
                .await?
        {
            for file in
                serde_json::from_value::<Vec<InitialFile>>(row.initial_files).unwrap_or_default()
            {
                let normalized = normalize_initial_file_path(&file.path);
                if let Some(existing) = files
                    .iter_mut()
                    .find(|existing| normalize_initial_file_path(&existing.path) == normalized)
                {
                    *existing = file;
                } else {
                    files.push(file);
                }
            }
        }

        Ok(files)
    }

    async fn validate_model_id(
        &self,
        org_id: i64,
        model_id: Option<ModelId>,
    ) -> Result<Option<ModelId>> {
        let Some(model_id) = model_id else {
            return Ok(None);
        };

        self.db
            .get_llm_model(org_id, model_id.uuid())
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("Model"))?;

        Ok(Some(model_id))
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
                self.populate_features(caller.org_id, &mut session).await?;
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
            self.populate_features(org_id, session).await?;
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
            locale: req.locale,
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
        title: &str,
    ) -> Result<Session> {
        let org_id = caller.org_id;
        let org_public_id = &caller.org_public_id;
        let user_tag = format!("user:{}", user_id);
        let tags = vec!["global-chat".to_string(), user_tag.clone()];

        // Look for existing chat session
        if let Some(row) = self.db.find_session_by_tags(org_id, &tags).await? {
            let mut session = Self::row_to_session(row, org_public_id);
            self.populate_features(caller.org_id, &mut session).await?;
            self.resolve_session_agent_id(org_id, &mut session).await?;
            return Ok(session);
        }

        // Create a new chat session
        let harness_id_typed = HarnessId::from_uuid(harness_id);
        let input = CreateSessionRow {
            org_id,
            harness_id: Some(harness_id_typed),
            agent_id: None,
            title: Some(title.to_string()),
            locale: None,
            tags: vec!["global-chat".to_string(), user_tag],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            hints: None,
        };
        let row = self.db.create_session(input).await?;
        let session_id = row.id.uuid();
        let mut session = Self::row_to_session(row, org_public_id);
        self.populate_features(org_id, &mut session).await?;

        // Apply capability mounts
        self.apply_capability_mounts(org_id, harness_id, None, &[], session_id)
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
        org_id: i64,
        harness_id: Uuid,
        agent_id: Option<Uuid>,
        session_capabilities: &[AgentCapabilityConfig],
    ) -> Result<Vec<String>> {
        let mut capability_ids = Vec::new();

        if self
            .resolve_effective_harness(org_id, HarnessId::from_uuid(harness_id))
            .await?
            .is_some()
        {
            capability_ids.extend(
                self.resolve_effective_harness(org_id, HarnessId::from_uuid(harness_id))
                    .await?
                    .map(|harness| {
                        harness
                            .capabilities
                            .into_iter()
                            .map(|cap| cap.capability_id().to_string())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
            );
        }

        if let Some(agent_id) = agent_id
            && self
                .db
                .get_agent(org_id, AgentId::from_uuid(agent_id))
                .await?
                .is_some()
        {
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
    async fn populate_features(&self, org_id: i64, session: &mut Session) -> Result<()> {
        let harness_id = session.harness_id.uuid();
        let agent_internal_id = session.agent_id.map(|a| a.uuid());

        let capability_ids = self
            .collect_session_capability_ids(
                org_id,
                harness_id,
                agent_internal_id,
                &session.capabilities,
            )
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
            harness_id: row
                .harness_id
                .unwrap_or_else(|| HarnessId::from_uuid(BASE_HARNESS_ID)),
            agent_id: row.agent_id,
            title: row.title,
            locale: row.locale,
            preview: None,        // Populated separately in list()
            output_preview: None, // Populated separately in list()
            tags: row.tags,
            model_id: row.model_id,
            capabilities,
            tools: serde_json::from_value(row.tools).unwrap_or_default(),
            hints: row.hints.and_then(|v| serde_json::from_value(v).ok()),
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

    async fn resolve_effective_harness(
        &self,
        org_id: i64,
        harness_id: HarnessId,
    ) -> Result<Option<everruns_core::Harness>> {
        resolve_effective_harness(self.db.as_ref(), org_id, harness_id).await
    }
}

fn normalize_initial_file_path(path: &str) -> String {
    if path == "/workspace" {
        "/".to_string()
    } else if let Some(stripped) = path.strip_prefix("/workspace/") {
        format!("/{}", stripped.trim_start_matches('/'))
    } else if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{agents::CreateAgentRequest, harnesses::CreateHarnessRequest};
    use crate::services::{AgentService, HarnessService};
    use crate::storage::{
        CreateLlmModelRow, CreateLlmProviderRow, CreateOrganizationRow, StorageBackend,
    };
    use everruns_core::{Caller, DEFAULT_ORG_ID, InitialFile};

    fn build_create_request(
        harness_id: HarnessId,
        agent_id: Option<AgentId>,
        model_id: Option<ModelId>,
    ) -> CreateSessionRequest {
        CreateSessionRequest {
            harness_id: Some(harness_id),
            agent_id,
            title: Some("Test Session".to_string()),
            locale: None,
            tags: vec![],
            model_id,
            capabilities: vec![],
            tools: vec![],
            hints: None,
        }
    }

    async fn create_second_org(db: &StorageBackend) -> i64 {
        db.create_organization_with_id(
            2,
            CreateOrganizationRow {
                public_id: "org_2".to_string(),
                name: "Org 2".to_string(),
                created_by: None,
            },
        )
        .await
        .unwrap()
        .unwrap()
        .org_id
    }

    async fn create_model(db: &StorageBackend, org_id: i64, model_id: &str) -> ModelId {
        let provider = db
            .create_llm_provider(
                org_id,
                CreateLlmProviderRow {
                    name: format!("Provider {org_id}"),
                    provider_type: "openai".to_string(),
                    base_url: None,
                    api_key_encrypted: None,
                    settings: None,
                },
            )
            .await
            .unwrap();

        db.create_llm_model(
            org_id,
            CreateLlmModelRow {
                provider_id: provider.id,
                model_id: model_id.to_string(),
                display_name: model_id.to_string(),
                capabilities: vec![],
                installed: true,
                is_favorite: false,
                source: "manual".to_string(),
                provider_metadata: None,
            },
        )
        .await
        .unwrap()
        .id
    }

    #[tokio::test]
    async fn starter_files_are_copied_into_new_sessions() {
        let db = Arc::new(StorageBackend::in_memory());
        let harness_service = HarnessService::new(db.clone());
        let agent_service = AgentService::new(db.clone());
        let session_service = SessionService::new(db.clone());
        let caller = Caller::internal(1);

        let harness = harness_service
            .create(
                &caller,
                CreateHarnessRequest {
                    name: "Harness".to_string(),
                    description: None,
                    system_prompt: "Harness prompt".to_string(),
                    parent_harness_id: None,
                    default_model_id: None,
                    tags: vec![],
                    capabilities: vec![],
                    initial_files: vec![
                        InitialFile {
                            path: "/workspace/config.txt".to_string(),
                            content: "from harness".to_string(),
                            encoding: "text".to_string(),
                            is_readonly: false,
                        },
                        InitialFile {
                            path: "/only-harness.txt".to_string(),
                            content: "h-only".to_string(),
                            encoding: "text".to_string(),
                            is_readonly: true,
                        },
                    ],
                },
            )
            .await
            .unwrap();

        let agent = agent_service
            .create(
                &caller,
                None,
                CreateAgentRequest {
                    id: None,
                    name: "Agent".to_string(),
                    description: None,
                    system_prompt: "Agent prompt".to_string(),
                    default_model_id: None,
                    tags: vec![],
                    capabilities: vec![],
                    initial_files: vec![
                        InitialFile {
                            path: "/config.txt".to_string(),
                            content: "from agent".to_string(),
                            encoding: "text".to_string(),
                            is_readonly: false,
                        },
                        InitialFile {
                            path: "/binary.bin".to_string(),
                            content: "AAE=".to_string(),
                            encoding: "base64".to_string(),
                            is_readonly: true,
                        },
                    ],
                    tools: vec![],
                },
            )
            .await
            .unwrap();

        let session = session_service
            .create(
                &caller,
                harness.id.uuid(),
                Some(agent.internal_id),
                Some(agent.public_id),
                build_create_request(harness.id, Some(agent.public_id), None),
            )
            .await
            .unwrap();

        let file_service = SessionFileService::new(db);
        let config = file_service
            .read_file(session.id.uuid(), "/config.txt")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(config.content.as_deref(), Some("from agent"));

        let harness_only = file_service
            .read_file(session.id.uuid(), "/only-harness.txt")
            .await
            .unwrap()
            .unwrap();
        assert!(harness_only.is_readonly);
        assert_eq!(harness_only.content.as_deref(), Some("h-only"));

        let binary = file_service
            .read_file(session.id.uuid(), "/binary.bin")
            .await
            .unwrap()
            .unwrap();
        assert!(binary.is_readonly);
        assert_eq!(binary.encoding, "base64");
        assert_eq!(binary.content.as_deref(), Some("AAE="));
    }

    #[tokio::test]
    async fn inherited_harness_starter_files_are_copied_into_new_sessions() {
        let db = Arc::new(StorageBackend::in_memory());
        let harness_service = HarnessService::new(db.clone());
        let session_service = SessionService::new(db.clone());
        let caller = Caller::internal(1);

        let parent = harness_service
            .create(
                &caller,
                CreateHarnessRequest {
                    name: "Parent".to_string(),
                    description: None,
                    system_prompt: "Parent prompt".to_string(),
                    parent_harness_id: None,
                    default_model_id: None,
                    tags: vec![],
                    capabilities: vec![],
                    initial_files: vec![
                        InitialFile {
                            path: "/workspace/config.txt".to_string(),
                            content: "from parent".to_string(),
                            encoding: "text".to_string(),
                            is_readonly: false,
                        },
                        InitialFile {
                            path: "/parent-only.txt".to_string(),
                            content: "only parent".to_string(),
                            encoding: "text".to_string(),
                            is_readonly: true,
                        },
                    ],
                },
            )
            .await
            .unwrap();

        let child = harness_service
            .create(
                &caller,
                CreateHarnessRequest {
                    name: "Child".to_string(),
                    description: None,
                    system_prompt: "Child prompt".to_string(),
                    parent_harness_id: Some(parent.id),
                    default_model_id: None,
                    tags: vec![],
                    capabilities: vec![],
                    initial_files: vec![InitialFile {
                        path: "/config.txt".to_string(),
                        content: "from child".to_string(),
                        encoding: "text".to_string(),
                        is_readonly: true,
                    }],
                },
            )
            .await
            .unwrap();

        let session = session_service
            .create(
                &caller,
                child.id.uuid(),
                None,
                None,
                build_create_request(child.id, None, None),
            )
            .await
            .unwrap();

        let file_service = SessionFileService::new(db);
        let config = file_service
            .read_file(session.id.uuid(), "/config.txt")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(config.content.as_deref(), Some("from child"));

        let parent_only = file_service
            .read_file(session.id.uuid(), "/parent-only.txt")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(parent_only.content.as_deref(), Some("only parent"));
    }

    #[tokio::test]
    async fn archived_dependencies_cannot_be_assigned_in_dev_mode() {
        let db = Arc::new(StorageBackend::in_memory());
        let harness_service = HarnessService::new(db.clone());
        let agent_service = AgentService::new(db.clone());
        let session_service = SessionService::new(db);
        let caller = Caller::internal(1);

        let harness = harness_service
            .create(
                &caller,
                CreateHarnessRequest {
                    name: "Harness".to_string(),
                    description: None,
                    system_prompt: "Harness prompt".to_string(),
                    parent_harness_id: None,
                    default_model_id: None,
                    tags: vec![],
                    capabilities: vec![],
                    initial_files: vec![],
                },
            )
            .await
            .unwrap();

        let agent = agent_service
            .create(
                &caller,
                None,
                CreateAgentRequest {
                    id: None,
                    name: "Agent".to_string(),
                    description: None,
                    system_prompt: "Agent prompt".to_string(),
                    default_model_id: None,
                    tags: vec![],
                    capabilities: vec![],
                    initial_files: vec![],
                    tools: vec![],
                },
            )
            .await
            .unwrap();

        harness_service
            .delete(&caller, harness.id.uuid())
            .await
            .unwrap();
        let error = session_service
            .create(
                &caller,
                harness.id.uuid(),
                Some(agent.internal_id),
                Some(agent.public_id),
                build_create_request(harness.id, Some(agent.public_id), None),
            )
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Archived or deleted harnesses cannot be assigned")
        );

        let harness = harness_service
            .create(
                &caller,
                CreateHarnessRequest {
                    name: "Harness 2".to_string(),
                    description: None,
                    system_prompt: "Harness prompt".to_string(),
                    parent_harness_id: None,
                    default_model_id: None,
                    tags: vec![],
                    capabilities: vec![],
                    initial_files: vec![],
                },
            )
            .await
            .unwrap();
        agent_service
            .delete(&caller, &agent.public_id.to_string())
            .await
            .unwrap();

        let error = session_service
            .create(
                &caller,
                harness.id.uuid(),
                Some(agent.internal_id),
                Some(agent.public_id),
                build_create_request(harness.id, Some(agent.public_id), None),
            )
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Archived or deleted agents cannot be assigned")
        );
    }

    #[tokio::test]
    async fn create_rejects_harness_from_another_org() {
        let db = Arc::new(StorageBackend::in_memory());
        let harness_service = HarnessService::new(db.clone());
        let session_service = SessionService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let other_org_id = create_second_org(&db).await;

        let other_harness = harness_service
            .create(
                &Caller::internal(other_org_id),
                CreateHarnessRequest {
                    name: "Other Harness".to_string(),
                    description: None,
                    system_prompt: "Other".to_string(),
                    parent_harness_id: None,
                    default_model_id: None,
                    tags: vec![],
                    capabilities: vec![],
                    initial_files: vec![],
                },
            )
            .await
            .unwrap();

        let err = session_service
            .create(
                &caller,
                other_harness.id.uuid(),
                None,
                None,
                build_create_request(other_harness.id, None, None),
            )
            .await
            .unwrap_err();

        let not_found = err.downcast_ref::<ResourceNotFoundError>().unwrap();
        assert_eq!(not_found.resource(), "Harness");
    }

    #[tokio::test]
    async fn create_rejects_model_from_another_org() {
        let db = Arc::new(StorageBackend::in_memory());
        let harness_service = HarnessService::new(db.clone());
        let session_service = SessionService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let other_org_id = create_second_org(&db).await;
        let other_model_id = create_model(&db, other_org_id, "cross-org-model").await;

        let harness = harness_service
            .create(
                &caller,
                CreateHarnessRequest {
                    name: "Harness".to_string(),
                    description: None,
                    system_prompt: "Harness".to_string(),
                    parent_harness_id: None,
                    default_model_id: None,
                    tags: vec![],
                    capabilities: vec![],
                    initial_files: vec![],
                },
            )
            .await
            .unwrap();

        let err = session_service
            .create(
                &caller,
                harness.id.uuid(),
                None,
                None,
                build_create_request(harness.id, None, Some(other_model_id)),
            )
            .await
            .unwrap_err();

        let not_found = err.downcast_ref::<ResourceNotFoundError>().unwrap();
        assert_eq!(not_found.resource(), "Model");
    }

    #[tokio::test]
    async fn get_skips_foreign_harness_and_agent_capability_features() {
        let db = Arc::new(StorageBackend::in_memory());
        let harness_service = HarnessService::new(db.clone());
        let agent_service = AgentService::new(db.clone());
        let session_service = SessionService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let other_org_id = create_second_org(&db).await;

        let other_harness = harness_service
            .create(
                &Caller::internal(other_org_id),
                CreateHarnessRequest {
                    name: "Other Harness".to_string(),
                    description: None,
                    system_prompt: "Other".to_string(),
                    parent_harness_id: None,
                    default_model_id: None,
                    tags: vec![],
                    capabilities: vec![AgentCapabilityConfig::new("sample_data")],
                    initial_files: vec![],
                },
            )
            .await
            .unwrap();

        let other_agent = agent_service
            .create(
                &Caller::internal(other_org_id),
                None,
                CreateAgentRequest {
                    id: None,
                    name: "Other Agent".to_string(),
                    description: None,
                    system_prompt: "Other".to_string(),
                    default_model_id: None,
                    tags: vec![],
                    capabilities: vec![AgentCapabilityConfig::new("session_schedule")],
                    initial_files: vec![],
                    tools: vec![],
                },
            )
            .await
            .unwrap();

        let session_row = db
            .create_session(CreateSessionRow {
                org_id: caller.org_id,
                harness_id: Some(other_harness.id),
                agent_id: Some(AgentId::from_uuid(other_agent.internal_id)),
                title: Some("Corrupt Session".to_string()),
                locale: None,
                tags: vec![],
                model_id: None,
                capabilities: serde_json::to_value(vec![AgentCapabilityConfig::new(
                    "session_sql_database",
                )])
                .unwrap(),
                tools: serde_json::json!([]),
                hints: None,
            })
            .await
            .unwrap();

        let session = session_service
            .get(&caller, session_row.id.uuid(), None)
            .await
            .unwrap()
            .unwrap();

        assert!(
            session.features.contains(&"sql_database".to_string()),
            "session capability should still apply: {:?}",
            session.features
        );
        assert!(
            !session.features.contains(&"file_system".to_string()),
            "foreign harness capability should not contribute features: {:?}",
            session.features
        );
        assert!(
            !session.features.contains(&"schedules".to_string()),
            "foreign agent capability should not contribute features: {:?}",
            session.features
        );
    }

    #[tokio::test]
    async fn apply_capability_mounts_skips_foreign_harness_and_agent_capabilities() {
        let db = Arc::new(StorageBackend::in_memory());
        let harness_service = HarnessService::new(db.clone());
        let agent_service = AgentService::new(db.clone());
        let session_service = SessionService::new(db.clone());
        let file_service = SessionFileService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let other_org_id = create_second_org(&db).await;

        let other_harness = harness_service
            .create(
                &Caller::internal(other_org_id),
                CreateHarnessRequest {
                    name: "Other Harness".to_string(),
                    description: None,
                    system_prompt: "Other".to_string(),
                    parent_harness_id: None,
                    default_model_id: None,
                    tags: vec![],
                    capabilities: vec![AgentCapabilityConfig::new("sample_data")],
                    initial_files: vec![],
                },
            )
            .await
            .unwrap();

        let other_agent = agent_service
            .create(
                &Caller::internal(other_org_id),
                None,
                CreateAgentRequest {
                    id: None,
                    name: "Other Agent".to_string(),
                    description: None,
                    system_prompt: "Other".to_string(),
                    default_model_id: None,
                    tags: vec![],
                    capabilities: vec![AgentCapabilityConfig::new("sample_data")],
                    initial_files: vec![],
                    tools: vec![],
                },
            )
            .await
            .unwrap();

        let session_row = db
            .create_session(CreateSessionRow {
                org_id: caller.org_id,
                harness_id: Some(HarnessId::from_uuid(BASE_HARNESS_ID)),
                agent_id: None,
                title: Some("Mount Test".to_string()),
                locale: None,
                tags: vec![],
                model_id: None,
                capabilities: serde_json::json!([]),
                tools: serde_json::json!([]),
                hints: None,
            })
            .await
            .unwrap();

        session_service
            .apply_capability_mounts(
                caller.org_id,
                other_harness.id.uuid(),
                Some(other_agent.internal_id),
                &[],
                session_row.id.uuid(),
            )
            .await
            .unwrap();

        assert!(
            file_service
                .read_file(session_row.id.uuid(), "/samples/users.json")
                .await
                .unwrap()
                .is_none(),
            "foreign harness/agent capabilities should not mount files"
        );
    }
}
