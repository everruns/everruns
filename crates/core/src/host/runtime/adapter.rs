//! [`RuntimeHostAdapter`] for [`InProcessRuntime`]: the services a turn
//! reads, including the ones a trusted host forwards
//! ([`HostExtensions`](crate::host::HostExtensions)).

use super::*;

#[async_trait]
impl RuntimeHostAdapter for InProcessRuntime {
    async fn set_session_status(
        &self,
        _org_id: i64,
        session_id: SessionId,
        status: SessionExecutionState,
    ) -> Result<()> {
        // The existence check keeps lifecycle warnings surfacing a missing
        // session; the store decides whether the state is kept.
        self.session_store
            .get_session(session_id)
            .await?
            .ok_or_else(|| AgentLoopError::session_not_found(session_id))?;
        self.session_store
            .set_execution_state(session_id, status)
            .await
    }

    async fn load_resolved_turn(
        &self,
        _org_id: i64,
        session_id: SessionId,
    ) -> Result<ResolvedTurnInputs> {
        let session = self.attached_session(session_id).await?;
        let agent = match session.agent_id {
            Some(agent_id) => self.agent_store.get_agent(agent_id).await?,
            None => None,
        };
        let harness = self
            .harness_store
            .get_harness(session.harness_id)
            .await?
            .ok_or_else(|| AgentLoopError::harness_not_found(session.harness_id))?;
        // Platform projection: stored records stop here; execution receives
        // the canonical resolved value (EVE-872).
        let snapshot = ResolvedExecutionSnapshot::project(&harness, agent.as_ref(), &session)?;
        let messages = match self
            .compaction_checkpoint_store
            .latest_source_sequence(session_id)
            .await?
        {
            Some(checkpoint) => {
                self.event_history
                    .load_since_checkpoint(session_id, checkpoint)
                    .await?
            }
            None => self.event_history.load(session_id).await?,
        };

        // Discover session MCP tools for the LLM (knowledge/integrations/runtime-mcp.md D4).
        // A host that serves MCP lists them per input instead
        // (`load_resolved_turn_for_execution`).
        #[cfg(feature = "mcp")]
        let mcp_tool_definitions = if self.extensions.hooks.mcp.is_some() {
            vec![]
        } else {
            let scoped_servers = self.session_mcp_servers(&session, agent.as_ref()).await;
            crate::host::mcp::discover_turn_tool_definitions(
                &self.mcp_discovery_cache,
                self.mcp_client(),
                session_id,
                &scoped_servers,
                self.storage_store.as_ref(),
            )
            .await
        };
        #[cfg(not(feature = "mcp"))]
        let mcp_tool_definitions = vec![];

        Ok(ResolvedTurnInputs {
            snapshot,
            messages,
            mcp_tool_definitions,
        })
    }

    async fn mcp_executor(
        &self,
        _org_id: i64,
        session_id: SessionId,
        _agent_id: Option<AgentId>,
    ) -> Option<Arc<dyn crate::McpToolInvoker>> {
        if let Some(mcp) = &self.extensions.hooks.mcp {
            return mcp.executor().await;
        }
        self.framework_mcp_executor(session_id).await
    }

    fn capability_registry(&self) -> CapabilityRegistry {
        // Snapshot per call, so a capability registered after composition
        // (EVE-917) reaches the next caller rather than a stale copy.
        (*self.host_composition.capability_registry()).clone()
    }

    fn driver_registry(&self) -> DriverRegistry {
        self.host_composition.driver_registry().clone()
    }

    fn harness_store(&self, _org_id: i64) -> Arc<dyn HarnessStore> {
        self.harness_store.clone()
    }

    fn agent_store(&self, _org_id: i64) -> Arc<dyn AgentStore> {
        self.agent_store.clone()
    }

    fn session_store(&self, _org_id: i64) -> Arc<dyn SessionStore> {
        self.session_store.clone()
    }

    fn session_mutator(&self, _org_id: i64) -> Arc<dyn SessionMutator> {
        self.session_store.clone()
    }

    fn provider_store(&self, _org_id: i64) -> Arc<dyn ProviderStore> {
        self.provider_store.clone()
    }

    fn message_store(&self) -> Arc<dyn MessageRetriever> {
        self.event_history.clone()
    }

    fn native_async_store(&self) -> Option<Arc<dyn crate::native_async_store::NativeAsyncStore>> {
        self.native_async_store.clone()
    }

    fn dreaming_store(&self) -> Option<Arc<dyn crate::dreaming::DreamingStore>> {
        Some(self.dreaming_store.clone())
    }

    fn compaction_checkpoint_store(&self) -> Option<Arc<dyn crate::CompactionCheckpointStore>> {
        Some(self.compaction_checkpoint_store.clone())
    }

    fn partial_stream_store(&self) -> Option<Arc<dyn crate::durability::PartialStreamStore>> {
        Some(super::super::partial_stream::over(&self.event_log))
    }

    fn event_emitter(&self) -> Arc<dyn EventEmitter> {
        self.event_emitter.clone()
    }

    fn bash_hook_dispatcher(
        &self,
        org_id: i64,
    ) -> Arc<dyn crate::hook_executor::BashHookDispatcher> {
        self.bash_hook_dispatcher_factory
            .as_ref()
            .map(|factory| factory(self.file_store(org_id)))
            .unwrap_or_else(|| Arc::new(crate::host::DisabledBashHookDispatcher))
    }

    fn file_store(&self, _org_id: i64) -> Arc<dyn SessionFileSystem> {
        self.file_store.clone()
    }

    fn storage_store(&self, _org_id: i64) -> Option<Arc<dyn SessionStorageStore>> {
        // In-process runtime: the store is already bound to this runtime's org.
        Some(self.storage_store.clone())
    }

    fn connection_resolver(&self, _org_id: i64) -> Option<Arc<dyn UserConnectionResolver>> {
        self.connection_resolver.clone()
    }

    fn session_task_registry(&self, _org_id: i64) -> Option<Arc<dyn SessionTaskRegistry>> {
        self.session_task_registry.clone()
    }

    fn schedule_store(
        &self,
        org_id: i64,
    ) -> Option<Arc<dyn crate::session_services::SessionScheduleStore>> {
        self.schedule_store_factory
            .as_ref()
            .map(|factory| factory(org_id))
    }

    fn tool_context_extensions(
        &self,
        request: crate::host::runtime_host::ToolContextRequest<'_>,
    ) -> crate::tool_context::ToolContextExtensions {
        self.tool_context_extensions_factory
            .as_ref()
            .map(|factory| factory(request.org_id, request.session_id))
            .unwrap_or_default()
    }

    fn subagent_delegate(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Option<Arc<dyn crate::subagent_delegation::SubagentSessionDelegate>> {
        self.subagent_delegate_factory
            .as_ref()
            .map(|factory| factory(org_id, session_id))
    }

    fn tool_augmentor(&self) -> Option<Arc<dyn crate::host::HostToolAugmentor>> {
        self.tool_augmentor.clone()
    }

    fn decisions(&self) -> Option<Arc<dyn crate::DecisionsService>> {
        Some(
            self.extensions
                .composition
                .decisions
                .clone()
                .unwrap_or_else(|| self.host_composition.decisions()),
        )
    }

    fn utility_llm_service(&self) -> Option<Arc<dyn crate::UtilityLlmService>> {
        Some(
            self.extensions
                .composition
                .utility_llm
                .clone()
                .unwrap_or_else(|| self.host_composition.utility_llm_service()),
        )
    }

    fn egress_service(&self) -> Option<Arc<dyn crate::EgressService>> {
        Some(
            self.extensions
                .composition
                .egress
                .clone()
                .unwrap_or_else(|| self.host_composition.egress_service()),
        )
    }

    fn provider_retry_config(&self) -> Option<everruns_contracts::llm_retry::LlmRetryConfig> {
        self.provider_retry_config.clone()
    }

    fn provider_stall_timeout(&self) -> Option<std::time::Duration> {
        self.provider_stall_timeout
    }

    fn reasoning_effort_handle(
        &self,
        _session_id: SessionId,
    ) -> Option<crate::tool_context::ReasoningEffortHandle> {
        self.reasoning_effort_handle.clone()
    }

    fn session_reasoning_effort(
        &self,
        _session_id: SessionId,
    ) -> Option<everruns_contracts::ReasoningEffort> {
        self.session_reasoning_effort
            .as_ref()
            .and_then(crate::tool_context::ReasoningEffortHandle::get)
    }

    fn budget_checker(&self, _: i64, agent: Option<AgentId>) -> Option<Arc<dyn BudgetChecker>> {
        match &self.extensions.authorities.budget {
            Some(budget) => budget(agent),
            None => self.budget_checker.clone(),
        }
    }

    fn payment_authority(
        &self,
        _: i64,
        agent: Option<AgentId>,
    ) -> Option<Arc<dyn crate::tool_execution::PaymentAuthority>> {
        self.extensions
            .authorities
            .payment
            .as_ref()
            .and_then(|payment| payment(agent))
    }

    fn session_creation_authority(
        &self,
        _: i64,
        _: SessionId,
    ) -> Option<Arc<dyn crate::delegation_services::SessionCreationAuthority>> {
        self.extensions.authorities.session_creation.clone()
    }

    fn outbound_tool_rate_limiter(
        &self,
        _: i64,
    ) -> Option<Arc<dyn crate::tool_execution::OutboundToolRateLimiter>> {
        self.extensions.authorities.outbound_rate_limiter.clone()
    }

    fn provider_credential_store(
        &self,
        _: i64,
    ) -> Option<Arc<dyn crate::connection_services::ProviderCredentialStore>> {
        self.extensions.stores.provider_credentials.clone()
    }

    fn leased_resource_store(
        &self,
        _: i64,
    ) -> Option<Arc<dyn crate::session_services::LeasedResourceStore>> {
        self.extensions.stores.leased_resources.clone()
    }

    fn durable_tool_result_store(
        &self,
    ) -> Option<Arc<dyn crate::durability::DurableToolResultStore>> {
        self.extensions.stores.durable_tool_results.clone()
    }

    fn session_resource_registry(
        &self,
        _: i64,
    ) -> Option<Arc<dyn crate::session_services::SessionResourceRegistry>> {
        self.extensions.stores.session_resources.clone()
    }

    fn subagent_spawn_store(
        &self,
    ) -> Option<Arc<dyn crate::delegation_services::SubagentSpawnStore>> {
        self.extensions.stores.subagent_spawns.clone()
    }

    fn image_resolver(&self, _: i64) -> Option<Arc<dyn crate::image_services::ImageResolver>> {
        self.extensions.stores.images.clone()
    }

    fn file_resolver(&self, _: i64) -> Option<Arc<dyn crate::file_services::FileResolver>> {
        self.extensions.stores.files.clone()
    }

    fn image_artifact_store(
        &self,
        _: i64,
    ) -> Option<Arc<dyn crate::image_services::ImageArtifactStore>> {
        self.extensions.stores.image_artifacts.clone()
    }

    fn hosted_mcp_resolver(
        &self,
        _: i64,
        _: SessionId,
        agent: Option<AgentId>,
        input: MessageId,
    ) -> Option<Arc<dyn everruns_contracts::hosted_mcp::HostedMcpResolver>> {
        self.extensions
            .hooks
            .hosted_mcp
            .as_ref()
            .and_then(|resolve| resolve(agent, input))
    }

    async fn load_resolved_turn_for_execution(
        &self,
        org_id: i64,
        session_id: SessionId,
        input_message_id: MessageId,
    ) -> Result<ResolvedTurnInputs> {
        let mut inputs = self.load_resolved_turn(org_id, session_id).await?;
        if let Some(mcp) = &self.extensions.hooks.mcp {
            inputs.mcp_tool_definitions = mcp.tool_definitions(input_message_id).await?;
        }
        if let Some(knowledge) = &self.extensions.hooks.knowledge {
            let configs = crate::runtime_context::resolve_snapshot_capabilities(
                &inputs.snapshot,
                &self.capability_registry(),
            );
            crate::host::add_automatic_knowledge(
                knowledge.as_ref(),
                &configs.resolved_capability_configs,
                session_id,
                input_message_id,
                &mut inputs.messages,
            )
            .await;
        }
        Ok(inputs)
    }
}

impl InProcessRuntime {
    /// The executor over the session agent's own MCP servers, through the
    /// Framework MCP client.
    #[cfg(feature = "mcp")]
    async fn framework_mcp_executor(
        &self,
        session_id: SessionId,
    ) -> Option<Arc<dyn crate::McpToolInvoker>> {
        let session = self.session_store.get_session(session_id).await.ok()??;
        let agent = match session.agent_id {
            Some(agent_id) => self.agent_store.get_agent(agent_id).await.ok().flatten(),
            None => None,
        };
        let scoped_servers = self.session_mcp_servers(&session, agent.as_ref()).await;
        crate::host::mcp::build_executor(self.mcp_client(), &scoped_servers)
            .map(|executor| executor as Arc<dyn crate::McpToolInvoker>)
    }

    #[cfg(not(feature = "mcp"))]
    async fn framework_mcp_executor(
        &self,
        _session_id: SessionId,
    ) -> Option<Arc<dyn crate::McpToolInvoker>> {
        None
    }
}
