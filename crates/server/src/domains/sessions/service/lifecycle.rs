//! Mutating a session: update, status, delete, and the pin/archive flags.

use super::*;

impl SessionService {
    pub async fn update(
        &self,
        caller: &Caller,
        id: Uuid,
        req: UpdateSessionRequest,
    ) -> Result<Option<Session>> {
        if !caller.is_internal
            && req.tags.as_ref().is_some_and(|tags| {
                tags.iter().any(|tag| {
                    tag == PLATFORM_CHAT_STARTER_TAG
                        || RESERVED_SESSION_TAG_PREFIXES
                            .iter()
                            .any(|prefix| tag.starts_with(prefix))
                })
            })
        {
            return Err(BadRequestError::new(RESERVED_SESSION_TAG_ERROR).into());
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

    /// Pin a session for a user
    pub async fn pin(&self, caller: &Caller, user_id: Uuid, session_id: Uuid) -> Result<()> {
        self.db
            .pin_session(user_id, SessionId::from_uuid(session_id), caller.org_id)
            .await
    }

    /// Archive a session: it stays readable and its history intact, but drops
    /// out of default list results. Idempotent.
    pub async fn archive(&self, caller: &Caller, session_id: Uuid) -> Result<bool> {
        Ok(self
            .db
            .set_session_archived(caller.org_id, SessionId::from_uuid(session_id), true)
            .await?
            .is_some())
    }

    /// Restore an archived session to the default list results. Idempotent.
    pub async fn unarchive(&self, caller: &Caller, session_id: Uuid) -> Result<bool> {
        Ok(self
            .db
            .set_session_archived(caller.org_id, SessionId::from_uuid(session_id), false)
            .await?
            .is_some())
    }

    /// Unpin a session for a user in the caller's current org.
    /// Authorization is enforced at `Command::run` via `UnpinSession::policy`.
    pub async fn unpin(&self, caller: &Caller, user_id: Uuid, session_id: Uuid) -> Result<bool> {
        self.db
            .unpin_session(user_id, SessionId::from_uuid(session_id), caller.org_id)
            .await
    }

    /// Fire session-lifecycle hooks (`session_start` / `session_end`) for a
    /// session. Advisory only — collects + finalizes hook specs from the given
    /// capability list, builds the bash-backed adapters against the session's
    /// VFS, and runs them. Any failure is logged, never propagated.
    pub(crate) async fn fire_session_lifecycle_hooks(
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
                    everruns_platform::capabilities::user_hooks::disabled_contributions(
                        config.config_value(),
                    ),
                );
            }
        }
        let specs = everruns_core::hook_adapter::finalize_hook_specs(contributions, &disabled);
        let file_store: Arc<dyn everruns_core::session_files::SessionFileSystem> = Arc::new(
            crate::domains::session_files::WorkspaceFileService::new(self.db.clone()),
        );
        let dispatcher: Arc<dyn everruns_core::hook_executor::BashHookDispatcher> =
            Arc::new(everruns_integrations_bashkit::BashkitShellHookDispatcher::new(file_store));
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

    /// Resolve the effective capability configs for a session (harness chain +
    /// agent + session), used by the lifecycle-hook firing helpers. Best-effort:
    /// returns an empty list if the harness/agent can't be loaded, so a
    /// resolution failure degrades to "no hooks" rather than erroring a
    /// create/delete.
    pub(crate) async fn resolve_session_capability_configs(
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

    /// Resolve a session's agent_id from internal UUID to the agent's public_id.
    /// The DB stores the internal UUID as FK; the API should return the public_id.
    pub(crate) async fn resolve_session_agent_id(
        &self,
        org_id: i64,
        session: &mut Session,
    ) -> Result<()> {
        if let Some(aid) = session.agent_id
            && let Some(public_id) = self.db.get_agent_public_id(org_id, aid).await?
            && let Ok(agent_id) = public_id.parse::<AgentId>()
        {
            session.agent_id = Some(agent_id);
        }
        Ok(())
    }
}
