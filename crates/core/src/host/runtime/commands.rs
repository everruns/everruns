//! Capability commands, command-host facilities and live registration
//! queries on a running in-process runtime.

use super::*;

impl InProcessRuntime {
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
