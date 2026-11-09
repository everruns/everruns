//! Capability commands, command-host facilities and live registration
//! queries on a running in-process runtime.

use super::*;

impl InProcessRuntime {
    /// Inspect current eager/revealed MCP readiness without starting discovery.
    #[cfg(feature = "mcp")]
    pub async fn mcp_discovery_statuses(
        &self,
        session_id: SessionId,
    ) -> Result<Vec<(String, crate::host::McpDiscoveryStatus)>> {
        Ok(self
            .mcp_readiness(session_id)
            .await?
            .servers
            .into_iter()
            .filter(|server| !server.deferred)
            .map(|server| (server.name, server.status))
            .collect())
    }

    /// Readiness of every configured MCP server of a session that discovers
    /// tools, including unrevealed deferred ones, without starting discovery.
    #[cfg(feature = "mcp")]
    pub async fn mcp_readiness(&self, session_id: SessionId) -> Result<crate::host::McpReadiness> {
        let (servers, deferred) = self.session_mcp_partition(session_id).await?;
        Ok(crate::host::mcp::server_readiness(
            &self.mcp_discovery_cache,
            session_id.uuid(),
            &servers,
            &deferred,
        ))
    }

    /// Run MCP discovery now and return the resulting readiness.
    ///
    /// `server` names one configured server, deferred or not; `None` probes
    /// every eager and revealed server. An attempt bypasses cached freshness,
    /// the transient retry cooldown and the authentication hold, asking the
    /// MCP auth provider for credentials again. A successful attempt serves
    /// the session's next turn; a transient failure keeps the previous
    /// complete catalog. Waits at most the discovery deadline.
    #[cfg(feature = "mcp")]
    pub async fn probe_mcp(
        &self,
        session_id: SessionId,
        server: Option<&str>,
    ) -> Result<crate::host::McpReadiness> {
        let (servers, deferred) = self.session_mcp_partition(session_id).await?;
        let names: Vec<String> = match server {
            Some(name) => vec![name.trim().to_string()],
            None => servers
                .iter()
                .filter(|(name, server)| server.tool_discovery && !deferred.contains(name))
                .map(|(name, _)| name.clone())
                .collect(),
        };
        crate::host::mcp::probe_servers(
            &self.mcp_discovery_cache,
            self.mcp_client(),
            session_id.uuid(),
            &servers,
            &names,
        )
        .await
        .map_err(|error| AgentLoopError::config(error.to_string()))?;
        Ok(crate::host::mcp::server_readiness(
            &self.mcp_discovery_cache,
            session_id.uuid(),
            &servers,
            &deferred,
        ))
    }

    /// Watch for MCP readiness changes on this runtime's discovery cache.
    #[cfg(feature = "mcp")]
    pub fn watch_mcp_readiness(&self) -> crate::host::McpReadinessWatch {
        crate::host::McpReadinessWatch::new(self.mcp_discovery_cache.subscribe())
    }

    /// A session's effective MCP servers and the deferred ones not yet revealed.
    #[cfg(feature = "mcp")]
    async fn session_mcp_partition(
        &self,
        session_id: SessionId,
    ) -> Result<(crate::ScopedMcpServers, Vec<String>)> {
        let session = self
            .session_store
            .get_session(session_id)
            .await?
            .ok_or_else(|| AgentLoopError::session_not_found(session_id))?;
        let agent = match session.agent_id {
            Some(id) => self.agent_store.get_agent(id).await?,
            None => None,
        };
        let servers = self.session_mcp_servers(&session, agent.as_ref()).await;
        let revealed = if servers.values().any(|server| server.deferred) {
            crate::revealed_mcp_servers(self.storage_store.as_ref(), session_id).await
        } else {
            Default::default()
        };
        let (_, deferred) = crate::partition_deferred_mcp_servers(&servers, &revealed);
        Ok((servers, deferred))
    }

    /// The implementation a canonical id or alias resolves to on this runtime.
    ///
    /// Lets a host that registers capabilities after composition tell its own
    /// registration apart from a same-id one the composition already holds.
    pub fn registered_capability(&self, capability_id: &str) -> Option<Arc<dyn Capability>> {
        self.host_composition
            .capability_registry()
            .get(capability_id)
            .cloned()
    }

    /// Forget cached MCP tool discovery for a session.
    ///
    /// The next context assembly or model call rediscovers the session's
    /// scoped servers, requesting credentials from the MCP auth provider
    /// again. Hosts call this after changing a session's servers or their
    /// credentials outside the capability activation path.
    #[cfg(feature = "mcp")]
    pub fn invalidate_mcp_discovery(&self, session_id: SessionId) {
        self.mcp_discovery_cache
            .invalidate_session(session_id.uuid());
    }

    /// Execute a system command declared by a registered capability.
    ///
    /// Looks up the first capability whose `commands()` includes the named
    /// command (in capability-resolution order) and delegates to its
    /// `execute_command`. Returns an error if no capability declares the
    /// requested name. The coding-CLI example uses this for `/model`
    /// (provided by `ModelSwitcherCapability`) so the dispatch path stays
    /// inside the capability instead of the TUI's local `handle_command`
    /// branches.
    pub async fn execute_command(
        &self,
        session_id: SessionId,
        request: crate::command::ExecuteCommandRequest,
    ) -> Result<crate::command::CommandResult> {
        let ctx = self.load_context(session_id).await?;
        let registry = self.host_composition.capability_registry();
        // The already-assembled context seeds the host so dispatch and
        // execution assemble it once.
        let host = self.store_command_host(session_id, ctx.clone());
        let exec_ctx = crate::command::CommandExecutionContext::new(session_id, Arc::new(host));
        for config in &ctx.resolved_capability_configs {
            let Some(capability) = registry.get(config.capability_id()) else {
                continue;
            };
            if capability.commands().iter().any(|c| c.name == request.name) {
                return capability.execute_command(&request, &exec_ctx).await;
            }
        }
        Err(AgentLoopError::config(format!(
            "no capability declares command /{}",
            request.name
        )))
    }

    /// Command host facilities over this session's effective context.
    ///
    /// The same store-backed host that capability commands receive in
    /// [`Self::execute_command`]: credential-free turn context after message
    /// filters and prompt contributions, and tool-less out-of-band completions
    /// against the session's resolved model and provider. Nothing it does
    /// appends to canonical history or runs a turn.
    pub async fn command_host(
        &self,
        session_id: SessionId,
    ) -> Result<Arc<dyn crate::command_host::CommandHost>> {
        let ctx = self.load_context(session_id).await?;
        Ok(Arc::new(self.store_command_host(session_id, ctx)))
    }

    // Context-aware commands (e.g. /btw) get the same store-backed host
    // facilities the server provides.
    fn store_command_host(
        &self,
        session_id: SessionId,
        assembled: AssembledTurnContext,
    ) -> crate::host::StoreCommandHost {
        let org = super::in_process_internal_org_id(&assembled.snapshot.organization_id);
        crate::host::StoreCommandHost::new(
            session_id,
            self.harness_store.clone(),
            self.agent_store.clone(),
            self.session_store.clone(),
            self.event_history.clone(),
            self.provider_store.clone(),
            (*self.host_composition.capability_registry()).clone(),
            self.host_composition.driver_registry().clone(),
        )
        .with_file_store(self.file_store.clone())
        .with_dreaming(Arc::new(
            crate::host::dreaming::RuntimeDreamingCommand::new(self.clone(), org, session_id)
                .with_local_wakeup(),
        ))
        .with_assembled_context(assembled)
    }

    /// List slash commands available for a session.
    ///
    /// Resolves the session's harness/agent capability chain and aggregates
    /// commands declared via [`Capability::commands`], deduplicated by name
    /// (first occurrence wins, matching the order of resolved capabilities).
    /// This is the embedded equivalent of the server's
    /// `GET /v1/sessions/{id}/commands` system-commands list — skill
    /// commands are not included here because skills are discovered via the
    /// platform filesystem rather than the capability registry.
    pub async fn list_commands(
        &self,
        session_id: SessionId,
    ) -> Result<Vec<crate::command::CommandDescriptor>> {
        let ctx = self.load_context(session_id).await?;
        let registry = self.host_composition.capability_registry();
        let mut seen = std::collections::HashSet::new();
        let mut commands = Vec::new();
        for config in &ctx.resolved_capability_configs {
            let Some(capability) = registry.get(config.capability_id()) else {
                continue;
            };
            for command in capability.commands() {
                if seen.insert(command.name.clone()) {
                    commands.push(command);
                }
            }
        }
        Ok(commands)
    }
}
