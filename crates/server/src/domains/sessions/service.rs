// Session service for business logic (M2)
//
// Design Decision: Capability mounts are applied at session creation time.
// This ensures mounted files are available immediately when the session starts.
// The service collects mounts from the agent's capabilities and applies them
// to the session filesystem. Session capabilities are applied after agent capabilities
// (additive behavior).

use crate::api::common::Pagination;
use crate::domains::harnesses::queries::resolve_effective as resolve_effective_harness;
use crate::domains::session_files::{CreateFileInput, SessionFileService};
use crate::domains::session_sandbox::SessionSandboxService;
use crate::domains::sessions::limits::OrgCaps;
use crate::errors::{BadRequestError, ResourceNotFoundError};
use crate::max_iterations;
use crate::org_init;
use crate::services::PrincipalService;
use crate::storage::{
    StorageBackend,
    models::{CreateSessionRow, MemoryFileRow, UpdateSession},
};
use anyhow::Result;
use everruns_core::session_sandbox::SESSION_SANDBOX_CAPABILITY_ID;
use everruns_core::{
    AgentCapabilityConfig, AgentId, AgentVersionPolicy, Caller, CapabilityRegistry,
    DeclarativeCapabilityDefinition, FeatureFlags, HarnessId, InitialFile, ModelId, MountAccess,
    MountEntry, MountPoint, MountSource, OrgRole, Permission, Policy, PrincipalId, Rule, Session,
    SessionFile, SessionId, SessionStatus, SubagentStatus, TokenUsage, WorkspaceId,
    capabilities::{
        MEMORY_CAPABILITY_ID, RiskLevel, SystemPromptContext, collect_capabilities_with_configs,
        compute_features, resolve_capability_configs,
    },
    is_declarative_capability,
    memory::{MemoryConfig, MemoryMountAccess},
    merge_capabilities, merge_initial_files, normalize_initial_file_path,
    parse_declarative_capability_id,
    typed_id::MemoryId,
};
use everruns_durable::UpdateField;
use std::collections::{HashMap, HashSet};
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
    principal_service: PrincipalService,
    capability_registry: CapabilityRegistry,
    session_file_service: SessionFileService,
    session_sandbox_service: Option<Arc<SessionSandboxService>>,
    caps: OrgCaps,
}

impl SessionService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self {
            principal_service: PrincipalService::new(db.clone()),
            capability_registry: CapabilityRegistry::with_builtins(),
            session_file_service: SessionFileService::new(db.clone()),
            db,
            session_sandbox_service: None,
            caps: OrgCaps::from_env(),
        }
    }

    /// Create a new SessionService with a custom capability registry.
    pub fn with_registry(db: Arc<StorageBackend>, registry: CapabilityRegistry) -> Self {
        Self {
            principal_service: PrincipalService::new(db.clone()),
            capability_registry: registry,
            session_file_service: SessionFileService::new(db.clone()),
            db,
            session_sandbox_service: None,
            caps: OrgCaps::from_env(),
        }
    }

    pub fn with_caps(mut self, caps: OrgCaps) -> Self {
        self.caps = caps;
        self
    }

    /// Attach a virtual mount registry to the internal session file service.
    pub fn with_virtual_registry(
        mut self,
        registry: Arc<crate::domains::session_files::virtual_mount_registry::VirtualMountRegistry>,
    ) -> Self {
        self.session_file_service =
            SessionFileService::new(self.db.clone()).with_virtual_registry(registry);
        self
    }

    pub fn with_session_sandbox_service(mut self, service: Arc<SessionSandboxService>) -> Self {
        self.session_sandbox_service = Some(service);
        self
    }

    pub async fn create(
        &self,
        caller: &Caller,
        harness_id: Uuid,
        agent_internal_id: Option<Uuid>,
        agent_public_id: Option<AgentId>,
        req: CreateSessionRequest,
    ) -> Result<Session> {
        self.create_inner(
            caller,
            harness_id,
            agent_internal_id,
            agent_public_id,
            None,
            None,
            req,
        )
        .await
    }

    /// Create a session from an App channel. The app backreference is server-owned
    /// and intentionally absent from public session create/update request types.
    ///
    /// `owner_principal_id` and `resolved_owner_user_id` come from the App row
    /// itself, not the caller. This is intentional: app-channel ingress
    /// (webhook, schedule, A2A, AG-UI, Slack) typically runs as
    /// `Caller::internal(org)` whose default principal is the system principal,
    /// while the App was created by a real user. Without this override, the
    /// session would be owned by `system-owner` and shared-session reuse via
    /// `find_app_session_by_tags_and_owner(.. app.owner_principal_id ..)` would
    /// fail to match it. See `specs/app-invocation-channels.md` and EVE-A2A
    /// follow-up.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_from_app(
        &self,
        caller: &Caller,
        harness_id: Uuid,
        agent_internal_id: Option<Uuid>,
        agent_public_id: Option<AgentId>,
        app_internal_id: Uuid,
        owner_principal_id: PrincipalId,
        resolved_owner_user_id: Option<Uuid>,
        req: CreateSessionRequest,
    ) -> Result<Session> {
        self.create_inner(
            caller,
            harness_id,
            agent_internal_id,
            agent_public_id,
            Some(app_internal_id),
            Some((owner_principal_id, resolved_owner_user_id)),
            req,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_inner(
        &self,
        caller: &Caller,
        harness_id: Uuid,
        agent_internal_id: Option<Uuid>,
        agent_public_id: Option<AgentId>,
        app_id: Option<Uuid>,
        // (principal, resolved_user) override; used by app-channel ingress so
        // the session owner matches the App row (not the internal caller).
        owner_override: Option<(PrincipalId, Option<Uuid>)>,
        req: CreateSessionRequest,
    ) -> Result<Session> {
        let org_id = caller.org_id;
        let org_public_id = &caller.org_public_id;
        let harness_id = HarnessId::from_uuid(harness_id);
        let agent_id = agent_internal_id.map(AgentId::from_uuid);

        // EVE-508: check per-org concurrent session cap before creating.
        let active_sessions = self.db.count_active_sessions_for_org(org_id).await?;
        if active_sessions >= self.caps.max_concurrent_sessions as i64 {
            return Err(BadRequestError::new(format!(
                "Too many concurrent sessions: org has {} active sessions (limit {}); retry later",
                active_sessions, self.caps.max_concurrent_sessions
            ))
            .into());
        }

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
        let agent_mcp_servers = agent
            .as_ref()
            .map(|agent| serde_json::from_value(agent.mcp_servers.clone()).unwrap_or_default());

        let resolved_agent_version = if FeatureFlags::current().agent_versions {
            if let Some(agent_id) = agent_id {
                let app_row = match app_id {
                    Some(app_internal_id) => self.db.get_app_by_id(org_id, app_internal_id).await?,
                    None => None,
                };
                match app_row
                    .as_ref()
                    .map(|app| AgentVersionPolicy::from(app.agent_version_policy.as_str()))
                    .unwrap_or_default()
                {
                    AgentVersionPolicy::Pinned => {
                        if let Some(version_id) = app_row.and_then(|app| app.agent_version_id) {
                            self.db
                                .get_agent_version(
                                    org_id,
                                    everruns_core::AgentVersionId::from_uuid(version_id),
                                )
                                .await?
                        } else {
                            None
                        }
                    }
                    AgentVersionPolicy::Latest => {
                        self.db.get_latest_agent_version(org_id, agent_id).await?
                    }
                    AgentVersionPolicy::Default => {
                        if let Some(version_id) = agent.as_ref().and_then(|a| a.default_version_id)
                        {
                            self.db.get_agent_version(org_id, version_id).await?
                        } else {
                            None
                        }
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        let agent_identity_id = if let Some(identity_id) = req.agent_identity_id {
            let identity = self
                .db
                .get_agent_identity(org_id, identity_id)
                .await?
                .ok_or_else(|| ResourceNotFoundError::new("Agent identity"))?;
            if identity.status != "active" {
                anyhow::bail!("Archived or deleted agent identities cannot be assigned");
            }
            Some(identity.id)
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

        let session_capabilities = sanitize_session_capabilities(req.capabilities);

        // Validate session-level capability refs before persisting
        crate::domains::capabilities::validation::validate_capability_refs(
            &self.db,
            org_id,
            &session_capabilities,
        )
        .await?;
        self.require_admin_for_high_risk_session_capabilities(
            caller,
            org_id,
            harness_id.uuid(),
            agent_id.map(|id| id.uuid()),
            &session_capabilities,
        )
        .await?;
        let mut scoped_mcp_layers = vec![&effective_harness.mcp_servers];
        if let Some(ref agent_mcp_servers) = agent_mcp_servers {
            scoped_mcp_layers.push(agent_mcp_servers);
        }
        scoped_mcp_layers.push(&req.mcp_servers);
        crate::domains::mcp_servers::scoped_mcp::validate_merged_scoped_mcp_servers(
            scoped_mcp_layers,
        )?;

        // Serialize capabilities to JSON for storage
        let capabilities_json = serde_json::to_value(&session_capabilities)?;

        let hints_json = req
            .hints
            .as_ref()
            .map(|h| serde_json::to_value(h).unwrap_or_default());

        if !caller.is_internal && req.tags.iter().any(|tag| tag.starts_with("__internal:")) {
            return Err(BadRequestError::new("Tags with '__internal:' prefix are reserved").into());
        }
        // THREAT[TM-AUTHZ-009]: `app:<id>`, `app_channel:<id>` and the legacy
        // `slack:app:<id>` tags all drive budget hierarchy attribution (see
        // specs/budgeting.md and `extract_app_subjects` in
        // `crates/server/src/domains/budgets/service.rs`). Allowing external
        // callers to forge any of them would let an org member opt their
        // session into another app's budget — corrupting spend attribution and
        // potentially exhausting that app's cap. Only the apps and Slack
        // domains (both using `Caller::internal`) are permitted to stamp them.
        if !caller.is_internal
            && req.tags.iter().any(|tag| {
                tag.starts_with("app:")
                    || tag.starts_with("app_channel:")
                    || tag.starts_with("slack:app:")
                    || tag.starts_with("ag_ui:app:")
            })
        {
            return Err(BadRequestError::new(
                "Tags with 'app:', 'app_channel:', 'slack:app:', or 'ag_ui:app:' prefix are reserved for internal subsystems",
            )
            .into());
        }

        let (owner_principal_id, resolved_owner_user_id) = match owner_override {
            // App-channel ingress: trust the App row's owner so shared-session
            // lookups (which key on `app.owner_principal_id`) actually match
            // sessions created here.
            Some((principal_id, resolved_user_id)) => (principal_id, resolved_user_id),
            None => {
                let owner_principal = self
                    .principal_service
                    .default_owner_principal(caller, agent_identity_id)
                    .await?;
                (owner_principal.id, owner_principal.resolved_user_id)
            }
        };

        let input = CreateSessionRow {
            org_id,
            app_id,
            harness_id: Some(harness_id),
            agent_id,
            agent_identity_id,
            owner_principal_id,
            resolved_owner_user_id,
            title: req.title,
            locale: req.locale.clone(),
            tags: req.tags,
            model_id,
            capabilities: capabilities_json,
            tools: serde_json::to_value(&req.tools).unwrap_or_default(),
            mcp_servers: serde_json::to_value(&req.mcp_servers).unwrap_or_default(),
            system_prompt: req.system_prompt.clone(),
            initial_files: serde_json::to_value(&req.initial_files).unwrap_or_default(),
            hints: hints_json,
            network_access: req
                .network_access
                .as_ref()
                .map(|na| serde_json::to_value(na).unwrap()),
            max_iterations: max_iterations::to_db(req.max_iterations)?,
            blueprint_id: None,
            blueprint_config: None,
        };
        let row = self.db.create_session(input).await?;
        let row = if let Some(version) = resolved_agent_version.as_ref() {
            self.db
                .update_session(
                    org_id,
                    row.id,
                    UpdateSession {
                        agent_version_id: Some(version.id),
                        agent_config_hash: Some(version.config_hash.clone()),
                        ..Default::default()
                    },
                )
                .await?
                .unwrap_or(row)
        } else {
            row
        };
        let mut session = Self::row_to_session(row, org_public_id, Some(harness_id));
        self.hydrate_ownership(org_id, &mut session).await?;

        // Populate features before overriding agent_id (needs internal UUID)
        self.populate_features(org_id, &mut session).await?;

        // Override agent_id with public_id (DB stores internal UUID as FK)
        session.agent_id = agent_public_id;

        // Apply capability mounts (harness + agent + session capabilities)
        self.apply_capability_mounts(
            org_id,
            harness_id.uuid(),
            agent_id.map(|a| a.uuid()),
            &session_capabilities,
            session.id.uuid(),
        )
        .await?;

        self.apply_initial_files(
            org_id,
            harness_id.uuid(),
            agent_id.map(|a| a.uuid()),
            &req.initial_files,
            session.id.uuid(),
        )
        .await?;

        // Effective capability list (harness + agent + session), resolved once
        // and shared by sandbox auto-start and the session_start hook so the
        // merge rule lives in exactly one place per call. Agent-cap lookup
        // failures propagate here (sandbox auto-start treats them as fatal),
        // unlike the best-effort `resolve_session_capability_configs` used by
        // the advisory delete path.
        let agent_capabilities = if let Some(agent_id) = agent_id {
            self.db
                .get_agent_capabilities(agent_id.uuid())
                .await?
                .into_iter()
                .map(|row| AgentCapabilityConfig::with_config(row.capability_id, row.config))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let effective_capabilities = merge_capabilities(
            &merge_capabilities(&effective_harness.capabilities, &agent_capabilities),
            &session_capabilities,
        );

        if let Some(service) = &self.session_sandbox_service {
            service
                .auto_start_for_capabilities(session.id, &effective_capabilities)
                .await;
        }

        // session_start lifecycle hooks (advisory). Fire after the session row,
        // mounts, and initial files are in place so a hook can observe/seed the
        // session VFS.
        self.fire_session_lifecycle_hooks(
            org_id,
            session.id,
            agent_id.map(|a| a.to_string()),
            &effective_capabilities,
            everruns_core::user_hook_types::HookEvent::SessionStart,
            serde_json::json!({ "agent_id": agent_id.map(|a| a.to_string()) }),
        )
        .await;

        Ok(session)
    }

    /// Create a blueprint-backed session (used by gRPC platform create).
    /// Skips agent validation since blueprint sessions don't inherit agent config.
    pub async fn create_blueprint_session(
        &self,
        caller: &Caller,
        harness_id: Uuid,
        blueprint_id: String,
        blueprint_config: Option<serde_json::Value>,
        req: CreateSessionRequest,
    ) -> Result<Session> {
        let org_id = caller.org_id;
        let org_public_id = &caller.org_public_id;
        let harness_id = HarnessId::from_uuid(harness_id);
        let effective_harness = crate::domains::harnesses::queries::resolve_effective(
            self.db.as_ref(),
            org_id,
            harness_id,
        )
        .await?
        .ok_or_else(|| ResourceNotFoundError::new("Harness"))?;
        crate::domains::mcp_servers::scoped_mcp::validate_merged_scoped_mcp_servers([
            &effective_harness.mcp_servers,
            &req.mcp_servers,
        ])?;
        let owner_principal = self
            .principal_service
            .default_owner_principal(caller, None)
            .await?;

        let input = CreateSessionRow {
            org_id,
            app_id: None,
            harness_id: Some(harness_id),
            agent_id: None,
            agent_identity_id: None,
            owner_principal_id: owner_principal.id,
            resolved_owner_user_id: owner_principal.resolved_user_id,
            title: req.title,
            locale: req.locale,
            tags: req.tags,
            model_id: None,
            capabilities: serde_json::Value::Array(vec![]),
            tools: serde_json::Value::Array(vec![]),
            mcp_servers: serde_json::to_value(&req.mcp_servers).unwrap_or_default(),
            system_prompt: req.system_prompt.clone(),
            initial_files: serde_json::to_value(&req.initial_files).unwrap_or_default(),
            hints: None,
            network_access: req
                .network_access
                .as_ref()
                .map(|na| serde_json::to_value(na).unwrap()),
            max_iterations: max_iterations::to_db(req.max_iterations)?,
            blueprint_id: Some(blueprint_id),
            blueprint_config,
        };
        let row = self.db.create_session(input).await?;
        let mut session = Self::row_to_session(row, org_public_id, Some(harness_id));
        self.populate_features(org_id, &mut session).await?;

        // Apply session-level initial files to the session filesystem
        if !req.initial_files.is_empty() {
            self.apply_initial_files(
                org_id,
                harness_id.uuid(),
                None, // Blueprint sessions have no agent
                &req.initial_files,
                session.id.uuid(),
            )
            .await?;
        }

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

        let mounts = self
            .collect_capability_mounts(
                org_id,
                harness_id,
                agent_id,
                session_capabilities,
                session_id,
            )
            .await?;
        if mounts.is_empty() {
            return Ok(()); // No mounts to apply
        }

        // Apply mounts to session filesystem
        let result = self
            .session_file_service
            .apply_capability_mounts(session_id, &mounts)
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

    async fn collect_capability_mounts(
        &self,
        org_id: i64,
        harness_id: Uuid,
        agent_id: Option<Uuid>,
        session_capabilities: &[AgentCapabilityConfig],
        session_id: Uuid,
    ) -> Result<Vec<MountPoint>> {
        let capability_configs = self
            .collect_session_capability_configs(org_id, harness_id, agent_id, session_capabilities)
            .await?;
        if capability_configs.is_empty() {
            return Ok(vec![]);
        }

        let ctx = SystemPromptContext::without_file_store(SessionId::from_uuid(session_id));
        let resolved_configs =
            resolve_capability_configs(&capability_configs, &self.capability_registry)?;
        let mut mounts =
            collect_capabilities_with_configs(&resolved_configs, &self.capability_registry, &ctx)
                .await
                .mounts;
        mounts.extend(
            self.collect_workspace_memory_mounts(org_id, &resolved_configs)
                .await?,
        );
        Ok(mounts)
    }

    async fn reconcile_capability_mounts(
        &self,
        org_id: i64,
        harness_id: Uuid,
        agent_id: Option<Uuid>,
        session_capabilities: &[AgentCapabilityConfig],
        session_id: Uuid,
    ) -> Result<()> {
        let mounts = self
            .collect_capability_mounts(
                org_id,
                harness_id,
                agent_id,
                session_capabilities,
                session_id,
            )
            .await?;

        self.session_file_service.evict_virtual_mounts(session_id);

        let mut seen_paths = HashSet::new();
        let mut mount_paths: Vec<String> = mounts
            .iter()
            .map(|mount| mount.path.clone())
            .filter(|path| seen_paths.insert(path.clone()))
            .collect();
        mount_paths.sort_by_key(|path| path.len());
        for path in mount_paths {
            let _ = self
                .db
                .delete_session_file_recursive(session_id, &path)
                .await?;
        }

        if mounts.is_empty() {
            return Ok(());
        }

        let result = self
            .session_file_service
            .apply_capability_mounts(session_id, &mounts)
            .await?;

        if !result.is_success() {
            tracing::warn!(
                session_id = %session_id,
                agent_id = ?agent_id,
                errors = ?result.errors,
                "Some capability mounts failed to apply after session repair"
            );
        }

        Ok(())
    }

    /// Copy harness/agent/session starter files into the session filesystem.
    async fn apply_initial_files(
        &self,
        org_id: i64,
        harness_id: Uuid,
        agent_id: Option<Uuid>,
        session_initial_files: &[InitialFile],
        session_id: Uuid,
    ) -> Result<()> {
        for file in self
            .collect_initial_files(org_id, harness_id, agent_id, session_initial_files)
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
        session_initial_files: &[InitialFile],
    ) -> Result<Vec<InitialFile>> {
        let harness_files = self
            .resolve_effective_harness(org_id, HarnessId::from_uuid(harness_id))
            .await?
            .map(|harness| harness.initial_files)
            .unwrap_or_default();

        let agent_files = if let Some(agent_id) = agent_id
            && let Some(row) = self
                .db
                .get_agent(org_id, AgentId::from_uuid(agent_id))
                .await?
        {
            serde_json::from_value::<Vec<InitialFile>>(row.initial_files).unwrap_or_default()
        } else {
            vec![]
        };

        // Fold: harness → agent → session (same merge semantics as AgentConfigOverlay)
        let merged = merge_initial_files(&harness_files, &agent_files);
        let merged = merge_initial_files(&merged, session_initial_files);

        Ok(merged)
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
                let fallback = if r.harness_id.is_none() {
                    Some(org_init::base_harness_id(&self.db, caller.org_id).await?)
                } else {
                    None
                };
                let mut session = Self::row_to_session(r, &caller.org_public_id, fallback);
                self.hydrate_ownership(caller.org_id, &mut session).await?;
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
        let fallback = if rows.iter().any(|r| r.harness_id.is_none()) {
            Some(org_init::base_harness_id(&self.db, org_id).await?)
        } else {
            None
        };
        let mut sessions: Vec<Session> = rows
            .into_iter()
            .map(|r| Self::row_to_session(r, org_public_id, fallback))
            .collect();

        for session in &mut sessions {
            self.hydrate_ownership(org_id, session).await?;
        }

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

    pub async fn update(
        &self,
        caller: &Caller,
        id: Uuid,
        req: UpdateSessionRequest,
    ) -> Result<Option<Session>> {
        if !caller.is_internal
            && req
                .tags
                .as_ref()
                .is_some_and(|tags| tags.iter().any(|tag| tag.starts_with("__internal:")))
        {
            return Err(BadRequestError::new("Tags with '__internal:' prefix are reserved").into());
        }
        // THREAT[TM-AUTHZ-009]: same reservation enforced on update — see
        // create() for the rationale. Includes the legacy `slack:app:` tag.
        if !caller.is_internal
            && req.tags.as_ref().is_some_and(|tags| {
                tags.iter().any(|tag| {
                    tag.starts_with("app:")
                        || tag.starts_with("app_channel:")
                        || tag.starts_with("slack:app:")
                        || tag.starts_with("ag_ui:app:")
                })
            })
        {
            return Err(BadRequestError::new(
                "Tags with 'app:', 'app_channel:', 'slack:app:', or 'ag_ui:app:' prefix are reserved for internal subsystems",
            )
            .into());
        }

        let agent_identity_id = match req.agent_identity_id {
            UpdateField::Set(identity_id) => {
                let identity = self
                    .db
                    .get_agent_identity(caller.org_id, identity_id)
                    .await?
                    .ok_or_else(|| ResourceNotFoundError::new("Agent identity"))?;
                if identity.status != "active" {
                    anyhow::bail!("Archived or deleted agent identities cannot be assigned");
                }
                UpdateField::Set(identity.id)
            }
            UpdateField::Clear => UpdateField::Clear,
            UpdateField::Unchanged => UpdateField::Unchanged,
        };
        let existing = if !matches!(agent_identity_id, UpdateField::Unchanged) {
            Some(
                self.db
                    .get_session(caller.org_id, SessionId::from_uuid(id))
                    .await?
                    .ok_or_else(|| ResourceNotFoundError::new("Session"))?,
            )
        } else {
            None
        };
        let (owner_principal_id, resolved_owner_user_id) = match agent_identity_id {
            UpdateField::Set(identity_id) => {
                let owner = self
                    .principal_service
                    .owner_for_entity(
                        caller.org_id,
                        existing
                            .as_ref()
                            .expect("existing session loaded for ownership update")
                            .owner_principal_id,
                        existing
                            .as_ref()
                            .expect("existing session loaded for ownership update")
                            .resolved_owner_user_id,
                        Some(identity_id),
                    )
                    .await?;
                (
                    Some(owner.id),
                    UpdateField::from_option(owner.resolved_user_id),
                )
            }
            UpdateField::Clear => {
                let owner = self
                    .principal_service
                    .owner_for_entity(
                        caller.org_id,
                        existing
                            .as_ref()
                            .expect("existing session loaded for ownership update")
                            .owner_principal_id,
                        existing
                            .as_ref()
                            .expect("existing session loaded for ownership update")
                            .resolved_owner_user_id,
                        None,
                    )
                    .await?;
                (
                    Some(owner.id),
                    UpdateField::from_option(owner.resolved_user_id),
                )
            }
            UpdateField::Unchanged => (None, UpdateField::Unchanged),
        };
        let input = UpdateSession {
            title: req.title,
            agent_identity_id,
            owner_principal_id,
            resolved_owner_user_id,
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
                let fallback = if r.harness_id.is_none() {
                    Some(org_init::base_harness_id(&self.db, caller.org_id).await?)
                } else {
                    None
                };
                let mut session = Self::row_to_session(r, &caller.org_public_id, fallback);
                self.hydrate_ownership(caller.org_id, &mut session).await?;
                self.resolve_session_agent_id(caller.org_id, &mut session)
                    .await?;
                Ok(Some(session))
            }
            None => Ok(None),
        }
    }

    /// Update session status (used by worker via gRPC)
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
                let fallback = if r.harness_id.is_none() {
                    Some(org_init::base_harness_id(&self.db, caller.org_id).await?)
                } else {
                    None
                };
                let mut session = Self::row_to_session(r, &caller.org_public_id, fallback);
                self.hydrate_ownership(caller.org_id, &mut session).await?;
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
    pub async fn get_or_create_chat_session(
        &self,
        caller: &Caller,
        user_id: Uuid,
        harness_id: Uuid,
        title: &str,
        locale: Option<String>,
    ) -> Result<Session> {
        let org_id = caller.org_id;
        let org_public_id = &caller.org_public_id;
        let user_tag = format!("user:{}", user_id);
        let tags = vec!["global-chat".to_string(), user_tag.clone()];
        let desired_harness_id = HarnessId::from_uuid(harness_id);

        let owner_principal = self
            .principal_service
            .ensure_user_principal(org_id, user_id)
            .await?;

        // Look for existing chat session owned by the user.
        if let Some(mut row) = self
            .db
            .find_session_by_tags_and_owner(org_id, owner_principal.id, &tags)
            .await?
        {
            if row.harness_id != Some(desired_harness_id) {
                tracing::info!(
                    session_id = %row.id,
                    previous_harness_id = ?row.harness_id,
                    desired_harness_id = %desired_harness_id,
                    "Repairing global chat session harness binding"
                );

                row = self
                    .db
                    .update_session(
                        org_id,
                        row.id,
                        UpdateSession {
                            harness_id: Some(desired_harness_id),
                            ..Default::default()
                        },
                    )
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("chat session disappeared during repair"))?;

                let session_capabilities: Vec<AgentCapabilityConfig> = match serde_json::from_value(
                    row.capabilities.clone(),
                ) {
                    Ok(capabilities) => capabilities,
                    Err(error) => {
                        tracing::error!(
                            session_id = %row.id,
                            error = %error,
                            "Failed to deserialize session capabilities during global chat repair; continuing with empty capabilities"
                        );
                        Vec::new()
                    }
                };
                self.reconcile_capability_mounts(
                    org_id,
                    desired_harness_id.uuid(),
                    row.agent_id.map(|agent_id| agent_id.uuid()),
                    &session_capabilities,
                    row.id.uuid(),
                )
                .await?;
            }

            let mut session = Self::row_to_session(row, org_public_id, Some(desired_harness_id));
            self.hydrate_ownership(org_id, &mut session).await?;
            self.populate_features(caller.org_id, &mut session).await?;
            self.resolve_session_agent_id(org_id, &mut session).await?;
            return Ok(session);
        }

        // Create a new chat session
        let harness_id_typed = HarnessId::from_uuid(harness_id);
        let input = CreateSessionRow {
            org_id,
            app_id: None,
            harness_id: Some(harness_id_typed),
            agent_id: None,
            agent_identity_id: None,
            owner_principal_id: owner_principal.id,
            resolved_owner_user_id: owner_principal.resolved_user_id,
            title: Some(title.to_string()),
            locale,
            tags: vec!["global-chat".to_string(), user_tag],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
            hints: None,
            max_iterations: None,
            blueprint_id: None,
            blueprint_config: None,
            network_access: None,
        };
        let row = self.db.create_session(input).await?;
        let session_id = row.id.uuid();
        let mut session = Self::row_to_session(row, org_public_id, Some(harness_id_typed));
        self.hydrate_ownership(org_id, &mut session).await?;
        self.populate_features(org_id, &mut session).await?;

        // Apply capability mounts
        self.apply_capability_mounts(org_id, harness_id, None, &[], session_id)
            .await?;

        Ok(session)
    }

    pub async fn delete(&self, caller: &Caller, id: Uuid) -> Result<bool> {
        // session_end lifecycle hooks fire before eviction so the hook command
        // can still read the session VFS. Advisory: failures never block the
        // delete. Resolve the session's capability list first (best-effort).
        let session_id = SessionId::from_uuid(id);
        if let Ok(Some(row)) = self.db.get_session(caller.org_id, session_id).await {
            let session_caps: Vec<AgentCapabilityConfig> =
                serde_json::from_value(row.capabilities.clone()).unwrap_or_default();
            let capabilities = self
                .resolve_session_capability_configs(
                    caller.org_id,
                    row.harness_id,
                    row.agent_id,
                    &session_caps,
                )
                .await;
            self.fire_session_lifecycle_hooks(
                caller.org_id,
                session_id,
                row.agent_id.map(|a| a.to_string()),
                &capabilities,
                everruns_core::user_hook_types::HookEvent::SessionEnd,
                serde_json::json!({ "reason": "deleted" }),
            )
            .await;
        }

        let deleted = self.db.delete_session(caller.org_id, session_id).await?;

        if deleted {
            self.session_file_service.evict_virtual_mounts(id);
        }

        Ok(deleted)
    }

    /// Resolve the effective capability configs for a session (harness chain +
    /// agent + session), used by the lifecycle-hook firing helpers. Best-effort:
    /// returns an empty list if the harness/agent can't be loaded, so a
    /// resolution failure degrades to "no hooks" rather than erroring a
    /// create/delete.
    async fn resolve_session_capability_configs(
        &self,
        org_id: i64,
        harness_id: Option<HarnessId>,
        agent_id: Option<AgentId>,
        session_caps: &[AgentCapabilityConfig],
    ) -> Vec<AgentCapabilityConfig> {
        let harness_caps = match harness_id {
            Some(harness_id) => match crate::domains::harnesses::queries::resolve_effective(
                self.db.as_ref(),
                org_id,
                harness_id,
            )
            .await
            {
                Ok(Some(h)) => h.capabilities,
                _ => Vec::new(),
            },
            None => Vec::new(),
        };
        let agent_caps = if let Some(agent_id) = agent_id {
            self.db
                .get_agent_capabilities(agent_id.uuid())
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|row| AgentCapabilityConfig::with_config(row.capability_id, row.config))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let merged = merge_capabilities(&harness_caps, &agent_caps);
        merge_capabilities(&merged, session_caps)
    }

    /// Fire session-lifecycle hooks (`session_start` / `session_end`) for a
    /// session. Advisory only — collects + finalizes hook specs from the given
    /// capability list, builds the bash-backed adapters against the session's
    /// VFS, and runs them. Any failure is logged, never propagated.
    async fn fire_session_lifecycle_hooks(
        &self,
        org_id: i64,
        session_id: SessionId,
        agent_id: Option<String>,
        capabilities: &[AgentCapabilityConfig],
        event: everruns_core::user_hook_types::HookEvent,
        data: serde_json::Value,
    ) {
        let resolved = match resolve_capability_configs(capabilities, &self.capability_registry) {
            Ok(r) => r,
            Err(error) => {
                tracing::warn!(
                    session_id = %session_id,
                    ?error,
                    "failed to resolve capabilities for session lifecycle hooks; skipping"
                );
                return;
            }
        };
        // Gather + finalize specs (namespace stamping, muting) identically to
        // the runtime act/turn paths.
        let mut contributions: Vec<(String, Vec<everruns_core::user_hook_types::UserHookSpec>)> =
            Vec::new();
        let mut disabled: Vec<String> = Vec::new();
        for config in &resolved {
            let Some(capability) = self.capability_registry.get(config.capability_id()) else {
                continue;
            };
            let specs = capability.user_hooks_with_config(&config.config);
            if !specs.is_empty() {
                contributions.push((config.capability_id().to_string(), specs));
            }
            if config.capability_id() == "user_hooks" {
                disabled.extend(
                    everruns_core::capabilities::user_hooks::disabled_contributions(&config.config),
                );
            }
        }
        let specs = everruns_core::hook_adapter::finalize_hook_specs(contributions, &disabled);
        let file_store: Arc<dyn everruns_core::traits::SessionFileSystem> = Arc::new(
            crate::domains::session_files::SessionFileService::new(self.db.clone()),
        );
        let dispatcher: Arc<dyn everruns_core::hook_executor::BashHookDispatcher> =
            Arc::new(everruns_core::hook_dispatch::BashkitShellHookDispatcher::new(file_store));
        let hooks = everruns_core::lifecycle_hooks::build_session_lifecycle_hooks(
            &specs, event, dispatcher,
        );
        if hooks.is_empty() {
            return;
        }
        let ctx = everruns_core::lifecycle_hooks::SessionHookContext {
            session_id,
            org_id: everruns_core::org_public_id_from_internal(org_id)
                .parse()
                .ok(),
            agent_id,
        };
        everruns_core::lifecycle_hooks::run_session_lifecycle_hooks(&hooks, &ctx, data).await;
    }

    /// Pin a session for a user
    pub async fn pin(&self, caller: &Caller, user_id: Uuid, session_id: Uuid) -> Result<()> {
        self.db
            .pin_session(user_id, SessionId::from_uuid(session_id), caller.org_id)
            .await
    }

    /// Unpin a session for a user in the caller's current org.
    /// Authorization is enforced at `Command::run` via `UnpinSession::policy`.
    pub async fn unpin(&self, caller: &Caller, user_id: Uuid, session_id: Uuid) -> Result<bool> {
        self.db
            .unpin_session(user_id, SessionId::from_uuid(session_id), caller.org_id)
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

    /// Collect merged capability configs for a session, preserving per-layer config.
    async fn collect_session_capability_configs(
        &self,
        org_id: i64,
        harness_id: Uuid,
        agent_id: Option<Uuid>,
        session_capabilities: &[AgentCapabilityConfig],
    ) -> Result<Vec<AgentCapabilityConfig>> {
        let mut capability_configs = self
            .resolve_effective_harness(org_id, HarnessId::from_uuid(harness_id))
            .await?
            .map(|harness| harness.capabilities)
            .unwrap_or_default();

        if let Some(agent_id) = agent_id
            && self
                .db
                .get_agent(org_id, AgentId::from_uuid(agent_id))
                .await?
                .is_some()
        {
            let agent_cap_rows = self.db.get_agent_capabilities(agent_id).await?;
            let agent_capabilities = agent_cap_rows
                .into_iter()
                .map(|row| AgentCapabilityConfig::with_config(row.capability_id, row.config))
                .collect::<Vec<_>>();
            capability_configs = merge_capabilities(&capability_configs, &agent_capabilities);
        }

        Ok(merge_capabilities(
            &capability_configs,
            session_capabilities,
        ))
    }

    async fn collect_workspace_memory_mounts(
        &self,
        org_id: i64,
        capability_configs: &[AgentCapabilityConfig],
    ) -> Result<Vec<MountPoint>> {
        let Some(config) = capability_configs
            .iter()
            .find(|config| config.capability_id() == MEMORY_CAPABILITY_ID)
        else {
            return Ok(vec![]);
        };
        let memory_config: MemoryConfig =
            serde_json::from_value(config.config.clone()).map_err(|error| {
                BadRequestError::new(format!("Invalid workspace memory config: {error}"))
            })?;
        let mut mounts = Vec::with_capacity(memory_config.mounts.len());

        for mount in memory_config.mounts {
            let memory_id = MemoryId::parse(&mount.memory)
                .map_err(|_| BadRequestError::new("Invalid workspace memory ID"))?;
            let memory = self
                .db
                .get_memory(org_id, memory_id)
                .await?
                .filter(|memory| memory.status == "active")
                .ok_or_else(|| ResourceNotFoundError::new("Memory"))?;

            if memory.is_readonly && mount.mode == MemoryMountAccess::ReadWrite {
                return Err(BadRequestError::new(format!(
                    "Memory {} is read-only and cannot be mounted readwrite",
                    mount.memory
                ))
                .into());
            }

            let access = if memory.is_readonly || mount.mode == MemoryMountAccess::ReadOnly {
                MountAccess::ReadOnly
            } else {
                MountAccess::ReadWrite
            };
            let files = self.db.list_all_memory_files(memory.id).await?;
            mounts.push(MountPoint::new(
                mount.path,
                access,
                MountSource::directory(memory_files_to_mount_entries(files)),
                MEMORY_CAPABILITY_ID,
            ));
        }

        Ok(mounts)
    }

    async fn require_admin_for_high_risk_session_capabilities(
        &self,
        caller: &Caller,
        org_id: i64,
        harness_id: Uuid,
        agent_id: Option<Uuid>,
        session_capabilities: &[AgentCapabilityConfig],
    ) -> Result<()> {
        if caller.role.has_permission(OrgRole::Admin) {
            return Ok(());
        }

        let mut capability_ids = self
            .collect_session_capability_ids(org_id, harness_id, agent_id, session_capabilities)
            .await?;
        // Expand declarative capability dependencies so hidden high-risk built-ins
        // (e.g. `web_fetch`, `bashkit_shell`) cannot bypass the admin gate. Validation
        // forbids nested declarative deps, so a single expansion pass is sufficient.
        let declarative_refs: Vec<String> = capability_ids
            .iter()
            .filter(|id| is_declarative_capability(id))
            .cloned()
            .collect();
        for cap_ref in &declarative_refs {
            let Some(name) = parse_declarative_capability_id(cap_ref) else {
                continue;
            };
            let Some(row) = self
                .db
                .get_declarative_capability_by_name(org_id, name)
                .await?
            else {
                continue;
            };
            // Fail closed: a broken declarative definition must NOT silently skip
            // dependency expansion, since this gates admin-only high-risk session
            // capability assignment.
            let definition: DeclarativeCapabilityDefinition =
                serde_json::from_value(row.definition).map_err(|error| {
                    anyhow::anyhow!(
                        "declarative capability {cap_ref} has malformed definition: {error}"
                    )
                })?;
            for dep in definition.dependencies {
                if !capability_ids.contains(&dep) {
                    capability_ids.push(dep);
                }
            }
        }
        let high_risk: Vec<String> = capability_ids
            .into_iter()
            .filter(|capability_id| {
                self.capability_registry
                    .get(capability_id)
                    .is_some_and(|capability| capability.risk_level() == RiskLevel::High)
            })
            .collect();
        if high_risk.is_empty() {
            return Ok(());
        }

        // EVE-437 / TM-AUTHZ: this is an authorization failure, not an
        // internal error. Returning an `anyhow::bail!` here used to map to
        // 500 because `classify_anyhow` did not recognize the message
        // substring. A `PolicyError` maps to `Forbidden` (403) at the
        // command boundary.
        Err(everruns_core::PolicyError::denied(
            "session_high_risk_capabilities",
            &format!(
                "Admin role required to create sessions with high-risk capabilities: {}",
                high_risk.join(", ")
            ),
        )
        .into())
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

    async fn hydrate_ownership(&self, org_id: i64, session: &mut Session) -> Result<()> {
        session.owner = self
            .principal_service
            .get_summary(org_id, session.owner_principal_id)
            .await?;
        session.effective_owner = self
            .principal_service
            .effective_owner_summary(org_id, session.resolved_owner_user_id)
            .await?;
        Ok(())
    }

    pub fn row_to_session(
        row: crate::storage::SessionRow,
        org_public_id: &str,
        fallback_harness: Option<HarnessId>,
    ) -> Session {
        // Convert database usage columns to TokenUsage. Actual and estimated cost
        // totals are tracked separately; the aggregate carries each so consumers
        // can prefer actual and reconcile drift.
        let usage = if row.total_input_tokens > 0 || row.total_output_tokens > 0 {
            Some(
                TokenUsage::with_cache(
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
                )
                .with_cost(
                    (row.total_actual_cost_usd > 0.0).then_some(row.total_actual_cost_usd),
                    (row.total_estimated_cost_usd > 0.0).then_some(row.total_estimated_cost_usd),
                ),
            )
        } else {
            None
        };

        // Parse capabilities from JSON
        let capabilities: Vec<AgentCapabilityConfig> =
            serde_json::from_value(row.capabilities).unwrap_or_default();

        Session {
            id: row.id,
            organization_id: org_public_id.to_string(),
            workspace_id: WorkspaceId::from_uuid(row.workspace_id),
            harness_id: row.harness_id.or(fallback_harness).unwrap_or_else(|| {
                panic!(
                    "session {} has no harness_id and no fallback was provided; \
                     ensure the org has a built-in 'base' harness provisioned",
                    row.id
                )
            }),
            agent_id: row.agent_id,
            agent_version_id: row.agent_version_id,
            agent_identity_id: row.agent_identity_id,
            owner_principal_id: row.owner_principal_id,
            resolved_owner_user_id: row.resolved_owner_user_id,
            owner: None,
            effective_owner: None,
            title: row.title,
            locale: row.locale,
            preview: None,        // Populated separately in list()
            output_preview: None, // Populated separately in list()
            tags: row.tags,
            model_id: row.model_id,
            capabilities,
            tools: serde_json::from_value(row.tools).unwrap_or_default(),
            mcp_servers: serde_json::from_value(row.mcp_servers).unwrap_or_default(),
            system_prompt: row.system_prompt,
            initial_files: serde_json::from_value(row.initial_files).unwrap_or_default(),
            network_access: row
                .network_access
                .and_then(|v| serde_json::from_value(v).ok()),
            hints: row.hints.and_then(|v| serde_json::from_value(v).ok()),
            max_iterations: max_iterations::from_db(row.max_iterations),
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
            blueprint_id: row.blueprint_id,
            blueprint_config: row.blueprint_config,
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

fn sanitize_session_capabilities(
    capabilities: Vec<AgentCapabilityConfig>,
) -> Vec<AgentCapabilityConfig> {
    capabilities
        .into_iter()
        .map(|mut capability| {
            if capability.capability_id() == SESSION_SANDBOX_CAPABILITY_ID
                && let Some(provider_config) = capability
                    .config
                    .get_mut("provider_config")
                    .and_then(serde_json::Value::as_object_mut)
            {
                let removed_api_base = provider_config.remove("api_base").is_some();
                let removed_toolbox_base = provider_config.remove("toolbox_base").is_some();
                if removed_api_base || removed_toolbox_base {
                    tracing::warn!(
                        "Ignoring session-level session_sandbox provider_config base URL overrides"
                    );
                }
            }
            capability
        })
        .collect()
}

fn memory_files_to_mount_entries(mut files: Vec<MemoryFileRow>) -> HashMap<String, MountEntry> {
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let mut entries = HashMap::new();

    for file in files {
        let path = file.path.trim_matches('/');
        if path.is_empty() {
            continue;
        }
        let segments = path
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>();
        if file.is_directory {
            insert_mount_directory(&mut entries, &segments);
            continue;
        }

        let bytes = file.content.unwrap_or_default();
        let (content, encoding) = SessionFile::encode_content(&bytes);
        insert_mount_file(&mut entries, &segments, content, encoding);
    }

    entries
}

fn insert_mount_directory(entries: &mut HashMap<String, MountEntry>, segments: &[&str]) {
    let Some((name, rest)) = segments.split_first() else {
        return;
    };
    let entry = entries
        .entry((*name).to_string())
        .or_insert_with(|| MountEntry::directory(HashMap::new()));
    if !entry.source.is_directory() {
        *entry = MountEntry::directory(HashMap::new());
    }
    if let MountSource::InlineDirectory { entries } = &mut entry.source {
        insert_mount_directory(entries, rest);
    }
}

fn insert_mount_file(
    entries: &mut HashMap<String, MountEntry>,
    segments: &[&str],
    content: String,
    encoding: String,
) {
    let Some((name, rest)) = segments.split_first() else {
        return;
    };
    if rest.is_empty() {
        entries.insert(
            (*name).to_string(),
            MountEntry::new(MountSource::InlineFile { content, encoding }),
        );
        return;
    }

    let entry = entries
        .entry((*name).to_string())
        .or_insert_with(|| MountEntry::directory(HashMap::new()));
    if !entry.source.is_directory() {
        *entry = MountEntry::directory(HashMap::new());
    }
    if let MountSource::InlineDirectory { entries } = &mut entry.source {
        insert_mount_file(entries, rest, content, encoding);
    }
}

// normalize_initial_file_path is imported from everruns_core::config_layer

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domains::common::{Command, Ctx};
    use crate::domains::memory::CreateMemory;
    use crate::domains::memory::types::{CreateMemorySourceRequest, GitMemorySourceRequest};
    use crate::domains::{
        agents::types::CreateAgentRequest, harnesses::types::CreateHarnessRequest,
    };
    use crate::services::{CapabilityService, PrincipalService};
    use crate::storage::{
        CreateHarnessRow, CreateLlmModelRow, CreateLlmProviderRow, CreateMemoryFileRow,
        CreateOrganizationRow, StorageBackend,
    };
    use everruns_core::capabilities::Capability;
    use everruns_core::{Caller, DEFAULT_ORG_ID, InitialFile, OrgRole};

    #[test]
    fn sanitize_session_capabilities_removes_daytona_base_url_overrides() {
        let capabilities = vec![AgentCapabilityConfig::with_config(
            SESSION_SANDBOX_CAPABILITY_ID,
            serde_json::json!({
                "provider": "daytona",
                "provider_config": {
                    "api_base": "https://attacker.example",
                    "toolbox_base": "https://attacker.example/toolbox",
                    "workspace_path": "/home/daytona/workspace",
                }
            }),
        )];

        let sanitized = sanitize_session_capabilities(capabilities);
        let provider_config = sanitized[0]
            .config
            .get("provider_config")
            .and_then(serde_json::Value::as_object)
            .expect("provider_config should be object");

        assert!(!provider_config.contains_key("api_base"));
        assert!(!provider_config.contains_key("toolbox_base"));
        assert_eq!(
            provider_config
                .get("workspace_path")
                .and_then(serde_json::Value::as_str),
            Some("/home/daytona/workspace")
        );
    }

    #[test]
    fn sanitize_session_capabilities_keeps_non_sandbox_capabilities() {
        let capabilities = vec![AgentCapabilityConfig::with_config(
            "shell",
            serde_json::json!({
                "provider_config": {
                    "api_base": "https://example.com"
                }
            }),
        )];

        let sanitized = sanitize_session_capabilities(capabilities.clone());
        assert_eq!(sanitized, capabilities);
    }

    fn test_ctx(caller: Caller, db: Arc<StorageBackend>) -> Ctx {
        let capability_service = Arc::new(CapabilityService::new(db.clone(), None));
        Ctx::new(
            caller,
            db,
            capability_service,
            None,
            Arc::new(everruns_core::DefaultPermissionResolver),
        )
    }

    fn external_caller(org_id: i64) -> Caller {
        Caller {
            org_id,
            org_public_id: everruns_core::organization::org_public_id_from_internal(org_id),
            user_id: None,
            role: OrgRole::Owner,
            is_platform_user: false,
            is_internal: false,
        }
    }

    fn build_create_request(
        harness_id: HarnessId,
        agent_id: Option<AgentId>,
        model_id: Option<ModelId>,
    ) -> CreateSessionRequest {
        CreateSessionRequest {
            harness_id: Some(harness_id),
            harness_name: None,
            agent_id,
            agent_identity_id: None,
            title: Some("Test Session".to_string()),
            locale: None,
            tags: vec![],
            model_id,
            capabilities: vec![],
            tools: vec![],
            mcp_servers: Default::default(),
            system_prompt: None,
            initial_files: vec![],
            hints: None,
            network_access: None,
            max_iterations: None,
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
                enabled: true,
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
    async fn app_backreference_is_only_set_by_app_session_create() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_service = SessionService::new(db.clone());
        let caller = Caller::internal(1);
        let ctx = test_ctx(caller.clone(), db.clone());

        let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "app-backref-harness".to_string(),
            display_name: Some("App Backref Harness".to_string()),
            description: None,
            system_prompt: "Harness prompt".to_string(),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
        })
        .execute(&ctx)
        .await
        .unwrap();

        let app_internal_id = Uuid::now_v7();
        // Build a real user principal to act as the App owner. This must be
        // distinct from the caller-derived default (the system principal that
        // `Caller::internal` resolves to) so the assertions below actually
        // exercise the override codepath rather than coincidentally matching.
        let principal_service = PrincipalService::new(db.clone());
        let user = db
            .create_user(crate::storage::CreateUserRow {
                external_id: None,
                email: "app-owner@example.com".to_string(),
                name: "App Owner".to_string(),
                avatar_url: None,
                roles: vec![],
                password_hash: None,
                email_verified: true,
                auth_provider: None,
                auth_provider_id: None,
            })
            .await
            .unwrap();
        let app_owner = principal_service
            .ensure_user_principal(1, user.id)
            .await
            .unwrap();
        let caller_default_owner = principal_service
            .default_owner_principal(&caller, None)
            .await
            .unwrap();
        assert_ne!(
            app_owner.id, caller_default_owner.id,
            "test setup invariant: app owner must differ from caller-derived default",
        );

        let app_session = session_service
            .create_from_app(
                &caller,
                harness.id.uuid(),
                None,
                None,
                app_internal_id,
                app_owner.id,
                app_owner.resolved_user_id,
                build_create_request(harness.id, None, None),
            )
            .await
            .unwrap();
        let stored_app_session = db
            .get_session(1, app_session.id)
            .await
            .unwrap()
            .expect("app session should be stored");
        assert_eq!(stored_app_session.app_id, Some(app_internal_id));
        // The override actually took effect: stored session is owned by the
        // App's owner, NOT the caller-derived system principal.
        assert_eq!(
            stored_app_session.owner_principal_id, app_owner.id,
            "create_from_app must persist the App's owner_principal_id",
        );
        assert_ne!(
            stored_app_session.owner_principal_id, caller_default_owner.id,
            "create_from_app must override the caller-derived default principal",
        );
        assert_eq!(
            stored_app_session.resolved_owner_user_id, app_owner.resolved_user_id,
            "create_from_app must persist the App's resolved_owner_user_id",
        );

        let normal_session = session_service
            .create(
                &caller,
                harness.id.uuid(),
                None,
                None,
                build_create_request(harness.id, None, None),
            )
            .await
            .unwrap();
        let stored_normal_session = db
            .get_session(1, normal_session.id)
            .await
            .unwrap()
            .expect("normal session should be stored");
        assert_eq!(stored_normal_session.app_id, None);
    }

    #[tokio::test]
    async fn starter_files_are_copied_into_new_sessions() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_service = SessionService::new(db.clone());
        let caller = Caller::internal(1);
        let ctx = test_ctx(caller.clone(), db.clone());

        let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "harness".to_string(),
            display_name: Some("Harness".to_string()),
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
            mcp_servers: Default::default(),
            network_access: None,
        })
        .execute(&ctx)
        .await
        .unwrap();

        let agent = crate::domains::agents::CreateAgent(CreateAgentRequest {
            id: None,
            name: "test-agent".to_string(),
            display_name: Some("Test Agent".to_string()),
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
            mcp_servers: Default::default(),
            network_access: None,
            max_iterations: None,
        })
        .execute(&ctx)
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
        let session_service = SessionService::new(db.clone());
        let caller = Caller::internal(1);
        let ctx = test_ctx(caller.clone(), db.clone());

        let parent = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "parent".to_string(),
            display_name: Some("Parent".to_string()),
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
            mcp_servers: Default::default(),
            network_access: None,
        })
        .execute(&ctx)
        .await
        .unwrap();

        let child = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "child".to_string(),
            display_name: Some("Child".to_string()),
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
            mcp_servers: Default::default(),
            network_access: None,
        })
        .execute(&ctx)
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
        let session_service = SessionService::new(db.clone());
        let caller = Caller::internal(1);
        let ctx = test_ctx(caller.clone(), db.clone());

        let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "harness".to_string(),
            display_name: Some("Harness".to_string()),
            description: None,
            system_prompt: "Harness prompt".to_string(),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
        })
        .execute(&ctx)
        .await
        .unwrap();

        let agent = crate::domains::agents::CreateAgent(CreateAgentRequest {
            id: None,
            name: "test-agent".to_string(),
            display_name: Some("Test Agent".to_string()),
            description: None,
            system_prompt: "Agent prompt".to_string(),
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            tools: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            max_iterations: None,
        })
        .execute(&ctx)
        .await
        .unwrap();

        crate::domains::harnesses::DeleteHarness {
            id: harness.id.to_string(),
        }
        .execute(&ctx)
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

        let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "harness-2".to_string(),
            display_name: Some("Harness 2".to_string()),
            description: None,
            system_prompt: "Harness prompt".to_string(),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
        })
        .execute(&ctx)
        .await
        .unwrap();
        crate::domains::agents::DeleteAgent {
            id: agent.public_id.to_string(),
        }
        .execute(&ctx)
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
        let session_service = SessionService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let other_org_id = create_second_org(&db).await;
        let other_ctx = test_ctx(Caller::internal(other_org_id), db.clone());

        let other_harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "other-harness".to_string(),
            display_name: Some("Other Harness".to_string()),
            description: None,
            system_prompt: "Other".to_string(),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
        })
        .execute(&other_ctx)
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
        let session_service = SessionService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let ctx = test_ctx(caller.clone(), db.clone());
        let other_org_id = create_second_org(&db).await;
        let other_model_id = create_model(&db, other_org_id, "cross-org-model").await;

        let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "harness".to_string(),
            display_name: Some("Harness".to_string()),
            description: None,
            system_prompt: "Harness".to_string(),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
        })
        .execute(&ctx)
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
        let session_service = SessionService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let other_org_id = create_second_org(&db).await;
        let other_ctx = test_ctx(Caller::internal(other_org_id), db.clone());

        let other_harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "other-harness".to_string(),
            display_name: Some("Other Harness".to_string()),
            description: None,
            system_prompt: "Other".to_string(),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![AgentCapabilityConfig::new("sample_data")],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
        })
        .execute(&other_ctx)
        .await
        .unwrap();

        let other_agent = crate::domains::agents::CreateAgent(CreateAgentRequest {
            id: None,
            name: "other-agent".to_string(),
            display_name: Some("Other Agent".to_string()),
            description: None,
            system_prompt: "Other".to_string(),
            default_model_id: None,
            tags: vec![],
            capabilities: vec![AgentCapabilityConfig::new("session_schedule")],
            initial_files: vec![],
            tools: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            max_iterations: None,
        })
        .execute(&other_ctx)
        .await
        .unwrap();

        let session_row = db
            .create_session(CreateSessionRow {
                org_id: caller.org_id,
                app_id: None,
                harness_id: Some(other_harness.id),
                agent_id: Some(AgentId::from_uuid(other_agent.internal_id)),
                agent_identity_id: None,
                owner_principal_id: everruns_core::PrincipalId::from_seed(1),
                resolved_owner_user_id: None,
                title: Some("Corrupt Session".to_string()),
                locale: None,
                tags: vec![],
                model_id: None,
                capabilities: serde_json::to_value(vec![AgentCapabilityConfig::new(
                    "session_sql_database",
                )])
                .unwrap(),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                system_prompt: None,
                initial_files: serde_json::Value::Array(vec![]),
                hints: None,
                max_iterations: None,
                blueprint_id: None,
                blueprint_config: None,
                network_access: None,
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

    struct TestHighRiskCapability;

    impl Capability for TestHighRiskCapability {
        fn id(&self) -> &str {
            "test_high_risk"
        }

        fn name(&self) -> &str {
            "Test High Risk"
        }

        fn description(&self) -> &str {
            "Test-only high risk capability"
        }

        fn risk_level(&self) -> RiskLevel {
            RiskLevel::High
        }
    }

    #[tokio::test]
    async fn create_rejects_high_risk_harness_capabilities_for_members() {
        let db = Arc::new(StorageBackend::in_memory());
        let mut registry = CapabilityRegistry::new();
        registry.register(TestHighRiskCapability);
        let session_service = SessionService::with_registry(db.clone(), registry);
        let owner = Caller::internal(DEFAULT_ORG_ID);

        let harness = db
            .create_harness(
                owner.org_id,
                CreateHarnessRow {
                    name: "restricted-harness".to_string(),
                    display_name: Some("Restricted Harness".to_string()),
                    description: None,
                    system_prompt: "restricted".to_string(),
                    parent_harness_id: None,
                    default_model_id: None,
                    tags: vec![],
                    initial_files: serde_json::json!([]),
                    mcp_servers: serde_json::json!({}),
                    network_access: None,
                    is_built_in: false,
                },
            )
            .await
            .unwrap();
        db.set_harness_capabilities(
            harness.id.uuid(),
            vec![("test_high_risk".to_string(), 0, serde_json::json!({}))],
        )
        .await
        .unwrap();

        let member = Caller {
            org_id: owner.org_id,
            org_public_id: owner.org_public_id.clone(),
            user_id: None,
            role: OrgRole::Member,
            is_platform_user: false,
            is_internal: false,
        };

        let err = session_service
            .create(
                &member,
                harness.id.uuid(),
                None,
                None,
                build_create_request(harness.id, None, None),
            )
            .await
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("Admin role required to create sessions with high-risk capabilities"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn create_rejects_declarative_capability_with_high_risk_dependency_for_members() {
        use crate::storage::models::CreateDeclarativeCapabilityRow;

        let db = Arc::new(StorageBackend::in_memory());
        let mut registry = CapabilityRegistry::new();
        registry.register(TestHighRiskCapability);
        let session_service = SessionService::with_registry(db.clone(), registry);
        let owner = Caller::internal(DEFAULT_ORG_ID);

        // Declarative capability that hides a high-risk built-in dependency.
        db.create_declarative_capability(
            owner.org_id,
            CreateDeclarativeCapabilityRow {
                public_id: everruns_core::DeclarativeCapabilityId::new().to_string(),
                name: "hidden_admin_tool".to_string(),
                display_name: Some("Hidden Admin Tool".to_string()),
                description: "wraps a high-risk built-in".to_string(),
                definition: serde_json::json!({
                    "name": "hidden_admin_tool",
                    "description": "wraps a high-risk built-in",
                    "dependencies": ["test_high_risk"],
                }),
            },
        )
        .await
        .unwrap();

        let harness = db
            .create_harness(
                owner.org_id,
                CreateHarnessRow {
                    name: "declarative-harness".to_string(),
                    display_name: Some("Declarative Harness".to_string()),
                    description: None,
                    system_prompt: "declarative".to_string(),
                    parent_harness_id: None,
                    default_model_id: None,
                    tags: vec![],
                    initial_files: serde_json::json!([]),
                    mcp_servers: serde_json::json!({}),
                    network_access: None,
                    is_built_in: false,
                },
            )
            .await
            .unwrap();
        db.set_harness_capabilities(
            harness.id.uuid(),
            vec![(
                "declarative:hidden_admin_tool".to_string(),
                0,
                serde_json::json!({}),
            )],
        )
        .await
        .unwrap();

        let member = Caller {
            org_id: owner.org_id,
            org_public_id: owner.org_public_id.clone(),
            user_id: None,
            role: OrgRole::Member,
            is_platform_user: false,
            is_internal: false,
        };

        let err = session_service
            .create(
                &member,
                harness.id.uuid(),
                None,
                None,
                build_create_request(harness.id, None, None),
            )
            .await
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("Admin role required to create sessions with high-risk capabilities"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn apply_capability_mounts_skips_foreign_harness_and_agent_capabilities() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_service = SessionService::new(db.clone());
        let file_service = SessionFileService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let other_org_id = create_second_org(&db).await;
        let other_ctx = test_ctx(Caller::internal(other_org_id), db.clone());

        let other_harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "other-harness".to_string(),
            display_name: Some("Other Harness".to_string()),
            description: None,
            system_prompt: "Other".to_string(),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![AgentCapabilityConfig::new("sample_data")],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
        })
        .execute(&other_ctx)
        .await
        .unwrap();

        let other_agent = crate::domains::agents::CreateAgent(CreateAgentRequest {
            id: None,
            name: "other-agent".to_string(),
            display_name: Some("Other Agent".to_string()),
            description: None,
            system_prompt: "Other".to_string(),
            default_model_id: None,
            tags: vec![],
            capabilities: vec![AgentCapabilityConfig::new("sample_data")],
            initial_files: vec![],
            tools: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            max_iterations: None,
        })
        .execute(&other_ctx)
        .await
        .unwrap();

        let session_row = db
            .create_session(CreateSessionRow {
                org_id: caller.org_id,
                app_id: None,
                harness_id: None,
                agent_id: None,
                agent_identity_id: None,
                owner_principal_id: everruns_core::PrincipalId::from_seed(1),
                resolved_owner_user_id: None,
                title: Some("Mount Test".to_string()),
                locale: None,
                tags: vec![],
                model_id: None,
                capabilities: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                system_prompt: None,
                initial_files: serde_json::Value::Array(vec![]),
                hints: None,
                max_iterations: None,
                blueprint_id: None,
                blueprint_config: None,
                network_access: None,
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

    #[tokio::test]
    async fn workspace_memory_mount_materializes_source_memory_readonly() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_service = SessionService::new(db.clone());
        let file_service = SessionFileService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let ctx = test_ctx(caller.clone(), db.clone());

        let memory = CreateMemory {
            name: "Repo Memory".to_string(),
            description: None,
            source: Some(CreateMemorySourceRequest::Git(GitMemorySourceRequest {
                url: "https://example.com/org/repo.git".to_string(),
                branch: None,
                root_folder: None,
                sync_interval_secs: None,
            })),
        }
        .execute(&ctx)
        .await
        .unwrap();
        let claimed = db
            .claim_next_memory_sync()
            .await
            .unwrap()
            .expect("memory should be pending sync");
        db.complete_memory_sync(
            claimed.id,
            claimed.updated_at,
            vec![CreateMemoryFileRow {
                path: "/README.md".to_string(),
                content: Some(b"hello from memory".to_vec()),
                is_directory: false,
                content_hash: None,
            }],
        )
        .await
        .unwrap();

        let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "memory-harness".to_string(),
            display_name: Some("Memory Harness".to_string()),
            description: None,
            system_prompt: "Harness prompt".to_string(),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![AgentCapabilityConfig::with_config(
                MEMORY_CAPABILITY_ID,
                serde_json::json!({
                    "mounts": [{
                        "memory": memory.id.to_string(),
                        "path": "/workspace/repo",
                        "mode": "readonly"
                    }]
                }),
            )],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
        })
        .execute(&ctx)
        .await
        .unwrap();

        let session = session_service
            .create(
                &caller,
                harness.id.uuid(),
                None,
                None,
                build_create_request(harness.id, None, None),
            )
            .await
            .unwrap();
        let file = file_service
            .read_file(session.id.uuid(), "/workspace/repo/README.md")
            .await
            .unwrap()
            .expect("mounted file should exist");

        let content = SessionFile::decode_content(
            file.content.as_deref().expect("mounted file has content"),
            &file.encoding,
        )
        .unwrap();
        assert_eq!(content, b"hello from memory");
        assert!(file.is_readonly);
    }

    #[tokio::test]
    async fn workspace_memory_mount_rejects_readwrite_source_volume() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_service = SessionService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let ctx = test_ctx(caller.clone(), db.clone());

        let memory = CreateMemory {
            name: "Read-only Repo".to_string(),
            description: None,
            source: Some(CreateMemorySourceRequest::Git(GitMemorySourceRequest {
                url: "https://example.com/org/repo.git".to_string(),
                branch: None,
                root_folder: None,
                sync_interval_secs: None,
            })),
        }
        .execute(&ctx)
        .await
        .unwrap();
        let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "readwrite-memory-harness".to_string(),
            display_name: Some("Readwrite Memory Harness".to_string()),
            description: None,
            system_prompt: "Harness prompt".to_string(),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![AgentCapabilityConfig::with_config(
                MEMORY_CAPABILITY_ID,
                serde_json::json!({
                    "mounts": [{
                        "memory": memory.id.to_string(),
                        "path": "/workspace/repo",
                        "mode": "readwrite"
                    }]
                }),
            )],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
        })
        .execute(&ctx)
        .await
        .unwrap();

        let err = session_service
            .create(
                &caller,
                harness.id.uuid(),
                None,
                None,
                build_create_request(harness.id, None, None),
            )
            .await
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("is read-only and cannot be mounted readwrite"),
            "unexpected error: {err}",
        );
    }

    #[tokio::test]
    async fn update_rejects_reserved_internal_tags_for_external_callers() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_service = SessionService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let ctx = test_ctx(caller.clone(), db.clone());

        let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "harness".to_string(),
            display_name: Some("Harness".to_string()),
            description: None,
            system_prompt: "Harness prompt".to_string(),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
        })
        .execute(&ctx)
        .await
        .unwrap();

        let session = session_service
            .create(
                &caller,
                harness.id.uuid(),
                None,
                None,
                build_create_request(harness.id, None, None),
            )
            .await
            .unwrap();

        let err = session_service
            .update(
                &external_caller(DEFAULT_ORG_ID),
                session.id.uuid(),
                UpdateSessionRequest {
                    title: None,
                    agent_identity_id: UpdateField::Unchanged,
                    locale: None,
                    tags: Some(vec!["__internal:app_invocation".to_string()]),
                },
            )
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("Tags with '__internal:' prefix are reserved")
        );
    }

    #[tokio::test]
    async fn update_rejects_reserved_app_tags_for_external_callers() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_service = SessionService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let ctx = test_ctx(caller.clone(), db.clone());

        let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "harness".to_string(),
            display_name: Some("Harness".to_string()),
            description: None,
            system_prompt: "Harness prompt".to_string(),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
        })
        .execute(&ctx)
        .await
        .unwrap();

        let session = session_service
            .create(
                &caller,
                harness.id.uuid(),
                None,
                None,
                build_create_request(harness.id, None, None),
            )
            .await
            .unwrap();

        for forbidden in [
            vec!["app:app_other".to_string()],
            vec!["app_channel:appchan_other".to_string()],
            vec!["slack:app:app_legacy_other".to_string()],
            vec!["ag_ui:app:app_ag_ui_other".to_string()],
        ] {
            let err = session_service
                .update(
                    &external_caller(DEFAULT_ORG_ID),
                    session.id.uuid(),
                    UpdateSessionRequest {
                        title: None,
                        agent_identity_id: UpdateField::Unchanged,
                        locale: None,
                        tags: Some(forbidden.clone()),
                    },
                )
                .await
                .unwrap_err();
            assert!(
                err.to_string().contains("reserved for internal subsystems"),
                "got: {err} for tags: {forbidden:?}"
            );
        }
    }

    #[tokio::test]
    async fn create_rejects_reserved_app_tags_for_external_callers() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_service = SessionService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let ctx = test_ctx(caller.clone(), db.clone());

        let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "harness".to_string(),
            display_name: Some("Harness".to_string()),
            description: None,
            system_prompt: "Harness prompt".to_string(),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
        })
        .execute(&ctx)
        .await
        .unwrap();

        for forbidden in [
            "app:app_someone_else",
            "app_channel:appchan_someone_else",
            "slack:app:app_legacy_someone_else",
            "ag_ui:app:app_ag_ui_someone_else",
        ] {
            let mut req = build_create_request(harness.id, None, None);
            req.tags = vec![forbidden.to_string()];

            let err = session_service
                .create(
                    &external_caller(DEFAULT_ORG_ID),
                    harness.id.uuid(),
                    None,
                    None,
                    req,
                )
                .await
                .unwrap_err();
            assert!(
                err.to_string().contains("reserved for internal subsystems"),
                "got: {err} for tag: {forbidden}"
            );
        }
    }

    #[tokio::test]
    async fn concurrent_session_cap_enforced() {
        use crate::domains::sessions::limits::OrgCaps;
        use crate::errors::BadRequestError;
        use crate::storage::models::UpdateSession;

        let db = Arc::new(StorageBackend::in_memory());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let ctx = test_ctx(caller.clone(), db.clone());

        let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "cap-test-harness".to_string(),
            display_name: Some("Cap Test Harness".to_string()),
            description: None,
            system_prompt: "test".to_string(),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
        })
        .execute(&ctx)
        .await
        .unwrap();

        let svc = SessionService::new(db.clone()).with_caps(OrgCaps {
            max_concurrent_sessions: 1,
            max_active_turns: 1_000,
        });

        // First session succeeds.
        let session = svc
            .create(
                &caller,
                harness.id.uuid(),
                None,
                None,
                build_create_request(harness.id, None, None),
            )
            .await
            .unwrap();

        // Ensure the session is in an active status (created as 'started' in in-memory backend).
        let _ = db
            .update_session(
                DEFAULT_ORG_ID,
                session.id,
                UpdateSession {
                    status: Some("started".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // Second session is rejected because cap = 1 is already reached.
        let err = svc
            .create(
                &caller,
                harness.id.uuid(),
                None,
                None,
                build_create_request(harness.id, None, None),
            )
            .await
            .unwrap_err();

        assert!(
            err.downcast_ref::<BadRequestError>().is_some(),
            "expected BadRequestError, got: {err}"
        );
        assert!(
            err.to_string().contains("Too many concurrent sessions"),
            "got: {err}"
        );
    }

    // Build a harness carrying a `user_hooks` capability with a single hook of
    // the given event that writes a sentinel into the session VFS. Exercises the
    // real server resolve -> finalize -> bashkit_shell dispatch path end-to-end.
    async fn harness_with_user_hook(
        db: &Arc<StorageBackend>,
        org_id: i64,
        name: &str,
        event: &str,
        command: &str,
    ) -> HarnessId {
        let harness = db
            .create_harness(
                org_id,
                CreateHarnessRow {
                    name: name.to_string(),
                    display_name: Some(name.to_string()),
                    description: None,
                    system_prompt: "hooked".to_string(),
                    parent_harness_id: None,
                    default_model_id: None,
                    tags: vec![],
                    initial_files: serde_json::json!([]),
                    mcp_servers: serde_json::json!({}),
                    network_access: None,
                    is_built_in: false,
                },
            )
            .await
            .unwrap();
        db.set_harness_capabilities(
            harness.id.uuid(),
            vec![(
                "user_hooks".to_string(),
                0,
                serde_json::json!({
                    "hooks": [{
                        "event": event,
                        "executor": { "type": "bash", "command": command },
                    }]
                }),
            )],
        )
        .await
        .unwrap();
        harness.id
    }

    #[tokio::test]
    async fn session_start_hook_fires_on_create() {
        let db = Arc::new(StorageBackend::in_memory());
        let mut registry = CapabilityRegistry::new();
        registry.register(everruns_core::capabilities::UserHooksCapability);
        let session_service = SessionService::with_registry(db.clone(), registry);
        let caller = Caller::internal(DEFAULT_ORG_ID);

        let harness_id = harness_with_user_hook(
            &db,
            caller.org_id,
            "session-start-harness",
            "session_start",
            "echo started > /workspace/.session_start_ok",
        )
        .await;

        let session = session_service
            .create(
                &caller,
                harness_id.uuid(),
                None,
                None,
                build_create_request(harness_id, None, None),
            )
            .await
            .unwrap();

        // The session_start hook ran during create and wrote into the VFS.
        let file = SessionFileService::new(db)
            .read_file(session.id.uuid(), "/.session_start_ok")
            .await
            .unwrap();
        assert!(
            file.as_ref()
                .is_some_and(|f| f.content.as_deref().is_some_and(|c| c.contains("started"))),
            "session_start hook should have written the sentinel, got: {file:?}"
        );
    }

    #[tokio::test]
    async fn session_end_hook_fires_on_delete_without_blocking() {
        let db = Arc::new(StorageBackend::in_memory());
        let mut registry = CapabilityRegistry::new();
        registry.register(everruns_core::capabilities::UserHooksCapability);
        let session_service = SessionService::with_registry(db.clone(), registry);
        let caller = Caller::internal(DEFAULT_ORG_ID);

        let harness_id = harness_with_user_hook(
            &db,
            caller.org_id,
            "session-end-harness",
            "session_end",
            "echo ending > /workspace/.session_end_ok",
        )
        .await;

        let session = session_service
            .create(
                &caller,
                harness_id.uuid(),
                None,
                None,
                build_create_request(harness_id, None, None),
            )
            .await
            .unwrap();

        // session_end is advisory: the hook fires (resolve -> dispatch) but never
        // blocks the delete, which must still succeed.
        let deleted = session_service
            .delete(&caller, session.id.uuid())
            .await
            .unwrap();
        assert!(
            deleted,
            "delete should succeed with a session_end hook present"
        );
    }
}
