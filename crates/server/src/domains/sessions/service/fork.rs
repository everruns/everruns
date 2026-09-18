//! Forking a session and copying its events, workspace files and storage.

use super::*;

impl SessionService {
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
                // A fork is not an app-channel arrival: it has no `app_id`
                // today and gets no `endpoint_id` for the same reason. It
                // keeps only the origin, below.
                None,
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

    pub(crate) async fn copy_session_events(
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

    pub(crate) async fn copy_workspace_files(
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

    pub(crate) async fn copy_session_storage(
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
}
