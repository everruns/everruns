// Session service for business logic (M2)
//
// Design Decision: Capability mounts are applied at session creation time.
// This ensures mounted files are available immediately when the session starts.
// The service collects mounts from the agent's capabilities and applies them
// to the session filesystem. Session capabilities are applied after agent capabilities
// (additive behavior).

use super::types::{SessionFacetCount, SessionFacetsResponse};
use crate::api::common::Pagination;
use crate::auth::rate_limit::OrgRateLimiter;
use crate::domains::harnesses::queries::resolve_effective as resolve_effective_harness;
use crate::domains::session_files::{CreateFileInput, WorkspaceFileService};
use crate::domains::session_sandbox::SessionSandboxService;
use crate::domains::sessions::limits::OrgCaps;
use crate::errors::{BadRequestError, ResourceLimitError, ResourceNotFoundError};
use crate::max_iterations;
use crate::org_init;
use crate::server::ResourceLimitsConfig;
use crate::services::{PrincipalService, row_to_principal};
use crate::storage::{
    StorageBackend,
    models::{
        CreateEventRow, CreateMemoryRow, CreateSessionFileRow, CreateSessionRow, MemoryFileRow,
        MemoryRow, SessionListFilters, UpdateSession, UpsertSessionKeyValue, UpsertSessionSecret,
    },
};
use anyhow::Result;
use everruns_core::session_sandbox::SESSION_SANDBOX_CAPABILITY_ID;
use everruns_core::{AgentCapabilityConfig, AgentId, Caller, CapabilityRegistry};
use everruns_core::{
    DeclarativeCapabilityDefinition, FeatureFlags, HarnessId, InitialFile, ModelId, MountAccess,
    MountEntry, MountPoint, MountSource, OrgRole, Permission, Policy, PrincipalId,
    PrincipalSummary, Rule, SessionFile, SessionId, SessionSeedMode, TokenUsage, WorkspaceId,
    capabilities::{
        AttachSkillCapability, MEMORY_CAPABILITY_ID, RiskLevel, SystemPromptContext,
        collect_capabilities_with_configs, compute_features, resolve_capability_configs,
    },
    is_declarative_capability, is_mcp_capability, is_plugin_capability, is_skill_capability,
    memory::{MemoryConfig, MemoryMountAccess},
    merge_capabilities, merge_initial_files, normalize_initial_file_path,
    parse_declarative_capability_id, parse_skill_capability_id,
    typed_id::MemoryId,
};
use everruns_durable::UpdateField;
use everruns_platform::AgentVersionPolicy;
use everruns_platform::{Session, SessionActivity, SessionSource, SessionStatus};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use uuid::Uuid;

use crate::api::sessions::{CreateSessionRequest, UpdateSessionRequest};

const AGENT_MEMORY_MOUNT_PATH: &str = "/memory/agent";
const USER_MEMORY_MOUNT_PATH: &str = "/memory/user";

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

/// Optional, caller-supplied overrides applied when forking a session
/// (knowledge/runtime-resources/forking-sessions.md). Every field omitted (`None`) inherits the
/// parent session's value.
#[derive(Debug, Clone, Default)]
pub struct ForkOverrides {
    pub title: Option<String>,
    pub goal: Option<String>,
    pub tags: Option<Vec<String>>,
    pub model_id: Option<ModelId>,
    pub agent_id: Option<AgentId>,
    pub locale: Option<String>,
    pub system_prompt: Option<String>,
}

/// Session counts grouped by status.
#[derive(Debug, Clone, Default)]
pub struct SessionStats {
    pub total: u32,
    pub active: u32,
    pub idle: u32,
    pub started: u32,
    pub waiting_for_tool_results: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum GetOrCreateChatSessionError {
    #[error("chat session creation rate limit exceeded")]
    RateLimited,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub struct SessionService {
    db: Arc<StorageBackend>,
    principal_service: PrincipalService,
    capability_registry: CapabilityRegistry,
    session_file_service: WorkspaceFileService,
    session_sandbox_service: Option<Arc<SessionSandboxService>>,
    caps: OrgCaps,
    resource_limits: ResourceLimitsConfig,
}

#[derive(Default)]
struct SessionListHydration {
    owners: HashMap<PrincipalId, PrincipalSummary>,
    effective_owners: HashMap<Uuid, PrincipalSummary>,
    agent_public_ids: HashMap<AgentId, AgentId>,
    agent_capability_ids: HashMap<AgentId, Vec<String>>,
    harness_capability_ids: HashMap<HarnessId, Vec<String>>,
}

#[derive(Clone, Copy, Debug, Default)]
struct ScopedMemoryContext {
    agent_id: Option<AgentId>,
    user_id: Option<Uuid>,
}

impl SessionService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self {
            principal_service: PrincipalService::new(db.clone()),
            capability_registry: everruns_platform::capabilities::hosted_capability_registry(),
            session_file_service: WorkspaceFileService::new(db.clone()),
            db,
            session_sandbox_service: None,
            caps: OrgCaps::from_env(),
            resource_limits: ResourceLimitsConfig::from_env(),
        }
    }

    /// Create a new SessionService with a custom capability registry.
    pub fn with_registry(db: Arc<StorageBackend>, registry: CapabilityRegistry) -> Self {
        Self {
            principal_service: PrincipalService::new(db.clone()),
            capability_registry: registry,
            session_file_service: WorkspaceFileService::new(db.clone()),
            db,
            session_sandbox_service: None,
            caps: OrgCaps::from_env(),
            resource_limits: ResourceLimitsConfig::from_env(),
        }
    }

    pub fn with_caps(mut self, caps: OrgCaps) -> Self {
        self.caps = caps;
        self
    }

    pub fn with_resource_limits(mut self, resource_limits: ResourceLimitsConfig) -> Self {
        self.resource_limits = resource_limits;
        self
    }

    /// Attach a virtual mount registry to the internal session file service.
    pub fn with_virtual_registry(
        mut self,
        registry: Arc<crate::domains::session_files::virtual_mount_registry::VirtualMountRegistry>,
    ) -> Self {
        self.session_file_service =
            WorkspaceFileService::new(self.db.clone()).with_virtual_registry(registry);
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
        source: SessionSource,
        req: CreateSessionRequest,
    ) -> Result<Session> {
        self.create_inner(
            caller,
            harness_id,
            agent_internal_id,
            agent_public_id,
            None,
            None,
            source,
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
    /// fail to match it. See `knowledge/integrations/app-invocation-channels.md` and EVE-A2A
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
        source: SessionSource,
        req: CreateSessionRequest,
    ) -> Result<Session> {
        self.create_inner(
            caller,
            harness_id,
            agent_internal_id,
            agent_public_id,
            Some(app_internal_id),
            Some((owner_principal_id, resolved_owner_user_id)),
            source,
            req,
        )
        .await
    }

    /// Create a session owned by an agent that its own schedule trigger woke
    /// (EVE-757). Mirrors [`Self::create_from_app`] but there is no App row:
    /// the session runs on the agent's harness (P1), is hosted by the agent
    /// (P2, via `agent_public_id`), and is owned by `owner_principal_id` so the
    /// shared-session reuse lookup (`find_session_by_tags_and_owner`) matches
    /// across fires. `app_id` is `None`.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_from_agent_trigger(
        &self,
        caller: &Caller,
        harness_id: Uuid,
        agent_internal_id: Uuid,
        agent_public_id: AgentId,
        owner_principal_id: PrincipalId,
        resolved_owner_user_id: Option<Uuid>,
        req: CreateSessionRequest,
    ) -> Result<Session> {
        self.create_inner(
            caller,
            harness_id,
            Some(agent_internal_id),
            Some(agent_public_id),
            None,
            Some((owner_principal_id, resolved_owner_user_id)),
            // An agent trigger is a schedule fire by construction.
            SessionSource::Schedule,
            req,
        )
        .await
    }

    /// Fork a session into a new, independent session (knowledge/runtime-resources/forking-sessions.md).
    ///
    /// Creates a fresh session that is config-identical to `parent_id` (modulo
    /// `overrides`), then deep-copies the parent's conversation history (events)
    /// and workspace files into it. Leased resources, sandboxes, tasks, and
    /// schedules are intentionally not copied — the fork re-leases on demand.
    /// Fork provenance is recorded via `set_session_fork_lineage`.
    ///
    /// Existence/active-status of the parent are also checked by the calling
    /// command for precise HTTP status codes; this method is the service-side
    /// source of truth for the config to copy and the workspace to clone.
    pub async fn fork(
        &self,
        caller: &Caller,
        parent_id: SessionId,
        overrides: ForkOverrides,
    ) -> Result<Session> {
        let org_id = caller.org_id;

        let parent_row = self
            .db
            .get_session(org_id, parent_id)
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("Session"))?;
        let parent = Self::row_to_session(parent_row, &caller.org_public_id, None);

        // Resolve the agent's internal id (public -> internal) when one is
        // assigned, mirroring CreateSession.
        let agent_public = overrides.agent_id.or(parent.agent_id);
        let (agent_internal_id, agent_public_id) = if let Some(agent_id) = agent_public {
            match self
                .db
                .get_agent_by_public_id(org_id, &agent_id.to_string())
                .await?
            {
                Some(row) => {
                    let public_id: AgentId = row
                        .public_id
                        .parse()
                        .unwrap_or_else(|_| AgentId::from_uuid(row.id.uuid()));
                    (Some(row.id.uuid()), Some(public_id))
                }
                None => {
                    return Err(ResourceNotFoundError::new("Agent").into());
                }
            }
        } else {
            (None, None)
        };

        let title = overrides.title.or_else(|| {
            Some(match parent.title.as_deref() {
                Some(t) => format!("{t} (fork)"),
                None => "Fork".to_string(),
            })
        });
        let goal = overrides.goal.or(parent.goal);
        let harness_uuid = parent.harness_id.uuid();

        // Build a create request from the parent's config + overrides. A new
        // isolated workspace is forced (`workspace_id: None`); never a subagent
        // (`parent_session_id: None`).
        let req = CreateSessionRequest {
            source: None,
            harness_id: Some(parent.harness_id),
            harness_name: None,
            agent_id: agent_public_id,
            agent_name: None,
            agent_identity_id: parent.agent_identity_id,
            title,
            goal,
            locale: overrides.locale.or(parent.locale),
            tags: overrides.tags.unwrap_or(parent.tags),
            model_id: overrides.model_id.or(parent.model_id),
            capabilities: parent.capabilities,
            tools: parent.tools,
            mcp_servers: parent.mcp_servers,
            system_prompt: overrides.system_prompt.or(parent.system_prompt),
            initial_files: parent.initial_files,
            hints: parent.hints,
            network_access: parent.network_access,
            max_iterations: parent.max_iterations,
            parallel_tool_calls: parent.parallel_tool_calls,
            parent_session_id: None,
            forked_from_session_id: Some(parent_id),
            budget_root_session_id: None,
            seed: SessionSeedMode::Fork,
            workspace_id: None,
        };

        let child = self
            .create_inner(
                caller,
                harness_uuid,
                agent_internal_id,
                agent_public_id,
                None,
                None,
                // A fork keeps the origin of what it branched from: a forked
                // chat thread is still a chat thread.
                parent.source,
                req,
            )
            .await?;

        Ok(child)
    }

    async fn apply_session_seed(
        &self,
        org_id: i64,
        source_session_id: SessionId,
        child_session_id: SessionId,
        child_workspace_id: Uuid,
        seed: SessionSeedMode,
        child: &mut Session,
    ) -> Result<()> {
        let source = self
            .db
            .get_session(org_id, source_session_id)
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("Source session"))?;
        let fork_sequence = if seed == SessionSeedMode::Fork {
            Some(
                self.copy_session_events(source_session_id, child_session_id)
                    .await?,
            )
            .flatten()
        } else {
            None
        };

        if matches!(seed, SessionSeedMode::Fork | SessionSeedMode::Workspace) {
            self.copy_workspace_files(source.workspace_id, child_workspace_id)
                .await?;
        }
        if seed == SessionSeedMode::Fork {
            self.copy_session_storage(source_session_id, child_session_id)
                .await?;
            if let Some(fork_sequence) = fork_sequence {
                self.db
                    .copy_compaction_checkpoints(source_session_id, child_session_id, fork_sequence)
                    .await?;
            }
        }

        self.db
            .set_session_fork_lineage(child_session_id, source_session_id, fork_sequence)
            .await?;
        child.forked_from_session_id = Some(source_session_id);
        child.forked_from_sequence = fork_sequence;
        Ok(())
    }

    async fn copy_session_events(
        &self,
        source_session_id: SessionId,
        child_session_id: SessionId,
    ) -> Result<Option<i32>> {
        let mut events = self
            .db
            .list_events(source_session_id, None, None, &[], &[], None, None)
            .await?;
        events.sort_by_key(|event| event.sequence);
        let fork_sequence = events.last().map(|event| event.sequence);
        for event in events {
            self.db
                .create_event(CreateEventRow {
                    session_id: child_session_id,
                    event_type: event.event_type,
                    ts: event.ts,
                    context: event.context,
                    data: event.data,
                    metadata: event.metadata,
                    tags: event.tags,
                })
                .await?;
        }
        Ok(fork_sequence)
    }

    async fn copy_workspace_files(
        &self,
        source_workspace_id: Uuid,
        child_workspace_id: Uuid,
    ) -> Result<()> {
        let files = self.db.list_all_session_files(source_workspace_id).await?;
        for file in files {
            if self
                .db
                .get_session_file(child_workspace_id, &file.path)
                .await?
                .is_some()
            {
                continue;
            }
            let content = if file.is_directory {
                None
            } else {
                self.db
                    .get_session_file(source_workspace_id, &file.path)
                    .await?
                    .and_then(|row| row.content)
            };
            self.db
                .create_session_file(CreateSessionFileRow {
                    session_id: SessionId::from_uuid(child_workspace_id),
                    path: file.path,
                    content,
                    is_directory: file.is_directory,
                    is_readonly: file.is_readonly,
                })
                .await?;
        }
        Ok(())
    }

    async fn copy_session_storage(
        &self,
        source_session_id: SessionId,
        child_session_id: SessionId,
    ) -> Result<()> {
        for key in self.db.list_session_keys(source_session_id.uuid()).await? {
            if let Some(row) = self
                .db
                .get_session_key_value(source_session_id.uuid(), &key.key)
                .await?
            {
                self.db
                    .upsert_session_key_value(UpsertSessionKeyValue {
                        session_id: child_session_id,
                        key: row.key,
                        value: row.value,
                    })
                    .await?;
            }
        }
        for secret in self
            .db
            .list_session_secrets(source_session_id.uuid())
            .await?
        {
            if let Some(row) = self
                .db
                .get_session_secret(source_session_id.uuid(), &secret.name)
                .await?
            {
                self.db
                    .upsert_session_secret(UpsertSessionSecret {
                        session_id: child_session_id,
                        name: row.name,
                        value_encrypted: row.value_encrypted,
                    })
                    .await?;
            }
        }
        Ok(())
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
        // How this session was started (EVE-852). Resolved by the calling
        // ingress path, never read from the request body except for the two
        // client-declarable variants the CreateSession command validates.
        source: SessionSource,
        req: CreateSessionRequest,
    ) -> Result<Session> {
        let org_id = caller.org_id;
        let org_public_id = &caller.org_public_id;
        let harness_id = HarnessId::from_uuid(harness_id);
        let agent_id = agent_internal_id.map(AgentId::from_uuid);

        // Enforce the absolute per-org live-session cap in the shared service
        // path so app-channel ingress cannot bypass the public command check.
        let max_sessions = self.resource_limits.max_sessions_per_org;
        let sessions = self.db.count_sessions_for_org(org_id).await?;
        if sessions >= max_sessions {
            return Err(ResourceLimitError::new(format!(
                "Session limit reached (max {max_sessions})"
            ))
            .into());
        }
        if req.seed != SessionSeedMode::Fresh && req.forked_from_session_id.is_none() {
            return Err(BadRequestError::new("seed requires forked_from_session_id").into());
        }

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

        // EVE-AARDVARK: authorize high-risk session capability assignment
        // before full config validation so unauthorized callers cannot force
        // expensive validation for capabilities they are not allowed to use.
        self.require_admin_for_high_risk_session_capabilities(
            caller,
            org_id,
            harness_id.uuid(),
            agent_id.map(|id| id.uuid()),
            &session_capabilities,
        )
        .await?;

        // Validate session-level capability refs before persisting.
        crate::domains::capabilities::validation::validate_capability_refs(
            &self.db,
            org_id,
            &session_capabilities,
        )
        .await?;

        // EVE-709: reject sessions whose harness/agent/session require a built-in
        // capability that is not available in this deployment (e.g. a feature-gated
        // `container_sandbox` when `FEATURE_CONTAINER_SANDBOX` is off). Without this
        // gate the missing capability's tools are silently dropped and the session
        // degrades into a different execution environment (e.g. bash), so the user
        // believes isolated work ran when it did not. Fail clearly instead.
        self.require_available_capabilities(
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
        // knowledge/security/budgeting.md and `extract_app_subjects` in
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

        // Optional attach to an existing shared workspace. When absent, the
        // storage layer auto-creates a default 1:1 workspace (see
        // knowledge/runtime-resources/workspace.md, "Default Workspace per Session").
        let workspace_id = match req.workspace_id {
            Some(public_id) => {
                let workspace = self
                    .db
                    .get_workspace(org_id, public_id)
                    .await?
                    .ok_or_else(|| ResourceNotFoundError::new("Workspace"))?;
                if workspace.status != "active" {
                    return Err(BadRequestError::new(format!(
                        "Workspace {public_id} is {} and cannot accept new sessions",
                        workspace.status
                    ))
                    .into());
                }
                // The session's rendered workspace_id is derived from the
                // internal id, so it only round-trips when id.hex == public_id
                // suffix. New workspaces pin this at creation, but a workspace
                // created before that invariant (random internal PK) would make
                // the session report a workspace_id that 404s against the
                // workspace API. Reject it with actionable guidance rather than
                // silently misrendering.
                if everruns_core::WorkspaceId::from_uuid(workspace.id).to_string()
                    != workspace.public_id
                {
                    return Err(BadRequestError::new(format!(
                        "Workspace {public_id} predates the id/public-id invariant \
                         and cannot be attached; recreate it via POST /v1/workspaces"
                    ))
                    .into());
                }
                Some(workspace.id)
            }
            None => None,
        };

        let requested_goal = req.goal.clone();
        let forked_from_session_id = req.forked_from_session_id;
        let seed = req.seed;

        let input = CreateSessionRow {
            org_id,
            source,
            app_id,
            harness_id: Some(harness_id),
            agent_id,
            agent_version_id: resolved_agent_version.as_ref().map(|version| version.id),
            agent_config_hash: resolved_agent_version
                .as_ref()
                .map(|version| version.config_hash.clone()),
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
                .map(|na| serde_json::to_value(na).unwrap_or_default()),
            max_iterations: max_iterations::to_db(req.max_iterations)?,
            parallel_tool_calls: req.parallel_tool_calls,
            blueprint_id: None,
            blueprint_config: None,
            parent_session_id: req.parent_session_id,
            budget_root_session_id: req.budget_root_session_id,
            workspace_id,
        };
        let row = self.db.create_session(input).await?;
        let row = if requested_goal.is_some() {
            self.db
                .update_session(
                    org_id,
                    row.id,
                    UpdateSession {
                        goal: requested_goal,
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

        let scoped_memory = ScopedMemoryContext {
            agent_id,
            // User memory is private to the resolved user. Do not materialize it
            // into caller-attached shared workspaces because workspace files are
            // currently workspace-wide rather than participant-local.
            user_id: if workspace_id.is_none() {
                resolved_owner_user_id
            } else {
                None
            },
        };

        // Apply capability mounts (harness + agent + session capabilities) and
        // seed initial files into the session's workspace. Key by workspace_id
        // (not session id) so an attached shared workspace receives them; for
        // the default 1:1 session these are equal.
        self.apply_capability_mounts(
            org_id,
            harness_id.uuid(),
            agent_id.map(|a| a.uuid()),
            &session_capabilities,
            session.workspace_id.uuid(),
            Some(scoped_memory),
        )
        .await?;

        self.apply_initial_files(
            org_id,
            harness_id.uuid(),
            agent_id.map(|a| a.uuid()),
            &req.initial_files,
            session.workspace_id.uuid(),
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
        let effective_capabilities =
            crate::domains::capabilities::queries::hydrate_declarative_capability_configs(
                self.db.as_ref(),
                org_id,
                effective_capabilities,
            )
            .await?;

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

        if let Some(source_session_id) = forked_from_session_id {
            self.apply_session_seed(
                org_id,
                source_session_id,
                session.id,
                session.workspace_id.uuid(),
                seed,
                &mut session,
            )
            .await?;
        }

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
        source: SessionSource,
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
        let requested_goal = req.goal.clone();

        let input = CreateSessionRow {
            workspace_id: None,
            org_id,
            source,
            app_id: None,
            harness_id: Some(harness_id),
            agent_id: None,
            agent_version_id: None,
            agent_config_hash: None,
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
                .map(|na| serde_json::to_value(na).unwrap_or_default()),
            max_iterations: max_iterations::to_db(req.max_iterations)?,
            parallel_tool_calls: req.parallel_tool_calls,
            blueprint_id: Some(blueprint_id),
            blueprint_config,
            parent_session_id: None,
            budget_root_session_id: None,
        };
        let mut row = self.db.create_session(input).await?;
        if requested_goal.is_some() {
            row = self
                .db
                .update_session(
                    org_id,
                    row.id,
                    UpdateSession {
                        goal: requested_goal,
                        ..Default::default()
                    },
                )
                .await?
                .unwrap_or(row);
        }
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
        scoped_memory: Option<ScopedMemoryContext>,
    ) -> Result<()> {
        let session_id = session_id.into();

        let mounts = self
            .collect_capability_mounts(
                org_id,
                harness_id,
                agent_id,
                session_capabilities,
                session_id,
                scoped_memory,
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
        scoped_memory: Option<ScopedMemoryContext>,
    ) -> Result<Vec<MountPoint>> {
        let capability_configs = self
            .collect_session_capability_configs(org_id, harness_id, agent_id, session_capabilities)
            .await?;

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
        // `skill:{uuid}` refs have no registry entry, so dependency resolution
        // drops them from `resolved_configs` — resolve them from org data
        // against the raw config list instead (same pattern as declarative and
        // plugin refs, which carry their definition in the config payload).
        mounts.extend(
            self.collect_registry_skill_mounts(org_id, &capability_configs)
                .await?,
        );
        ensure_no_reserved_memory_mounts(&mounts)?;
        if let Some(scoped_memory) = scoped_memory {
            mounts.extend(
                self.collect_scoped_memory_mounts(org_id, scoped_memory)
                    .await?,
            );
        }
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
                None,
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
        let files = self
            .collect_initial_files(org_id, harness_id, agent_id, session_initial_files)
            .await?;
        ensure_no_reserved_memory_initial_files(&files)?;

        for file in files {
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
            .get_model(org_id, model_id.uuid())
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

    /// Resolve the model used by turns without a per-message override.
    ///
    /// Session creation materializes agent and harness defaults into `model_id`,
    /// so only an unbound session continues to follow the organization default.
    pub async fn resolved_model_id(
        &self,
        org_id: i64,
        session: &Session,
    ) -> Result<Option<ModelId>> {
        if let Some(model_id) = session.model_id {
            return Ok(self
                .db
                .get_model(org_id, model_id.uuid())
                .await?
                .map(|model| model.id));
        }

        Ok(self
            .db
            .get_default_model(org_id)
            .await?
            .map(|model| model.id))
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
        user_id: Option<Uuid>,
        filters: &SessionListFilters,
        pagination: Pagination,
    ) -> Result<(Vec<Session>, u32)> {
        let org_id = caller.org_id;
        let org_public_id = &caller.org_public_id;
        let (rows, total) = self.db.list_sessions(org_id, filters, pagination).await?;
        let fallback = if rows.iter().any(|r| r.harness_id.is_none()) {
            Some(org_init::base_harness_id(&self.db, org_id).await?)
        } else {
            None
        };
        let mut sessions: Vec<Session> = rows
            .into_iter()
            .map(|r| Self::row_to_session(r, org_public_id, fallback))
            .collect();

        if sessions.is_empty() {
            return Ok((sessions, total));
        }

        let hydration = self.load_session_list_hydration(org_id, &sessions).await?;
        self.apply_session_list_hydration(&mut sessions, &hydration);

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

    /// Facet-rail counts and masthead metrics for the sessions surface
    /// (EVE-852), aggregated over the same predicate as [`Self::list`].
    ///
    /// Agent buckets are returned keyed by the agent's public id so the caller
    /// never has to expose or resolve internal UUIDs.
    pub async fn facets(
        &self,
        caller: &Caller,
        filters: &SessionListFilters,
    ) -> Result<SessionFacetsResponse> {
        let row = self.db.session_facets(caller.org_id, filters).await?;

        let bucket = |buckets: Vec<crate::storage::SessionFacetBucket>| {
            buckets
                .into_iter()
                .map(|b| SessionFacetCount {
                    value: b.value,
                    count: b.count as u64,
                })
                .collect::<Vec<_>>()
        };

        Ok(SessionFacetsResponse {
            total: row.total as u64,
            by_activity: bucket(row.by_activity),
            by_source: bucket(row.by_source),
            by_agent: row
                .by_agent
                .into_iter()
                .filter_map(|b| {
                    Uuid::parse_str(&b.value).ok().map(|id| SessionFacetCount {
                        value: AgentId::from_uuid(id).to_string(),
                        count: b.count as u64,
                    })
                })
                .collect(),
            active_now: row.active_now as u64,
            failed_today: row.failed_today as u64,
            p95_duration_ms: row.p95_duration_ms as u64,
            tokens_today: row.tokens_today as u64,
        })
    }

    async fn load_session_list_hydration(
        &self,
        org_id: i64,
        sessions: &[Session],
    ) -> Result<SessionListHydration> {
        // THREAT[TM-TENANT-001]: every batch loader receives the caller's org_id;
        // capability-table reads additionally join through their org-scoped owner.
        let principal_ids: Vec<PrincipalId> = sessions
            .iter()
            .map(|session| session.owner_principal_id)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let resolved_user_ids: Vec<Uuid> = sessions
            .iter()
            .filter_map(|session| session.resolved_owner_user_id)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let principal_rows = self
            .db
            .get_principals_for_session_list(org_id, &principal_ids, &resolved_user_ids)
            .await?;

        let mut hydration = SessionListHydration::default();
        for row in principal_rows {
            let principal = row_to_principal(row);
            if principal.status != everruns_platform::PrincipalStatus::Deleted {
                hydration.owners.insert(principal.id, principal.summary());
            }
            if principal.kind == everruns_core::PrincipalKind::User
                && let Some(user_id) = principal.subject_id
            {
                hydration
                    .effective_owners
                    .insert(user_id, principal.summary());
            }
        }

        let agent_ids: Vec<AgentId> = sessions
            .iter()
            .filter_map(|session| session.agent_id)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        if !agent_ids.is_empty() {
            let agent_rows = self.db.get_agents_by_ids(org_id, &agent_ids).await?;
            let existing_agent_ids: Vec<AgentId> = agent_rows.iter().map(|row| row.id).collect();
            for row in agent_rows {
                if let Ok(public_id) = row.public_id.parse::<AgentId>() {
                    hydration.agent_public_ids.insert(row.id, public_id);
                }
            }
            for row in self
                .db
                .get_agent_capabilities_by_agent_ids(org_id, &existing_agent_ids)
                .await?
            {
                let capability_ids = hydration
                    .agent_capability_ids
                    .entry(row.agent_id)
                    .or_default();
                if !capability_ids.contains(&row.capability_id) {
                    capability_ids.push(row.capability_id);
                }
            }
        }

        let harness_ids: HashSet<HarnessId> =
            sessions.iter().map(|session| session.harness_id).collect();
        hydration.harness_capability_ids = self
            .load_session_list_harness_capability_ids(org_id, harness_ids)
            .await?;

        Ok(hydration)
    }

    async fn load_session_list_harness_capability_ids(
        &self,
        org_id: i64,
        root_ids: HashSet<HarnessId>,
    ) -> Result<HashMap<HarnessId, Vec<String>>> {
        let root_id_list: Vec<HarnessId> = root_ids.iter().copied().collect();
        let rows_by_id: HashMap<HarnessId, _> = self
            .db
            .get_harness_ancestry_by_ids(org_id, &root_id_list)
            .await?
            .into_iter()
            .map(|row| (row.id, row))
            .collect();

        let loaded_ids: Vec<HarnessId> = rows_by_id.keys().copied().collect();
        let mut layer_capability_ids: HashMap<HarnessId, Vec<String>> = HashMap::new();
        for row in self
            .db
            .get_harness_capabilities_by_harness_ids(org_id, &loaded_ids)
            .await?
        {
            layer_capability_ids
                .entry(row.harness_id)
                .or_default()
                .push(row.capability_id);
        }

        let mut effective_by_root = HashMap::new();
        for root_id in root_ids {
            if !rows_by_id.contains_key(&root_id) {
                continue;
            }
            let mut chain = Vec::new();
            let mut visited = HashSet::new();
            let mut cursor = Some(root_id);
            while let Some(id) = cursor {
                if !visited.insert(id) {
                    anyhow::bail!("Harness inheritance cycle detected");
                }
                let row = rows_by_id
                    .get(&id)
                    .ok_or_else(|| ResourceNotFoundError::new("Parent harness"))?;
                chain.push(id);
                cursor = row.parent_harness_id;
            }

            let mut capability_ids = Vec::new();
            for id in chain.into_iter().rev() {
                for capability_id in layer_capability_ids.get(&id).into_iter().flatten() {
                    if !capability_ids.contains(capability_id) {
                        capability_ids.push(capability_id.clone());
                    }
                }
            }
            effective_by_root.insert(root_id, capability_ids);
        }

        Ok(effective_by_root)
    }

    fn apply_session_list_hydration(
        &self,
        sessions: &mut [Session],
        hydration: &SessionListHydration,
    ) {
        for session in sessions {
            session.owner = hydration.owners.get(&session.owner_principal_id).cloned();
            session.effective_owner = session
                .resolved_owner_user_id
                .and_then(|id| hydration.effective_owners.get(&id).cloned());

            let agent_internal_id = session.agent_id;
            let mut capability_ids = hydration
                .harness_capability_ids
                .get(&session.harness_id)
                .cloned()
                .unwrap_or_default();
            if let Some(agent_id) = agent_internal_id
                && let Some(agent_capability_ids) = hydration.agent_capability_ids.get(&agent_id)
            {
                for capability_id in agent_capability_ids {
                    if !capability_ids.contains(capability_id) {
                        capability_ids.push(capability_id.clone());
                    }
                }
            }
            for capability in &session.capabilities {
                let capability_id = capability.capability_id().to_string();
                if !capability_ids.contains(&capability_id) {
                    capability_ids.push(capability_id);
                }
            }
            session.features = compute_features(&capability_ids, &self.capability_registry);

            if let Some(agent_id) = agent_internal_id
                && let Some(public_id) = hydration.agent_public_ids.get(&agent_id)
            {
                session.agent_id = Some(*public_id);
            }
        }
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
            goal: req.goal,
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
        org_rate_limiter: Option<&OrgRateLimiter>,
    ) -> std::result::Result<Session, GetOrCreateChatSessionError> {
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

        // THREAT[TM-DOS-016]: Only a cache miss creates a resource. Charge the
        // shared per-org session bucket here so transport-independent callers
        // are covered without throttling ordinary global-chat reuse.
        if let Some(limiter) = org_rate_limiter
            && limiter.check_session_create(org_id).await.is_err()
        {
            return Err(GetOrCreateChatSessionError::RateLimited);
        }

        // Create a new chat session
        let source = SessionSource::Chat;
        let harness_id_typed = HarnessId::from_uuid(harness_id);
        let input = CreateSessionRow {
            workspace_id: None,
            org_id,
            source,
            app_id: None,
            harness_id: Some(harness_id_typed),
            agent_id: None,
            agent_version_id: None,
            agent_config_hash: None,
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
            parallel_tool_calls: None,
            blueprint_id: None,
            blueprint_config: None,
            network_access: None,
            parent_session_id: None,
            budget_root_session_id: None,
        };
        let row = self.db.create_session(input).await?;
        let session_id = row.id.uuid();
        let mut session = Self::row_to_session(row, org_public_id, Some(harness_id_typed));
        self.hydrate_ownership(org_id, &mut session).await?;
        self.populate_features(org_id, &mut session).await?;

        // Apply capability mounts
        self.apply_capability_mounts(
            org_id,
            harness_id,
            None,
            &[],
            session_id,
            Some(ScopedMemoryContext {
                agent_id: None,
                user_id: Some(user_id),
            }),
        )
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
        let merged = merge_capabilities(&merged, session_caps);
        match crate::domains::capabilities::queries::hydrate_declarative_capability_configs(
            self.db.as_ref(),
            org_id,
            merged,
        )
        .await
        {
            Ok(capabilities) => capabilities,
            Err(error) => {
                tracing::warn!(%error, "failed to hydrate session capabilities");
                Vec::new()
            }
        }
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
            let specs = capability.user_hooks_with_config(config.config_value());
            if !specs.is_empty() {
                contributions.push((config.capability_id().to_string(), specs));
            }
            if config.capability_id() == "user_hooks" {
                disabled.extend(
                    everruns_core::capabilities::user_hooks::disabled_contributions(
                        config.config_value(),
                    ),
                );
            }
        }
        let specs = everruns_core::hook_adapter::finalize_hook_specs(contributions, &disabled);
        let file_store: Arc<dyn everruns_core::traits::SessionFileSystem> = Arc::new(
            crate::domains::session_files::WorkspaceFileService::new(self.db.clone()),
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

        let capability_configs = merge_capabilities(&capability_configs, session_capabilities);
        crate::domains::capabilities::queries::hydrate_declarative_capability_configs(
            self.db.as_ref(),
            org_id,
            capability_configs,
        )
        .await
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
        let memory_config: MemoryConfig = serde_json::from_value(config.config_value().clone())
            .map_err(|error| {
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
            if memory.scope != "org" {
                return Err(BadRequestError::new(
                    "Scoped memories are server-managed and cannot be mounted explicitly",
                )
                .into());
            }

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

    /// Mount registry skills referenced as `skill:{uuid}` capability refs.
    ///
    /// Each active skill is reconstructed into `/.agents/skills/{name}/`
    /// (SKILL.md + bundled text files) so the built-in `SkillsCapability`
    /// discovers it alongside workspace skills.
    ///
    /// A skill that is missing or not active is skipped with a warning rather
    /// than failing session creation: `validate_capability_refs` accepts a
    /// `skill:{uuid}` ref at any status, so archiving or disabling a skill must
    /// not take down every agent that references it. A session missing one
    /// skill is still runnable — unlike a missing memory mount, which is fatal.
    async fn collect_registry_skill_mounts(
        &self,
        org_id: i64,
        capability_configs: &[AgentCapabilityConfig],
    ) -> Result<Vec<MountPoint>> {
        let mut seen = HashSet::new();
        let mut mounts = Vec::new();
        for config in capability_configs {
            let cap_id = config.capability_id();
            if !is_skill_capability(cap_id) {
                continue;
            }
            let skill_uuid = parse_skill_capability_id(cap_id).ok_or_else(|| {
                BadRequestError::new(format!("Invalid skill capability reference: {cap_id}"))
            })?;
            if !seen.insert(skill_uuid) {
                continue;
            }
            let Some(row) = self.db.get_skill(org_id, skill_uuid).await? else {
                tracing::warn!(
                    skill_id = %skill_uuid,
                    "Referenced skill not found; skipping session mount"
                );
                continue;
            };
            if row.status != "active" {
                tracing::warn!(
                    skill_id = %skill_uuid,
                    status = %row.status,
                    "Referenced skill is not active; skipping session mount"
                );
                continue;
            }
            let skill = crate::domains::skills::queries::row_to_skill(&row);

            // Bundled files: text only. SKILL.md is reconstructed from the
            // stored fields, so drop any archived copy to keep it canonical.
            let files: Vec<(String, String)> = self
                .db
                .list_skill_files(skill_uuid)
                .await?
                .into_iter()
                .filter(|file| file.path != "SKILL.md")
                .filter_map(|file| {
                    if file.is_binary {
                        tracing::warn!(
                            skill = %skill.name,
                            path = %file.path,
                            "Skipping binary skill file in session mount"
                        );
                        return None;
                    }
                    file.content.map(|content| (file.path, content))
                })
                .collect();

            let capability = AttachSkillCapability::from_registry_with_options(
                skill_uuid,
                skill.name,
                skill.description,
                row.instructions.clone(),
                files,
                skill.user_invocable,
                skill.disable_model_invocation,
            );
            mounts.extend(everruns_core::capabilities::Capability::mounts(&capability));
        }
        Ok(mounts)
    }

    async fn collect_scoped_memory_mounts(
        &self,
        org_id: i64,
        context: ScopedMemoryContext,
    ) -> Result<Vec<MountPoint>> {
        let mut mounts = Vec::with_capacity(2);

        if let Some(agent_id) = context.agent_id {
            let memory = self
                .get_or_create_scoped_memory(
                    org_id,
                    "agent",
                    Some(agent_id),
                    None,
                    format!("agent-memory-{}", agent_id.uuid().simple()),
                    "Server-managed per-agent memory.",
                )
                .await?;
            mounts.push(
                self.memory_row_to_mount(memory, AGENT_MEMORY_MOUNT_PATH)
                    .await?,
            );
        }

        if let Some(user_id) = context.user_id {
            let memory = self
                .get_or_create_scoped_memory(
                    org_id,
                    "user",
                    None,
                    Some(user_id),
                    format!("user-memory-{}", user_id.simple()),
                    "Server-managed per-user memory.",
                )
                .await?;
            mounts.push(
                self.memory_row_to_mount(memory, USER_MEMORY_MOUNT_PATH)
                    .await?,
            );
        }

        Ok(mounts)
    }

    async fn get_or_create_scoped_memory(
        &self,
        org_id: i64,
        scope: &str,
        owner_agent_id: Option<AgentId>,
        owner_user_id: Option<Uuid>,
        name: String,
        description: &str,
    ) -> Result<MemoryRow> {
        if let Some(memory) = self
            .db
            .get_memory_by_scope_owner(org_id, scope, owner_agent_id, owner_user_id)
            .await?
            .filter(|memory| memory.status == "active")
        {
            return Ok(memory);
        }

        self.db
            .create_memory(
                org_id,
                CreateMemoryRow {
                    public_id: MemoryId::new().to_string(),
                    name,
                    description: Some(description.to_string()),
                    scope: scope.to_string(),
                    owner_agent_id,
                    owner_user_id,
                    source_type: "manual".to_string(),
                    source_config: serde_json::json!({}),
                    is_readonly: false,
                    sync_status: "idle".to_string(),
                    owner_principal_id: None,
                    resolved_owner_user_id: owner_user_id,
                },
            )
            .await
    }

    async fn memory_row_to_mount(&self, memory: MemoryRow, mount_path: &str) -> Result<MountPoint> {
        let files = self.db.list_all_memory_files(memory.id).await?;
        Ok(MountPoint::new(
            mount_path,
            MountAccess::ReadWrite,
            MountSource::directory(memory_files_to_mount_entries(files)),
            MEMORY_CAPABILITY_ID,
        ))
    }

    /// EVE-709: reject session creation when a required built-in capability is not
    /// available in this deployment.
    ///
    /// The effective capability set (harness chain + agent + session) may name
    /// built-in capabilities that are feature-gated (e.g. `container_sandbox`
    /// behind `FEATURE_CONTAINER_SANDBOX`). When such a capability is disabled it
    /// is absent from the registry, its tools never register, and the session
    /// silently runs without them — degrading into a different execution
    /// environment. Rather than degrade silently, fail with a clear error naming
    /// the unavailable capabilities.
    ///
    /// Only plain built-in references are checked. Namespaced refs
    /// (`declarative:`, `plugin:`, `skill:`, `mcp:`) resolve from org data rather
    /// than the registry, so their absence from the registry is expected and is
    /// validated separately by `validate_capability_refs`.
    async fn require_available_capabilities(
        &self,
        org_id: i64,
        harness_id: Uuid,
        agent_id: Option<Uuid>,
        session_capabilities: &[AgentCapabilityConfig],
    ) -> Result<()> {
        let capability_ids = self
            .collect_session_capability_ids(org_id, harness_id, agent_id, session_capabilities)
            .await?;

        let mut missing: Vec<String> = capability_ids
            .into_iter()
            .filter(|id| {
                !is_declarative_capability(id)
                    && !is_plugin_capability(id)
                    && !is_skill_capability(id)
                    && !is_mcp_capability(id)
                    && !self.capability_registry.has(id)
            })
            .collect();

        if missing.is_empty() {
            return Ok(());
        }

        missing.sort();
        missing.dedup();
        Err(BadRequestError::new(format!(
            "Harness requires capabilities unavailable in this deployment: {}",
            missing.join(", ")
        ))
        .into())
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
                )
                .with_effective_cost((row.total_cost_usd > 0.0).then_some(row.total_cost_usd)),
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
            goal: row.goal,
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
            parallel_tool_calls: row.parallel_tool_calls,
            status: SessionStatus::from(row.status.as_str()),
            source: SessionSource::from(row.source.as_str()),
            activity: SessionActivity::derive(
                &SessionStatus::from(row.status.as_str()),
                row.last_turn_status.as_deref(),
            ),
            created_at: row.created_at,
            updated_at: row.updated_at,
            started_at: row.started_at,
            finished_at: row.finished_at,
            usage,
            is_pinned: None,             // Populated by caller with user context
            active_schedule_count: None, // Populated by caller
            features: vec![],            // Populated by caller via populate_features()
            parent_session_id: row.parent_session_id,
            forked_from_session_id: row.forked_from_session_id,
            forked_from_sequence: row.forked_from_sequence,
            blueprint_id: row.blueprint_id,
            blueprint_config: row.blueprint_config,
        }
    }

    async fn resolve_effective_harness(
        &self,
        org_id: i64,
        harness_id: HarnessId,
    ) -> Result<Option<everruns_platform::Harness>> {
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
                    .config_mut()
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

fn ensure_no_reserved_memory_mounts(mounts: &[MountPoint]) -> Result<()> {
    for mount in mounts {
        if let Some(reserved) = reserved_memory_path(&mount.path) {
            return Err(BadRequestError::new(format!(
                "Mount path {} is reserved for server-managed memory ({reserved})",
                mount.path
            ))
            .into());
        }
    }
    Ok(())
}

fn ensure_no_reserved_memory_initial_files(files: &[InitialFile]) -> Result<()> {
    for file in files {
        if let Some(reserved) = reserved_memory_path(&file.path) {
            return Err(BadRequestError::new(format!(
                "Initial file path {} is reserved for server-managed memory ({reserved})",
                file.path
            ))
            .into());
        }
    }
    Ok(())
}

fn reserved_memory_path(path: &str) -> Option<&'static str> {
    let path = normalize_initial_file_path(path);
    if path == "/memory" || path.starts_with("/memory/") {
        Some("/memory/*")
    } else {
        None
    }
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
        CreateHarnessRow, CreateMemoryFileRow, CreateModelRow, CreateOrganizationRow,
        CreateProviderRow, StorageBackend, UpdateAgent,
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
            .config_value()
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

    async fn test_ctx(caller: Caller, db: Arc<StorageBackend>) -> Ctx {
        crate::org_init::initialize_org_harnesses(&db, caller.org_id)
            .await
            .expect("initialize built-in harnesses for session service tests");
        let capability_service = Arc::new(CapabilityService::new(db.clone(), None));
        Ctx::new(
            caller,
            db,
            capability_service,
            None,
            Arc::new(everruns_core::DefaultPermissionResolver),
        )
        .with_feature_flags(crate::domains::common::all_feature_flags_for_test())
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
            source: None,
            workspace_id: None,
            harness_id: Some(harness_id),
            harness_name: None,
            agent_id,
            agent_name: None,
            agent_identity_id: None,
            title: Some("Test Session".to_string()),
            goal: None,
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
            parallel_tool_calls: None,
            parent_session_id: None,
            forked_from_session_id: None,
            budget_root_session_id: None,
            seed: SessionSeedMode::Fresh,
        }
    }

    #[tokio::test]
    async fn session_list_lookup_count_is_independent_of_page_size() {
        let db = Arc::new(StorageBackend::in_memory());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let ctx = test_ctx(caller.clone(), db.clone()).await;

        let parent = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "list-parent-harness".to_string(),
            display_name: Some("List Parent Harness".to_string()),
            description: None,
            system_prompt: Some("parent".to_string()),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            embedder_metadata: Default::default(),
        })
        .execute(&ctx)
        .await
        .unwrap();
        let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "list-child-harness".to_string(),
            display_name: Some("List Child Harness".to_string()),
            description: None,
            system_prompt: Some("child".to_string()),
            parent_harness_id: Some(parent.id),
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            embedder_metadata: Default::default(),
        })
        .execute(&ctx)
        .await
        .unwrap();
        let agent = crate::domains::agents::CreateAgent(CreateAgentRequest {
            id: None,
            name: "list-agent".to_string(),
            display_name: Some("List Agent".to_string()),
            description: None,
            system_prompt: "agent".to_string(),
            default_model_id: None,
            harness_id: None,
            harness_name: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            tools: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
        })
        .execute(&ctx)
        .await
        .unwrap();
        let service = SessionService::new(db.clone());

        for index in 0..20 {
            let mut request = build_create_request(harness.id, Some(agent.public_id), None);
            request.title = Some(format!("List session {index}"));
            service
                .create(
                    &caller,
                    harness.id.uuid(),
                    Some(agent.internal_id),
                    Some(agent.public_id),
                    SessionSource::Api,
                    request,
                )
                .await
                .unwrap();
        }

        db.reset_session_list_lookup_count();
        let (one, _) = service
            .list(
                &caller,
                Some(everruns_platform::ANONYMOUS_USER_ID),
                &SessionListFilters::default(),
                Pagination {
                    limit: 1,
                    offset: 0,
                },
            )
            .await
            .unwrap();
        let one_lookup_count = db.session_list_lookup_count();
        assert_eq!(one.len(), 1);

        db.reset_session_list_lookup_count();
        let (twenty, _) = service
            .list(
                &caller,
                Some(everruns_platform::ANONYMOUS_USER_ID),
                &SessionListFilters::default(),
                Pagination {
                    limit: 20,
                    offset: 0,
                },
            )
            .await
            .unwrap();
        let twenty_lookup_count = db.session_list_lookup_count();
        assert_eq!(twenty.len(), 20);

        assert_eq!(
            twenty_lookup_count, one_lookup_count,
            "session-list storage lookups must stay bounded as page size grows"
        );
        assert_eq!(
            twenty_lookup_count, 9,
            "session-list hydration should use the fixed batch-query budget"
        );

        db.set_session_list_lookup_delay_ms(2);
        db.reset_session_list_lookup_count();
        let legacy_started = tokio::time::Instant::now();
        let (legacy_rows, _) = db
            .list_sessions(
                DEFAULT_ORG_ID,
                &SessionListFilters::default(),
                Pagination {
                    limit: 20,
                    offset: 0,
                },
            )
            .await
            .unwrap();
        let mut legacy_sessions: Vec<Session> = legacy_rows
            .into_iter()
            .map(|row| SessionService::row_to_session(row, &caller.org_public_id, None))
            .collect();
        for session in &mut legacy_sessions {
            service
                .hydrate_ownership(DEFAULT_ORG_ID, session)
                .await
                .unwrap();
        }
        for session in &mut legacy_sessions {
            service
                .resolve_effective_harness(DEFAULT_ORG_ID, session.harness_id)
                .await
                .unwrap();
            service
                .populate_features(DEFAULT_ORG_ID, session)
                .await
                .unwrap();
        }
        for session in &mut legacy_sessions {
            service
                .resolve_session_agent_id(DEFAULT_ORG_ID, session)
                .await
                .unwrap();
        }
        let legacy_ids: Vec<Uuid> = legacy_sessions
            .iter()
            .map(|session| session.id.uuid())
            .collect();
        db.get_session_previews(&legacy_ids).await.unwrap();
        db.get_session_output_previews(&legacy_ids).await.unwrap();
        db.list_pinned_session_ids(everruns_platform::ANONYMOUS_USER_ID, DEFAULT_ORG_ID)
            .await
            .unwrap();
        let legacy_elapsed = legacy_started.elapsed();
        let legacy_lookup_count = db.session_list_lookup_count();

        db.reset_session_list_lookup_count();
        let batched_started = tokio::time::Instant::now();
        service
            .list(
                &caller,
                Some(everruns_platform::ANONYMOUS_USER_ID),
                &SessionListFilters::default(),
                Pagination {
                    limit: 20,
                    offset: 0,
                },
            )
            .await
            .unwrap();
        let batched_elapsed = batched_started.elapsed();
        let batched_lookup_count = db.session_list_lookup_count();
        db.set_session_list_lookup_delay_ms(0);

        eprintln!(
            "sessions-list benchmark (20 rows, 2ms simulated DB latency): before={legacy_lookup_count} lookups/{legacy_elapsed:?}, after={batched_lookup_count} lookups/{batched_elapsed:?}"
        );
        assert_eq!(legacy_lookup_count, 244);
        assert_eq!(batched_lookup_count, 9);
        assert!(
            batched_elapsed * 5 < legacy_elapsed,
            "batched hydration should be materially faster under cross-cloud latency"
        );
    }

    #[tokio::test]
    async fn session_list_batch_hydration_preserves_response_fields() {
        let db = Arc::new(StorageBackend::in_memory());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let ctx = test_ctx(caller.clone(), db.clone()).await;

        let parent = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "hydration-parent".to_string(),
            display_name: Some("Hydration Parent".to_string()),
            description: None,
            system_prompt: Some("parent".to_string()),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![AgentCapabilityConfig::new("session_file_system")],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            embedder_metadata: Default::default(),
        })
        .execute(&ctx)
        .await
        .unwrap();
        let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "hydration-child".to_string(),
            display_name: Some("Hydration Child".to_string()),
            description: None,
            system_prompt: Some("child".to_string()),
            parent_harness_id: Some(parent.id),
            default_model_id: None,
            tags: vec![],
            capabilities: vec![AgentCapabilityConfig::new("session_tasks")],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            embedder_metadata: Default::default(),
        })
        .execute(&ctx)
        .await
        .unwrap();
        let agent = crate::domains::agents::CreateAgent(CreateAgentRequest {
            id: None,
            name: "hydration-agent".to_string(),
            display_name: Some("Hydration Agent".to_string()),
            description: None,
            system_prompt: "agent".to_string(),
            default_model_id: None,
            harness_id: None,
            harness_name: None,
            tags: vec![],
            capabilities: vec![AgentCapabilityConfig::new("session_schedule")],
            initial_files: vec![],
            tools: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
        })
        .execute(&ctx)
        .await
        .unwrap();
        let service = SessionService::new(db.clone());

        let mut agent_request = build_create_request(harness.id, Some(agent.public_id), None);
        agent_request.title = Some("agent session".to_string());
        agent_request.capabilities = vec![AgentCapabilityConfig::new("session_storage")];
        let agent_session = service
            .create(
                &caller,
                harness.id.uuid(),
                Some(agent.internal_id),
                Some(agent.public_id),
                SessionSource::Api,
                agent_request,
            )
            .await
            .unwrap();
        let no_agent_session = service
            .create(
                &caller,
                harness.id.uuid(),
                None,
                None,
                SessionSource::Api,
                build_create_request(harness.id, None, None),
            )
            .await
            .unwrap();

        for (event_type, text) in [
            ("input.message", "input preview"),
            ("output.message.completed", "output preview"),
        ] {
            db.create_event(CreateEventRow {
                session_id: agent_session.id,
                event_type: event_type.to_string(),
                ts: chrono::Utc::now(),
                context: serde_json::json!({}),
                data: serde_json::json!({
                    "message": {"content": [{"type": "text", "text": text}]}
                }),
                metadata: None,
                tags: None,
            })
            .await
            .unwrap();
        }
        db.pin_session(
            everruns_platform::ANONYMOUS_USER_ID,
            agent_session.id,
            DEFAULT_ORG_ID,
        )
        .await
        .unwrap();

        db.update_agent(
            DEFAULT_ORG_ID,
            AgentId::from_uuid(agent.internal_id),
            UpdateAgent {
                status: Some("deleted".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let missing_agent_id = AgentId::new();
        let missing_owner_id = PrincipalId::new();
        let missing_reference_session = db
            .create_session(CreateSessionRow {
                source: everruns_platform::SessionSource::Api,
                workspace_id: None,
                org_id: DEFAULT_ORG_ID,
                app_id: None,
                harness_id: Some(harness.id),
                agent_id: Some(missing_agent_id),
                agent_version_id: None,
                agent_config_hash: None,
                agent_identity_id: None,
                owner_principal_id: missing_owner_id,
                resolved_owner_user_id: None,
                title: Some("missing references".to_string()),
                locale: None,
                tags: vec![],
                model_id: None,
                capabilities: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                system_prompt: None,
                initial_files: serde_json::json!([]),
                hints: None,
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
                blueprint_id: None,
                blueprint_config: None,
                parent_session_id: None,
                budget_root_session_id: None,
            })
            .await
            .unwrap();

        let (sessions, total) = service
            .list(
                &caller,
                Some(everruns_platform::ANONYMOUS_USER_ID),
                &SessionListFilters::default(),
                Pagination {
                    limit: 20,
                    offset: 0,
                },
            )
            .await
            .unwrap();
        assert_eq!(total, 3);
        assert_eq!(sessions.len(), 3);
        assert!(
            sessions
                .windows(2)
                .all(|pair| pair[0].created_at >= pair[1].created_at)
        );

        let listed_agent = sessions
            .iter()
            .find(|session| session.id == agent_session.id)
            .unwrap();
        assert_eq!(listed_agent.agent_id, Some(agent.public_id));
        assert!(listed_agent.owner.is_some());
        assert_eq!(listed_agent.preview.as_deref(), Some("input preview"));
        assert_eq!(
            listed_agent.output_preview.as_deref(),
            Some("output preview")
        );
        assert_eq!(listed_agent.is_pinned, Some(true));
        for feature in ["file_system", "session_tasks", "schedules", "key_value"] {
            assert!(
                listed_agent.features.iter().any(|value| value == feature),
                "missing feature {feature}: {:?}",
                listed_agent.features
            );
        }

        let listed_no_agent = sessions
            .iter()
            .find(|session| session.id == no_agent_session.id)
            .unwrap();
        assert_eq!(listed_no_agent.agent_id, None);
        assert_eq!(listed_no_agent.is_pinned, Some(false));
        assert!(
            listed_no_agent
                .features
                .iter()
                .any(|value| value == "file_system")
        );
        assert!(
            listed_no_agent
                .features
                .iter()
                .any(|value| value == "session_tasks")
        );

        let listed_missing = sessions
            .iter()
            .find(|session| session.id == missing_reference_session.id)
            .unwrap();
        assert_eq!(listed_missing.agent_id, Some(missing_agent_id));
        assert!(listed_missing.owner.is_none());

        let (empty_page, empty_total) = service
            .list(
                &caller,
                Some(everruns_platform::ANONYMOUS_USER_ID),
                &SessionListFilters::default(),
                Pagination {
                    limit: 20,
                    offset: 100,
                },
            )
            .await
            .unwrap();
        assert!(empty_page.is_empty());
        assert_eq!(empty_total, 3);
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
            .create_provider(
                org_id,
                CreateProviderRow {
                    name: format!("Provider {org_id}"),
                    provider_type: "openai".to_string(),
                    base_url: None,
                    api_key_encrypted: None,
                    settings: None,
                },
            )
            .await
            .unwrap();

        db.create_model(
            org_id,
            CreateModelRow {
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
    async fn resolved_model_id_tracks_default_and_preserves_explicit_binding() {
        let db = Arc::new(StorageBackend::in_memory());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let _ctx = test_ctx(caller.clone(), db.clone()).await;
        let service = SessionService::new(db.clone());
        let harness_id = org_init::base_harness_id(&db, caller.org_id).await.unwrap();

        let inherited = service
            .create(
                &caller,
                harness_id.uuid(),
                None,
                None,
                SessionSource::Api,
                build_create_request(harness_id, None, None),
            )
            .await
            .unwrap();
        assert_eq!(
            service
                .resolved_model_id(caller.org_id, &inherited)
                .await
                .unwrap(),
            None
        );

        let first_default = create_model(&db, caller.org_id, "first-default").await;
        db.upsert_organization_settings(caller.org_id, Some(first_default.uuid()))
            .await
            .unwrap();
        assert_eq!(
            service
                .resolved_model_id(caller.org_id, &inherited)
                .await
                .unwrap(),
            Some(first_default)
        );

        let second_default = create_model(&db, caller.org_id, "second-default").await;
        db.upsert_organization_settings(caller.org_id, Some(second_default.uuid()))
            .await
            .unwrap();
        assert_eq!(
            service
                .resolved_model_id(caller.org_id, &inherited)
                .await
                .unwrap(),
            Some(second_default)
        );

        let explicit = service
            .create(
                &caller,
                harness_id.uuid(),
                None,
                None,
                SessionSource::Api,
                build_create_request(harness_id, None, Some(first_default)),
            )
            .await
            .unwrap();
        assert_eq!(
            service
                .resolved_model_id(caller.org_id, &explicit)
                .await
                .unwrap(),
            Some(first_default)
        );
    }

    #[tokio::test]
    async fn app_backreference_is_only_set_by_app_session_create() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_service = SessionService::new(db.clone());
        let caller = Caller::internal(1);
        let ctx = test_ctx(caller.clone(), db.clone()).await;

        let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "app-backref-harness".to_string(),
            display_name: Some("App Backref Harness".to_string()),
            description: None,
            system_prompt: Some("Harness prompt".to_string()),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            embedder_metadata: Default::default(),
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
                SessionSource::Api,
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
                SessionSource::Api,
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
    async fn fork_copies_history_files_and_records_lineage() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_service = SessionService::new(db.clone());
        let caller = Caller::internal(1);
        let ctx = test_ctx(caller.clone(), db.clone()).await;

        let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "fork-harness".to_string(),
            display_name: Some("Fork Harness".to_string()),
            description: None,
            system_prompt: Some("Harness prompt".to_string()),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            embedder_metadata: Default::default(),
        })
        .execute(&ctx)
        .await
        .unwrap();

        let parent = session_service
            .create(
                &caller,
                harness.id.uuid(),
                None,
                None,
                SessionSource::Api,
                build_create_request(harness.id, None, None),
            )
            .await
            .unwrap();

        // Seed the parent with conversation history and a workspace file.
        for (etype, text) in [
            ("input.message", "hello"),
            ("output.message.completed", "hi there"),
        ] {
            db.create_event(CreateEventRow {
                session_id: parent.id,
                event_type: etype.to_string(),
                ts: chrono::Utc::now(),
                context: serde_json::json!({}),
                data: serde_json::json!({ "text": text }),
                metadata: None,
                tags: None,
            })
            .await
            .unwrap();
        }
        db.create_session_file(CreateSessionFileRow {
            session_id: SessionId::from_uuid(parent.workspace_id.uuid()),
            path: "/notes.txt".to_string(),
            content: Some(b"fork me".to_vec()),
            is_directory: false,
            is_readonly: false,
        })
        .await
        .unwrap();
        db.upsert_session_key_value(UpsertSessionKeyValue {
            session_id: parent.id,
            key: "state".to_string(),
            value: "ready".to_string(),
        })
        .await
        .unwrap();
        db.upsert_session_secret(UpsertSessionSecret {
            session_id: parent.id,
            name: "API_TOKEN".to_string(),
            value_encrypted: b"ciphertext".to_vec(),
        })
        .await
        .unwrap();

        let parent_events = db
            .list_events(parent.id, None, None, &[], &[], None, None)
            .await
            .unwrap();

        let child = session_service
            .fork(
                &caller,
                parent.id,
                ForkOverrides {
                    title: Some("Branched".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // Independent identity + recorded lineage.
        assert_ne!(child.id, parent.id);
        assert_ne!(child.workspace_id.uuid(), parent.workspace_id.uuid());
        assert_eq!(child.forked_from_session_id, Some(parent.id));
        assert_eq!(
            child.forked_from_sequence,
            parent_events.iter().map(|e| e.sequence).max()
        );
        assert_eq!(child.title.as_deref(), Some("Branched"));

        // History copied verbatim (same count and per-type ordering).
        let child_events = db
            .list_events(child.id, None, None, &[], &[], None, None)
            .await
            .unwrap();
        assert_eq!(child_events.len(), parent_events.len());
        let parent_types: Vec<_> = parent_events.iter().map(|e| e.event_type.clone()).collect();
        let child_types: Vec<_> = child_events.iter().map(|e| e.event_type.clone()).collect();
        assert_eq!(child_types, parent_types);

        // Workspace file copied into the child's isolated workspace.
        let copied = db
            .get_session_file(child.workspace_id.uuid(), "/notes.txt")
            .await
            .unwrap()
            .expect("forked workspace should contain the parent's file");
        assert_eq!(copied.content.as_deref(), Some(b"fork me".as_slice()));

        let copied_kv = db
            .get_session_key_value(child.id.uuid(), "state")
            .await
            .unwrap()
            .expect("forked session should contain KV");
        assert_eq!(copied_kv.value, "ready");
        let copied_secret = db
            .get_session_secret(child.id.uuid(), "API_TOKEN")
            .await
            .unwrap()
            .expect("forked session should contain secret");
        assert_eq!(copied_secret.value_encrypted, b"ciphertext");

        // The parent is untouched.
        let parent_after = db.get_session(1, parent.id).await.unwrap().unwrap();
        assert_eq!(parent_after.forked_from_session_id, None);
    }

    #[tokio::test]
    async fn starter_files_are_copied_into_new_sessions() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_service = SessionService::new(db.clone());
        let caller = Caller::internal(1);
        let ctx = test_ctx(caller.clone(), db.clone()).await;

        let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "harness".to_string(),
            display_name: Some("Harness".to_string()),
            description: None,
            system_prompt: Some("Harness prompt".to_string()),
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
            embedder_metadata: Default::default(),
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
            harness_id: None,
            harness_name: None,
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
            parallel_tool_calls: None,
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
                SessionSource::Api,
                build_create_request(harness.id, Some(agent.public_id), None),
            )
            .await
            .unwrap();

        let file_service = WorkspaceFileService::new(db);
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
    async fn scoped_memories_are_auto_created_and_mounted_for_new_sessions() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_service = SessionService::new(db.clone());
        let user = db
            .create_user(crate::storage::CreateUserRow {
                external_id: None,
                email: "memory-owner@example.com".to_string(),
                name: "Memory Owner".to_string(),
                avatar_url: None,
                roles: vec![],
                password_hash: None,
                email_verified: true,
                auth_provider: None,
                auth_provider_id: None,
            })
            .await
            .unwrap();
        let caller = Caller {
            user_id: Some(user.id),
            ..external_caller(DEFAULT_ORG_ID)
        };
        let ctx = test_ctx(caller.clone(), db.clone()).await;

        let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "scoped-memory-harness".to_string(),
            display_name: Some("Scoped Memory Harness".to_string()),
            description: None,
            system_prompt: Some("Harness prompt".to_string()),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            embedder_metadata: Default::default(),
        })
        .execute(&ctx)
        .await
        .unwrap();

        let agent = crate::domains::agents::CreateAgent(CreateAgentRequest {
            id: None,
            name: "scoped-memory-agent".to_string(),
            display_name: Some("Scoped Memory Agent".to_string()),
            description: None,
            system_prompt: "Agent prompt".to_string(),
            default_model_id: None,
            harness_id: None,
            harness_name: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            tools: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
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
                SessionSource::Api,
                build_create_request(harness.id, Some(agent.public_id), None),
            )
            .await
            .unwrap();

        let file_service = WorkspaceFileService::new(db.clone());
        let agent_mount = file_service
            .stat(session.id.uuid(), AGENT_MEMORY_MOUNT_PATH)
            .await
            .unwrap()
            .expect("agent memory mount exists");
        assert!(agent_mount.is_directory);
        assert!(!agent_mount.is_readonly);

        let user_mount = file_service
            .stat(session.id.uuid(), USER_MEMORY_MOUNT_PATH)
            .await
            .unwrap()
            .expect("user memory mount exists");
        assert!(user_mount.is_directory);
        assert!(!user_mount.is_readonly);

        assert!(
            db.get_memory_by_scope_owner(
                DEFAULT_ORG_ID,
                "agent",
                Some(AgentId::from_uuid(agent.internal_id)),
                None,
            )
            .await
            .unwrap()
            .is_some()
        );
        assert!(
            db.get_memory_by_scope_owner(DEFAULT_ORG_ID, "user", None, Some(user.id))
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            db.list_memories(DEFAULT_ORG_ID, None, false)
                .await
                .unwrap()
                .is_empty(),
            "scoped memories stay hidden from org memory listing"
        );
    }

    #[tokio::test]
    async fn session_initial_files_cannot_claim_reserved_memory_paths() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_service = SessionService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let ctx = test_ctx(caller.clone(), db.clone()).await;

        let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "reserved-memory-path-harness".to_string(),
            display_name: Some("Reserved Memory Path Harness".to_string()),
            description: None,
            system_prompt: Some("Harness prompt".to_string()),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            embedder_metadata: Default::default(),
        })
        .execute(&ctx)
        .await
        .unwrap();

        let mut req = build_create_request(harness.id, None, None);
        req.initial_files.push(InitialFile {
            path: "/memory/agent/profile.md".to_string(),
            content: "owned by the server".to_string(),
            encoding: "text".to_string(),
            is_readonly: false,
        });

        let err = session_service
            .create(
                &caller,
                harness.id.uuid(),
                None,
                None,
                SessionSource::Api,
                req,
            )
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("reserved for server-managed memory"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn inherited_harness_starter_files_are_copied_into_new_sessions() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_service = SessionService::new(db.clone());
        let caller = Caller::internal(1);
        let ctx = test_ctx(caller.clone(), db.clone()).await;

        let parent = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "parent".to_string(),
            display_name: Some("Parent".to_string()),
            description: None,
            system_prompt: Some("Parent prompt".to_string()),
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
            embedder_metadata: Default::default(),
        })
        .execute(&ctx)
        .await
        .unwrap();

        let child = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "child".to_string(),
            display_name: Some("Child".to_string()),
            description: None,
            system_prompt: Some("Child prompt".to_string()),
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
            embedder_metadata: Default::default(),
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
                SessionSource::Api,
                build_create_request(child.id, None, None),
            )
            .await
            .unwrap();

        let file_service = WorkspaceFileService::new(db);
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
        let ctx = test_ctx(caller.clone(), db.clone()).await;

        let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "harness".to_string(),
            display_name: Some("Harness".to_string()),
            description: None,
            system_prompt: Some("Harness prompt".to_string()),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            embedder_metadata: Default::default(),
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
            harness_id: None,
            harness_name: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            tools: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
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
                SessionSource::Api,
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
            system_prompt: Some("Harness prompt".to_string()),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            embedder_metadata: Default::default(),
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
                SessionSource::Api,
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
        let other_ctx = test_ctx(Caller::internal(other_org_id), db.clone()).await;

        let other_harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "other-harness".to_string(),
            display_name: Some("Other Harness".to_string()),
            description: None,
            system_prompt: Some("Other".to_string()),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            embedder_metadata: Default::default(),
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
                SessionSource::Api,
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
        let ctx = test_ctx(caller.clone(), db.clone()).await;
        let other_org_id = create_second_org(&db).await;
        let other_model_id = create_model(&db, other_org_id, "cross-org-model").await;

        let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "harness".to_string(),
            display_name: Some("Harness".to_string()),
            description: None,
            system_prompt: Some("Harness".to_string()),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            embedder_metadata: Default::default(),
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
                SessionSource::Api,
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
        let other_ctx = test_ctx(Caller::internal(other_org_id), db.clone()).await;

        let other_harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "other-harness".to_string(),
            display_name: Some("Other Harness".to_string()),
            description: None,
            system_prompt: Some("Other".to_string()),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![AgentCapabilityConfig::new("session_file_system")],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            embedder_metadata: Default::default(),
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
            harness_id: None,
            harness_name: None,
            tags: vec![],
            capabilities: vec![AgentCapabilityConfig::new("session_schedule")],
            initial_files: vec![],
            tools: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
        })
        .execute(&other_ctx)
        .await
        .unwrap();

        let session_row = db
            .create_session(CreateSessionRow {
                source: everruns_platform::SessionSource::Api,
                workspace_id: None,
                org_id: caller.org_id,
                app_id: None,
                harness_id: Some(other_harness.id),
                agent_id: Some(AgentId::from_uuid(other_agent.internal_id)),
                agent_version_id: None,
                agent_config_hash: None,
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
                parallel_tool_calls: None,
                blueprint_id: None,
                blueprint_config: None,
                network_access: None,
                parent_session_id: None,
                budget_root_session_id: None,
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

    // EVE-709: a harness declaring a built-in capability that is not registered in
    // this deployment (e.g. feature-gated `container_sandbox`) must fail session
    // creation with a clear error rather than silently dropping the capability's
    // tools and degrading into a different execution environment.
    #[tokio::test]
    async fn create_rejects_harness_with_unavailable_builtin_capability() {
        let db = Arc::new(StorageBackend::in_memory());
        // Empty registry stands in for a deployment where `container_sandbox` is
        // feature-gated off, so it is absent from the capability registry.
        let registry = CapabilityRegistry::new();
        let session_service = SessionService::with_registry(db.clone(), registry);
        let owner = Caller::internal(DEFAULT_ORG_ID);

        let harness = db
            .create_harness(
                owner.org_id,
                CreateHarnessRow {
                    name: "coding-container".to_string(),
                    display_name: Some("Coding (Container)".to_string()),
                    description: None,
                    system_prompt: Some("coding".to_string()),
                    parent_harness_id: None,
                    default_model_id: None,
                    tags: vec![],
                    initial_files: serde_json::json!([]),
                    mcp_servers: serde_json::json!({}),
                    network_access: None,
                    embedder_metadata: serde_json::json!({}),
                    is_built_in: false,
                },
            )
            .await
            .unwrap();
        db.set_harness_capabilities(
            harness.id.uuid(),
            vec![("container_sandbox".to_string(), 0, serde_json::json!({}))],
        )
        .await
        .unwrap();

        // Owner (admin) so the high-risk capability gate does not fire first.
        let err = session_service
            .create(
                &owner,
                harness.id.uuid(),
                None,
                None,
                SessionSource::Api,
                build_create_request(harness.id, None, None),
            )
            .await
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("capabilities unavailable in this deployment")
                && err.to_string().contains("container_sandbox"),
            "unexpected error: {err}"
        );
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
                    system_prompt: Some("restricted".to_string()),
                    parent_harness_id: None,
                    default_model_id: None,
                    tags: vec![],
                    initial_files: serde_json::json!([]),
                    mcp_servers: serde_json::json!({}),
                    network_access: None,
                    embedder_metadata: serde_json::json!({}),
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
                SessionSource::Api,
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
                    system_prompt: Some("declarative".to_string()),
                    parent_harness_id: None,
                    default_model_id: None,
                    tags: vec![],
                    initial_files: serde_json::json!([]),
                    mcp_servers: serde_json::json!({}),
                    network_access: None,
                    embedder_metadata: serde_json::json!({}),
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
                SessionSource::Api,
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
        let file_service = WorkspaceFileService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let other_org_id = create_second_org(&db).await;
        let other_ctx = test_ctx(Caller::internal(other_org_id), db.clone()).await;

        let other_harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "other-harness".to_string(),
            display_name: Some("Other Harness".to_string()),
            description: None,
            system_prompt: Some("Other".to_string()),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![AgentCapabilityConfig::new("data_knowledge")],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            embedder_metadata: Default::default(),
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
            harness_id: None,
            harness_name: None,
            tags: vec![],
            capabilities: vec![AgentCapabilityConfig::new("data_knowledge")],
            initial_files: vec![],
            tools: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
        })
        .execute(&other_ctx)
        .await
        .unwrap();

        let session_row = db
            .create_session(CreateSessionRow {
                source: everruns_platform::SessionSource::Api,
                workspace_id: None,
                org_id: caller.org_id,
                app_id: None,
                harness_id: None,
                agent_id: None,
                agent_version_id: None,
                agent_config_hash: None,
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
                parallel_tool_calls: None,
                blueprint_id: None,
                blueprint_config: None,
                network_access: None,
                parent_session_id: None,
                budget_root_session_id: None,
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
                None,
            )
            .await
            .unwrap();

        assert!(
            file_service
                .read_file(session_row.id.uuid(), "/knowledge/index.md")
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
        let file_service = WorkspaceFileService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let ctx = test_ctx(caller.clone(), db.clone()).await;

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
            system_prompt: Some("Harness prompt".to_string()),
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
            embedder_metadata: Default::default(),
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
                SessionSource::Api,
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
        let ctx = test_ctx(caller.clone(), db.clone()).await;

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
            system_prompt: Some("Harness prompt".to_string()),
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
            embedder_metadata: Default::default(),
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
                SessionSource::Api,
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
        let ctx = test_ctx(caller.clone(), db.clone()).await;

        let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "harness".to_string(),
            display_name: Some("Harness".to_string()),
            description: None,
            system_prompt: Some("Harness prompt".to_string()),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            embedder_metadata: Default::default(),
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
                SessionSource::Api,
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
                    goal: None,
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
        let ctx = test_ctx(caller.clone(), db.clone()).await;

        let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "harness".to_string(),
            display_name: Some("Harness".to_string()),
            description: None,
            system_prompt: Some("Harness prompt".to_string()),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            embedder_metadata: Default::default(),
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
                SessionSource::Api,
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
                        goal: None,
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
        let ctx = test_ctx(caller.clone(), db.clone()).await;

        let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "harness".to_string(),
            display_name: Some("Harness".to_string()),
            description: None,
            system_prompt: Some("Harness prompt".to_string()),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            embedder_metadata: Default::default(),
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
                    SessionSource::Api,
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
    async fn app_session_creation_enforces_total_session_cap() {
        let db = Arc::new(StorageBackend::in_memory());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let ctx = test_ctx(caller.clone(), db.clone()).await;

        let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "app-session-cap-harness".to_string(),
            display_name: Some("App Session Cap Harness".to_string()),
            description: None,
            system_prompt: Some("test".to_string()),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            embedder_metadata: Default::default(),
        })
        .execute(&ctx)
        .await
        .unwrap();

        let resource_limits = ResourceLimitsConfig {
            max_sessions_per_org: 1,
            ..Default::default()
        };
        let svc = SessionService::new(db.clone()).with_resource_limits(resource_limits);

        let owner_principal = svc
            .principal_service
            .default_owner_principal(&caller, None)
            .await
            .unwrap();
        let app_id = Uuid::new_v4();

        svc.create_from_app(
            &caller,
            harness.id.uuid(),
            None,
            None,
            app_id,
            owner_principal.id,
            owner_principal.resolved_user_id,
            SessionSource::Api,
            build_create_request(harness.id, None, None),
        )
        .await
        .unwrap();

        let err = svc
            .create_from_app(
                &caller,
                harness.id.uuid(),
                None,
                None,
                app_id,
                owner_principal.id,
                owner_principal.resolved_user_id,
                SessionSource::Api,
                build_create_request(harness.id, None, None),
            )
            .await
            .unwrap_err();

        assert!(
            err.downcast_ref::<ResourceLimitError>().is_some(),
            "expected ResourceLimitError, got: {err}"
        );
        assert!(
            err.to_string().contains("Session limit reached"),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn concurrent_session_cap_enforced() {
        use crate::domains::sessions::limits::OrgCaps;
        use crate::errors::BadRequestError;
        use crate::storage::models::UpdateSession;

        let db = Arc::new(StorageBackend::in_memory());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let ctx = test_ctx(caller.clone(), db.clone()).await;

        let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
            name: "cap-test-harness".to_string(),
            display_name: Some("Cap Test Harness".to_string()),
            description: None,
            system_prompt: Some("test".to_string()),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            embedder_metadata: Default::default(),
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
                SessionSource::Api,
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
                SessionSource::Api,
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
                    system_prompt: Some("hooked".to_string()),
                    parent_harness_id: None,
                    default_model_id: None,
                    tags: vec![],
                    initial_files: serde_json::json!([]),
                    mcp_servers: serde_json::json!({}),
                    network_access: None,
                    embedder_metadata: serde_json::json!({}),
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
                SessionSource::Api,
                build_create_request(harness_id, None, None),
            )
            .await
            .unwrap();

        // The session_start hook ran during create and wrote into the VFS.
        let file = WorkspaceFileService::new(db)
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
                SessionSource::Api,
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
