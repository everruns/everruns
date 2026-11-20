//! Construction and entry points for reason execution.

use super::*;

impl ReasonAtom {
    /// Create a new ReasonAtom
    pub fn new(
        context_resolver: impl TurnContextResolver + 'static,
        message_retriever: impl MessageRetriever + 'static,
        capability_registry: CapabilityRegistry,
        event_emitter: impl PhaseEffectSink + 'static,
    ) -> Self {
        Self {
            causal_context: Default::default(),
            native_async: None,
            context_resolver: Arc::new(context_resolver),
            message_retriever: Arc::new(message_retriever),
            capability_registry,
            event_emitter: PhaseEffectEmitter::new(Arc::new(event_emitter)),
            image_resolver: None,
            file_resolver: None,
            provider_stall_timeout: None,
            provider_retry_config: LlmRetryConfig::default(),
            durable_tool_result_store: None,
            partial_stream_store: None,
            reasoning_effort_handle: None,
            utility_llm_service: None,
            decisions: None,
            mcp_invoker: None,
            schedule_store: None,
            compaction_checkpoint_store: None,
            background_call: Default::default(),
        }
    }

    /// Set the session schedule store used by `usage_limit_auto_continue` to
    /// schedule a continuation after a provider usage limit resets.
    pub fn with_schedule_store(
        mut self,
        store: Arc<dyn crate::engine::session_services::SessionScheduleStore>,
    ) -> Self {
        self.schedule_store = Some(store);
        self
    }

    pub fn with_compaction_checkpoint_store(
        mut self,
        store: Arc<dyn crate::engine::CompactionCheckpointStore>,
    ) -> Self {
        self.compaction_checkpoint_store = Some(store);
        self
    }

    /// Set the image resolver: image_file references are resolved to image
    /// data before the LLM call (required for multimodal attachments).
    pub fn with_image_resolver(mut self, resolver: Arc<dyn ImageResolver>) -> Self {
        self.image_resolver = Some(resolver);
        self
    }

    pub fn with_file_resolver(mut self, resolver: Arc<dyn FileResolver>) -> Self {
        self.file_resolver = Some(resolver);
        self
    }

    /// Set the provider stall timeout. If no token arrives within this window,
    /// the stream is aborted and the activity fails with a retryable error.
    pub fn with_provider_stall_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.provider_stall_timeout = Some(timeout);
        self
    }

    /// Set the bounded provider recovery policy.
    pub fn with_provider_retry_config(mut self, config: LlmRetryConfig) -> Self {
        self.provider_retry_config = config;
        self
    }

    /// Set the durable tool result store for transcript repair (EVE-533): repair
    /// replays settled results or synthesizes interrupted placeholders from it
    /// rather than always emitting a generic "cancelled" message.
    pub fn with_durable_tool_result_store(
        mut self,
        store: Arc<dyn DurableToolResultStore>,
    ) -> Self {
        self.durable_tool_result_store = Some(store);
        self
    }

    /// Set the partial-stream store for ContinuePartial recovery (EVE-532).
    pub fn with_partial_stream_store(mut self, store: Arc<dyn PartialStreamStore>) -> Self {
        self.partial_stream_store = Some(store);
        self
    }

    /// Set the live reasoning-effort handle (EVE-595).
    ///
    /// When set and holding a value, the effort it carries overrides the
    /// message-derived effort for every LLM step. Because the handle is shared
    /// and re-read on each step, a tool that mutates it mid-turn causes
    /// subsequent steps in the same turn to use the new effort.
    pub fn with_reasoning_effort_handle(
        mut self,
        handle: crate::engine::tool_context::ReasoningEffortHandle,
    ) -> Self {
        self.reasoning_effort_handle = Some(handle);
        self
    }

    /// Set the utility LLM service used by model-backed end-of-message output
    /// guardrails (EVE-573). When unset, those guardrails fail open.
    pub fn with_utility_llm_service(
        mut self,
        service: Arc<dyn crate::engine::UtilityLlmService>,
    ) -> Self {
        self.utility_llm_service = Some(service);
        self
    }

    /// Set the decisions used by guardrail checks that ask for typed
    /// answers. When unset, those checks fail open.
    pub fn with_decisions(mut self, service: Arc<dyn crate::engine::DecisionsService>) -> Self {
        self.decisions = Some(service);
        self
    }

    /// Set the scoped MCP invoker used by output-stage `mcp` guardrail checks.
    pub fn with_mcp_invoker(mut self, invoker: Arc<dyn crate::engine::McpToolInvoker>) -> Self {
        self.mcp_invoker = Some(invoker);
        self
    }
}

impl ReasonAtom {
    /// Stable phase name used by logs and durable activity adapters.
    pub fn name(&self) -> &'static str {
        "reason"
    }

    /// Execute a reason phase using host-injected portable contracts.
    pub async fn execute(&self, input: ReasonInput) -> Result<ReasonResult> {
        self.execute_inner(input, None).await
    }
}

impl ReasonAtom {
    /// Execute using a context already assembled by the host.
    pub async fn execute_with_assembled_context(
        &self,
        input: ReasonInput,
        assembled: AssembledTurnContext,
    ) -> Result<ReasonResult> {
        self.execute_inner(input, Some(assembled)).await
    }
}
