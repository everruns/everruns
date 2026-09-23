//! Constructing sessions: from scratch, from an app, from a trigger, from a blueprint.

use super::*;

impl SessionService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self {
            principal_service: PrincipalService::new(db.clone()),
            capability_registry: crate::platform::oss_capability_registry(),
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
        app_internal_id: Option<Uuid>,
        agent_version_policy: AgentVersionPolicy,
        agent_version_id: Option<everruns_provider::typed_id::AgentVersionId>,
        // Internal id of the channel this ingress resolved, recorded as
        // `sessions.channel_id` so provenance names the door, not the bundle
        // (EVE-1004). `None` only where the caller genuinely has no channel
        // pointer — migrated App schedules, whose trigger row kept
        // `execution_app_id` but never a channel id. Leaving those NULL is
        // the same rule the 137 backfill follows: derive or leave unknown,
        // never guess an App's channel for it.
        channel_internal_id: Option<Uuid>,
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
            Some((agent_version_policy, agent_version_id)),
            app_internal_id,
            channel_internal_id,
            Some((owner_principal_id, resolved_owner_user_id)),
            source,
            req,
        )
        .await
    }

    /// Create a session owned by an agent that one of its triggers started.
    /// Mirrors [`Self::create_from_app`] but there is no App row: the session
    /// runs on the agent's harness, is hosted by the agent through
    /// `agent_public_id`, and is owned by `owner_principal_id` so the
    /// shared-session reuse lookup (`find_session_by_tags_and_owner`) matches
    /// across invocations. `app_id` is `None`.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_from_agent_trigger(
        &self,
        caller: &Caller,
        harness_id: Uuid,
        agent_internal_id: Uuid,
        agent_public_id: AgentId,
        owner_principal_id: PrincipalId,
        resolved_owner_user_id: Option<Uuid>,
        source: SessionSource,
        req: CreateSessionRequest,
    ) -> Result<Session> {
        self.create_inner(
            caller,
            harness_id,
            Some(agent_internal_id),
            Some(agent_public_id),
            None,
            None,
            None,
            Some((owner_principal_id, resolved_owner_user_id)),
            source,
            req,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn create_inner(
        &self,
        caller: &Caller,
        harness_id: Uuid,
        agent_internal_id: Option<Uuid>,
        agent_public_id: Option<AgentId>,
        agent_version_selection: Option<(
            AgentVersionPolicy,
            Option<everruns_provider::typed_id::AgentVersionId>,
        )>,
        app_id: Option<Uuid>,
        // Channel whose ingress is creating this session (EVE-1004). Every
        // app-channel path knows its channel, so this is passed rather than
        // inferred from tags — the tag spelling differs per transport and a
        // multi-channel App makes `app_id` alone ambiguous.
        channel_id: Option<Uuid>,
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
                let (version_policy, pinned_version_id) =
                    agent_version_selection.unwrap_or_default();
                match version_policy {
                    AgentVersionPolicy::Pinned => {
                        if let Some(version_id) = pinned_version_id {
                            self.db.get_agent_version(org_id, version_id).await?
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
        crate::domains::mcp_servers::scoped_mcp::validate_merged_scoped_mcp_servers_for_org(
            &self.db,
            org_id,
            scoped_mcp_layers,
        )
        .await?;

        // Serialize capabilities to JSON for storage
        let capabilities_json = serde_json::to_value(&session_capabilities)?;

        let hints_json = req
            .hints
            .as_ref()
            .map(|h| serde_json::to_value(h).unwrap_or_default());

        if !caller.is_internal
            && req.tags.iter().any(|tag| {
                RESERVED_SESSION_TAG_PREFIXES
                    .iter()
                    .any(|prefix| tag.starts_with(prefix))
            })
        {
            return Err(BadRequestError::new(RESERVED_SESSION_TAG_ERROR).into());
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
                if everruns_provider::typed_id::WorkspaceId::from_uuid(workspace.id).to_string()
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
            channel_id,
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
            harness_id: Some(harness_id),
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
        crate::domains::mcp_servers::scoped_mcp::validate_merged_scoped_mcp_servers_for_org(
            &self.db,
            org_id,
            [&effective_harness.mcp_servers, &req.mcp_servers],
        )
        .await?;
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
            channel_id: None,
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

    pub(crate) async fn apply_session_seed(
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

    pub(crate) async fn validate_model_id(
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
}
