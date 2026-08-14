// In-Memory Agentic Loop
//
// Convenience helpers for running full agentic loops in memory without
// external dependencies (database, real LLM, etc.). Perfect for:
// - Unit and integration tests
// - Prototyping and experimentation
// - Examples and documentation
//
// The `InMemoryAgenticLoop` bundles all in-memory stores and atoms,
// providing a simple API for executing agent turns.

use std::sync::Arc;

use crate::llmsim_driver::{LlmSimConfig, LlmSimDriver};
use chrono::Utc;
use everruns_capability::CapabilityRef as AgentCapabilityConfig;
use everruns_core::ExecutionContext;
use everruns_core::agent_definition::AgentDefinition;
use everruns_core::capabilities::{Capability, CapabilityRegistry};
use everruns_core::event_emitter::EventEmitter;
use everruns_core::events::{Event, EventContext, EventData, EventRequest, InputMessageData};
use everruns_core::message::Message;
use everruns_core::message_retriever::{InputMessage, MessageRetriever};
use everruns_core::session::ExecutionSession;
use everruns_core::tools::{Tool, ToolRegistry, ToolRegistryBuilder};
use everruns_core::turn::{TurnAction, TurnContext, TurnOutcome, TurnStateMachine, TurnStopReason};
use everruns_engine::{
    ActAtom, ActInput, InputAtom, InputAtomInput, ReasonAtom, ReasonInput, ReasonResult,
};
use everruns_host::{
    EventHistory, EventReadLimit, EventReadRequest, EventReader, HostEventEmitter,
    InMemoryAgentStore, InMemoryEventLog, InMemoryHarnessStore, InMemoryProviderStore,
    InMemorySessionStore, NoopEventSink, StoreTurnContextResolver,
};
use everruns_provider::driver_registry::ProviderConfig;
use everruns_provider::driver_registry::{DriverId, DriverRegistry};
use everruns_provider::error::Result;
use everruns_provider::model_spec::ModelSpec;
use everruns_provider::tool_types::ToolCall;
use everruns_provider::typed_id::{AgentId, HarnessId, MessageId, SessionId, TurnId};

// ============================================================================
// Turn Result
// ============================================================================

/// Result of executing a turn
#[derive(Debug, Clone)]
pub struct TurnResult {
    /// Final text response from the agent
    pub response: String,
    /// Number of reasoning iterations (Reason → Act cycles)
    pub iterations: usize,
    /// Total tool calls made during the turn
    pub tool_calls_count: usize,
    /// Whether the turn completed successfully
    pub success: bool,
    /// Error message if the turn failed
    pub error: Option<String>,
    /// Structured reason the turn stopped.
    pub stop_reason: TurnStopReason,
    /// Turn ID for this turn
    pub turn_id: TurnId,
    /// Compact request/response evidence for each provider generation in this turn.
    pub llm_generations: Vec<LlmGenerationSummary>,
}

/// Request/response evidence retained by the in-memory harness for one LLM call.
///
/// This deliberately excludes prompts and tool arguments. It is enough for tests
/// to distinguish a model declining an advertised tool from the runtime omitting
/// the tool, or from a provider reporting tool calls that the stream parser lost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmGenerationSummary {
    /// Tool names advertised by the runtime for this generation.
    pub available_tools: Vec<String>,
    /// Tool calls parsed from the provider response.
    pub output_tool_calls_count: usize,
    /// Provider finish reasons, normalized by the driver.
    pub finish_reasons: Vec<String>,
    /// Whether the provider generation completed successfully.
    pub success: bool,
}

impl TurnResult {
    /// Check if the response contains a specific substring
    pub fn contains(&self, text: &str) -> bool {
        self.response.contains(text)
    }

    /// Create a TurnResult from a TurnOutcome and turn_id.
    fn from_outcome(outcome: TurnOutcome, turn_id: TurnId) -> Self {
        let stop_reason = outcome.stop_reason();
        match outcome {
            TurnOutcome::Success {
                response,
                iterations,
                tool_calls_count,
                ..
            } => Self {
                response,
                iterations,
                tool_calls_count,
                success: true,
                error: None,
                stop_reason,
                turn_id,
                llm_generations: vec![],
            },
            TurnOutcome::Failed {
                error, iterations, ..
            } => Self {
                response: String::new(),
                iterations,
                tool_calls_count: 0,
                success: false,
                error: Some(error),
                stop_reason,
                turn_id,
                llm_generations: vec![],
            },
            TurnOutcome::MaxIterationsReached {
                response,
                iterations,
                tool_calls_count,
            } => Self {
                response,
                iterations,
                tool_calls_count,
                success: true, // Max iterations is not a failure
                error: None,
                stop_reason,
                turn_id,
                llm_generations: vec![],
            },
            // A sealed turn was deliberately stopped (EVE-534). Surface it as a
            // non-success with the seal reason so in-memory callers can observe
            // it distinctly from a normal completion.
            TurnOutcome::Sealed {
                reason,
                response,
                iterations,
                tool_calls_count,
            } => Self {
                response,
                iterations,
                tool_calls_count,
                success: false,
                error: Some(format!("turn sealed: {reason}")),
                stop_reason,
                turn_id,
                llm_generations: vec![],
            },
        }
    }
}

// ============================================================================
// Builder
// ============================================================================

/// Credential-free model identity plus optional host-owned provider configuration.
pub struct InMemoryModelConfig {
    model: ModelSpec,
    provider_config: Option<ProviderConfig>,
}

impl InMemoryModelConfig {
    /// Split the credential-free model identity from its optional host-owned configuration.
    pub fn into_parts(self) -> (ModelSpec, Option<ProviderConfig>) {
        (self.model, self.provider_config)
    }
}

impl From<ModelSpec> for InMemoryModelConfig {
    fn from(model: ModelSpec) -> Self {
        Self {
            model,
            provider_config: None,
        }
    }
}

impl From<(ModelSpec, ProviderConfig)> for InMemoryModelConfig {
    fn from((model, provider_config): (ModelSpec, ProviderConfig)) -> Self {
        Self {
            model,
            provider_config: Some(provider_config),
        }
    }
}

/// Builder for creating an `InMemoryAgenticLoop`
pub struct InMemoryAgenticLoopBuilder {
    agent_name: String,
    system_prompt: String,
    model: Option<ModelSpec>,
    provider_config: Option<ProviderConfig>,
    driver_registry: Option<DriverRegistry>,
    llm_sim_config: Option<LlmSimConfig>,
    tools: Vec<Box<dyn Tool>>,
    capabilities: Vec<Box<dyn Capability>>,
    max_iterations: usize,
    parallel_tool_calls: Option<bool>,
    reasoning_effort_handle: Option<everruns_core::tool_context::ReasoningEffortHandle>,
}

impl Default for InMemoryAgenticLoopBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryAgenticLoopBuilder {
    /// Create a new builder with defaults (uses simulated LLM)
    pub fn new() -> Self {
        Self {
            agent_name: "Test Agent".to_string(),
            system_prompt: "You are a helpful assistant.".to_string(),
            model: None,
            provider_config: None,
            driver_registry: None,
            llm_sim_config: Some(LlmSimConfig::default()),
            tools: vec![],
            capabilities: vec![],
            max_iterations: 10,
            parallel_tool_calls: None,
            reasoning_effort_handle: None,
        }
    }

    /// Share a live reasoning-effort handle (EVE-595) across the loop's
    /// `ReasonAtom` and `ActAtom`. Tools receive a clone via their
    /// `ToolContext` and can mutate it mid-turn so subsequent LLM steps in the
    /// same `run_turn` observe the new effort.
    pub fn reasoning_effort_handle(
        mut self,
        handle: everruns_core::tool_context::ReasoningEffortHandle,
    ) -> Self {
        self.reasoning_effort_handle = Some(handle);
        self
    }

    /// Set the agent name
    pub fn agent_name(mut self, name: impl Into<String>) -> Self {
        self.agent_name = name.into();
        self
    }

    /// Set the system prompt
    pub fn system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = prompt.into();
        self
    }

    /// Use a simulated LLM with a fixed response (no real API calls)
    pub fn with_simulated_response(mut self, response: impl Into<String>) -> Self {
        self.llm_sim_config = Some(LlmSimConfig::fixed(response));
        self.model = None;
        self.driver_registry = None;
        self
    }

    /// Use a simulated LLM with custom configuration
    pub fn with_llm_sim(mut self, config: LlmSimConfig) -> Self {
        self.llm_sim_config = Some(config);
        self.model = None;
        self.driver_registry = None;
        self
    }

    /// Set the LLM model to use
    ///
    /// # Example
    ///
    /// ```ignore
    /// use everruns_core::{DriverId, ModelSpec, ProviderConfig};
    ///
    /// let model = ModelSpec::on("anthropic", "claude-sonnet-4-20250514");
    /// let config = ProviderConfig::new(DriverId::Anthropic)
    ///     .with_api_key(std::env::var("ANTHROPIC_API_KEY").unwrap());
    ///
    /// let runner = InMemoryAgenticLoop::builder()
    ///     .model(model)
    ///     .driver_registry(driver_registry)
    ///     .build()
    ///     .await?;
    /// ```
    pub fn model(mut self, config: impl Into<InMemoryModelConfig>) -> Self {
        let config = config.into();
        self.model = Some(config.model);
        self.provider_config = config.provider_config;
        self.llm_sim_config = None;
        self
    }

    /// Set credentials and endpoint configuration for the selected provider.
    pub fn provider_config(mut self, config: ProviderConfig) -> Self {
        self.provider_config = Some(config);
        self
    }

    /// Set the driver registry for LLM providers
    ///
    /// # Example
    ///
    /// ```ignore
    /// use everruns_provider::driver_registry::DriverRegistry;
    ///
    /// let mut driver_registry = DriverRegistry::new();
    /// everruns_anthropic::register_driver(&mut driver_registry);
    ///
    /// let runner = InMemoryAgenticLoop::builder()
    ///     .model(model)
    ///     .driver_registry(driver_registry)
    ///     .build()
    ///     .await?;
    /// ```
    pub fn driver_registry(mut self, driver_registry: DriverRegistry) -> Self {
        self.driver_registry = Some(driver_registry);
        self.llm_sim_config = None;
        self
    }

    /// Add a tool
    pub fn tool<T: Tool + 'static>(mut self, tool: T) -> Self {
        self.tools.push(Box::new(tool));
        self
    }

    /// Add a capability (which may provide tools and system prompt additions)
    ///
    /// Capabilities provide a way to bundle related tools and functionality.
    /// For example, the `current_time` capability provides a `get_current_time` tool.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use everruns_builtins::CurrentTimeCapability;
    ///
    /// let runner = InMemoryAgenticLoop::builder()
    ///     .capability(CurrentTimeCapability)
    ///     .build()
    ///     .await?;
    /// ```
    pub fn capability<C: Capability + 'static>(mut self, capability: C) -> Self {
        self.capabilities.push(Box::new(capability));
        self
    }

    /// Set maximum iterations per turn
    pub fn max_iterations(mut self, max: usize) -> Self {
        self.max_iterations = max;
        self
    }

    /// Set the request-level parallel tool calling preference (EVE-598).
    ///
    /// `Some(true)` signals the provider that parallel tool calls are wanted;
    /// `Some(false)` requests at most one tool call per turn and forces serial
    /// execution. `None` (default) preserves provider defaults.
    pub fn parallel_tool_calls(mut self, parallel_tool_calls: Option<bool>) -> Self {
        self.parallel_tool_calls = parallel_tool_calls;
        self
    }

    /// Build the agentic loop
    pub async fn build(self) -> Result<InMemoryAgenticLoop> {
        // Create stores
        let harness_store = InMemoryHarnessStore::new();
        let agent_store = InMemoryAgentStore::new();
        let session_store = InMemorySessionStore::new();
        let event_log = Arc::new(InMemoryEventLog::new());
        let message_retriever = EventHistory::new(event_log.clone());
        let event_emitter = HostEventEmitter::new(event_log.clone(), Arc::new(NoopEventSink));

        // Build capability configs for the agent from capabilities
        let agent_capability_configs: Vec<AgentCapabilityConfig> = self
            .capabilities
            .iter()
            .map(|cap| AgentCapabilityConfig::new(cap.id()))
            .collect();

        // Create harness (portable execution configuration keyed by id; the
        // stored persistence record lives in everruns-platform, EVE-881).
        let harness_id = HarnessId::new();
        let harness =
            everruns_core::HarnessDefinition::new("in-memory", self.system_prompt.clone());
        harness_store.add_harness(harness_id, harness).await;

        // Surface explicitly-added tools (via `.tool(...)`) as agent tool
        // definitions so ReasonAtom returns them and ActAtom executes them
        // (rather than treating them as unknown). Capability-provided tools are
        // already surfaced through the capability registry.
        let explicit_tool_definitions: Vec<everruns_provider::tool_types::ToolDefinition> =
            self.tools.iter().map(|tool| tool.to_definition()).collect();

        // Create agent
        let agent_id = AgentId::new();
        let agent = AgentDefinition {
            display_name: Some(self.agent_name),
            capabilities: agent_capability_configs,
            parallel_tool_calls: self.parallel_tool_calls,
            tools: explicit_tool_definitions,
            ..AgentDefinition::new(agent_id, "in-memory", self.system_prompt)
        };
        agent_store.add_agent(agent).await;

        // Create session
        let session_id = SessionId::new();
        let session = ExecutionSession {
            agent_id: Some(agent_id),
            title: Some("In-Memory Session".to_string()),
            ..ExecutionSession::with_own_workspace(session_id, harness_id)
        };
        session_store.add_session(session).await;

        // Capture the configured model name before the if-let consumes self.model.
        // Used below to resolve model-adaptive capabilities against the right variant.
        let configured_model = self.model.as_ref().map(|m| m.model.clone());

        // Create provider store and driver registry
        let provider_store = InMemoryProviderStore::new();
        if let Some(config) = self.provider_config {
            provider_store.set_provider_config(config).await;
        }
        let driver_registry =
            if let (Some(model), Some(registry)) = (self.model, self.driver_registry) {
                // Use provided model and driver registry
                provider_store.set_default_model_spec(model).await;
                registry
            } else {
                // Use LlmSim (default or explicitly configured)
                let config = self.llm_sim_config.unwrap_or_default();
                let model = ModelSpec::on((DriverId::LlmSim).as_str(), "llmsim-model".to_string());
                provider_store.set_default_model_spec(model).await;

                // Create the driver once and share it across calls.
                // This ensures sequence-based responses work correctly
                // because the Arc counters are shared.
                let driver = LlmSimDriver::new(config);
                let mut registry = DriverRegistry::new();
                registry.register(DriverId::LlmSim, move |_config| Box::new(driver.clone()));
                registry
            };

        // Build tool registry - include tools from capabilities. Resolve each
        // capability against the configured model so model-adaptive capabilities
        // (e.g. auto_tool_search) contribute the right variant for this harness.
        let configured_model_ref = configured_model.as_deref();
        let mut tool_builder = ToolRegistryBuilder::new();
        for capability in &self.capabilities {
            let effective: &dyn everruns_core::Capability = capability
                .resolve_for_model(configured_model_ref)
                .unwrap_or_else(|| capability.as_ref());
            for tool in effective.tools() {
                tool_builder = tool_builder.tool_boxed(tool);
            }
        }

        // Add explicit tools (can override capability tools)
        for tool in self.tools {
            tool_builder = tool_builder.tool_boxed(tool);
        }
        let tool_registry = tool_builder.build();

        // Create capability registry with added capabilities
        let mut capability_registry = CapabilityRegistry::new();
        for capability in self.capabilities {
            capability_registry.register_boxed(capability);
        }

        let input_atom = InputAtom::new(message_retriever.clone());
        let context_resolver = StoreTurnContextResolver::new(
            Arc::new(harness_store.clone()),
            Arc::new(agent_store.clone()),
            Arc::new(session_store.clone()),
            Arc::new(message_retriever.clone()),
            Arc::new(provider_store.clone()),
            capability_registry.clone(),
            driver_registry,
        );
        let mut reason_atom = ReasonAtom::new(
            context_resolver,
            message_retriever.clone(),
            capability_registry,
            event_emitter.clone(),
        );
        let mut act_atom = ActAtom::new(tool_registry.clone(), event_emitter.clone())
            .with_tool_registry(Arc::new(tool_registry.clone()));
        if let Some(handle) = &self.reasoning_effort_handle {
            reason_atom = reason_atom.with_reasoning_effort_handle(handle.clone());
            act_atom = act_atom.with_reasoning_effort_handle(handle.clone());
        }

        Ok(InMemoryAgenticLoop {
            harness_id,
            agent_id,
            session_id,
            harness_store,
            agent_store,
            session_store,
            event_log,
            message_retriever,
            provider_store,
            event_emitter,
            tool_registry,
            input_atom: Arc::new(input_atom),
            reason_atom: Arc::new(reason_atom),
            act_atom: Arc::new(act_atom),
            max_iterations: self.max_iterations,
            reasoning_effort_handle: self.reasoning_effort_handle,
        })
    }
}

// ============================================================================
// InMemoryAgenticLoop
// ============================================================================

/// In-memory agentic loop for testing and prototyping
///
/// Bundles all in-memory stores and atoms into a convenient interface
/// for running agent turns without external dependencies.
///
/// # Example
///
/// ```ignore
/// use everruns_test_support::in_memory_loop::InMemoryAgenticLoop;
///
/// // Simple usage with simulated LLM
/// let mut loop_runner = InMemoryAgenticLoop::builder()
///     .system_prompt("You are a helpful assistant.")
///     .with_simulated_response("Hello! I can help you with that.")
///     .build()
///     .await?;
///
/// let result = loop_runner.run_turn("Hi there!").await?;
/// assert!(result.success);
/// println!("Response: {}", result.response);
///
/// // With real LLM (requires API key)
/// let mut loop_runner = InMemoryAgenticLoop::builder()
///     .with_real_llm()
///     .tool(MyCustomTool)
///     .build()
///     .await?;
/// ```
pub struct InMemoryAgenticLoop {
    harness_id: HarnessId,
    agent_id: AgentId,
    session_id: SessionId,
    #[allow(dead_code)]
    harness_store: InMemoryHarnessStore,
    #[allow(dead_code)]
    agent_store: InMemoryAgentStore,
    #[allow(dead_code)]
    session_store: InMemorySessionStore,
    event_log: Arc<InMemoryEventLog>,
    message_retriever: EventHistory,
    #[allow(dead_code)]
    provider_store: InMemoryProviderStore,
    event_emitter: HostEventEmitter,
    tool_registry: ToolRegistry,
    input_atom: Arc<InputAtom<EventHistory>>,
    reason_atom: Arc<ReasonAtom>,
    act_atom: Arc<ActAtom<ToolRegistry, HostEventEmitter>>,
    max_iterations: usize,
    reasoning_effort_handle: Option<everruns_core::tool_context::ReasoningEffortHandle>,
}

impl InMemoryAgenticLoop {
    /// Create a new builder
    pub fn builder() -> InMemoryAgenticLoopBuilder {
        InMemoryAgenticLoopBuilder::new()
    }

    /// Get the agent ID
    pub fn agent_id(&self) -> AgentId {
        self.agent_id
    }

    /// Get the session ID
    pub fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// Run a turn with the given user input
    ///
    /// Accepts either a string or an `InputMessage` for full control over
    /// message options like reasoning effort.
    ///
    /// This executes the full agentic loop using the TurnStateMachine:
    /// 1. Add user message
    /// 2. Record input (InputAtom)
    /// 3. Reason loop (ReasonAtom → ActAtom → repeat until done)
    ///
    /// The TurnStateMachine ensures consistent orchestration logic,
    /// proper error handling (checking success flag), and turn ID management.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Simple string input
    /// let result = runner.run_turn("Hello").await?;
    ///
    /// // Full InputMessage with controls
    /// let input = InputMessage {
    ///     role: MessageRole::User,
    ///     content: vec![ContentPart::text("What is 2+2?")],
    ///     controls: Some(Controls {
    ///         model_id: None,
    ///         reasoning: Some(ReasoningConfig { effort: Some("medium".into()) }),
    ///     }),
    ///     metadata: None,
    ///     tags: vec![],
    /// };
    /// let result = runner.run_turn(input).await?;
    /// ```
    pub async fn run_turn(&self, input: impl Into<InputMessage>) -> Result<TurnResult> {
        // The live effort override is turn-scoped: tools may set it for later
        // LLM steps in this turn, but stale values must not override the next
        // turn's message controls.
        if let Some(handle) = &self.reasoning_effort_handle {
            handle.set(None);
        }

        // Append the accepted input once. EventHistory supplies the read model
        // consumed by every atom; there is no writable message-store facade.
        let input = input.into();
        let tags = input.tags.clone();
        let message = Message {
            id: MessageId::new(),
            role: input.role,
            content: input.content,
            phase: None,
            thinking: None,
            thinking_signature: None,
            controls: input.controls,
            metadata: input.metadata,
            external_actor: None,
            created_at: Utc::now(),
        };
        let mut request = EventRequest::new(
            self.session_id,
            EventContext::empty(),
            InputMessageData::new(message.clone()),
        );
        if !tags.is_empty() {
            request = request.with_tags(tags);
        }
        self.event_emitter.emit(request).await?;

        // Create turn context and state machine
        let turn_context = TurnContext::new(self.session_id, message.id, self.agent_id, 0);
        let mut state_machine = TurnStateMachine::new(turn_context, self.max_iterations);

        // Track last reason result for ActAtom
        let mut last_reason_result: Option<ReasonResult> = None;
        // Track response_id from last reason call for chaining
        let mut previous_response_id: Option<String> = None;

        // Execute the turn using the state machine
        loop {
            match state_machine.next_action() {
                TurnAction::ExecuteInput => {
                    let base_context = ExecutionContext::new(
                        state_machine.context().session_id,
                        state_machine.context().turn_id,
                        state_machine.context().input_message_id,
                    );
                    self.input_atom
                        .execute(InputAtomInput {
                            context: base_context,
                        })
                        .await?;
                    state_machine.on_input_completed();
                }

                TurnAction::ExecuteReason => {
                    let base_context = ExecutionContext::new(
                        state_machine.context().session_id,
                        state_machine.context().turn_id,
                        state_machine.context().input_message_id,
                    );
                    let reason_result = self
                        .reason_atom
                        .execute(ReasonInput {
                            context: base_context.next_exec(),
                            harness_id: self.harness_id,
                            agent_id: Some(self.agent_id),
                            org_id: 0,
                            mcp_tool_definitions: vec![],
                            previous_response_id: previous_response_id.take(),
                            iteration: state_machine.current_iteration() as u32 + 1,
                        })
                        .await?;

                    let tool_call_count = reason_result.tool_calls.len();
                    previous_response_id = reason_result.response_id.clone();
                    // In-memory loop has no signal mechanism, so
                    // has_pending_user_messages is always false.
                    state_machine.on_reason_completed(
                        reason_result.text.clone(),
                        tool_call_count,
                        reason_result.success,
                        reason_result.error.clone(),
                        reason_result.finish_reason.clone(),
                        false,
                    );

                    // Store for ActAtom if needed
                    if reason_result.has_tool_calls {
                        last_reason_result = Some(reason_result);
                    }
                }

                TurnAction::ExecuteAct => {
                    let reason_result = last_reason_result
                        .take()
                        .expect("ExecuteAct requires prior ReasonResult with tool calls");
                    let base_context = ExecutionContext::new(
                        state_machine.context().session_id,
                        state_machine.context().turn_id,
                        state_machine.context().input_message_id,
                    );
                    self.act_atom
                        .execute(ActInput {
                            org_id: Some(0),
                            context: base_context.next_exec(),
                            harness_id: self.harness_id,
                            agent_id: Some(self.agent_id),
                            tool_calls: reason_result.tool_calls,
                            tool_definitions: reason_result.tool_definitions,
                            locale: reason_result.locale,
                            blueprint_id: None,
                            network_access: reason_result.network_access,
                            // Request-level parallel tool calling preference,
                            // carried from agent config through reason (EVE-598).
                            parallel_tool_calls: reason_result.parallel_tool_calls,
                        })
                        .await?;
                    state_machine.on_act_completed();
                }

                TurnAction::Complete(outcome) => {
                    let turn_id = state_machine.context().turn_id;
                    let mut result = TurnResult::from_outcome(outcome, turn_id);
                    result.llm_generations = self
                        .events()
                        .await
                        .into_iter()
                        .filter(|event| event.context.turn_id == Some(turn_id))
                        .filter_map(|event| {
                            let EventData::LlmGeneration(data) = event.data else {
                                return None;
                            };
                            Some(LlmGenerationSummary {
                                available_tools: data
                                    .tools
                                    .into_iter()
                                    .map(|tool| tool.name)
                                    .collect(),
                                output_tool_calls_count: data.output.tool_calls.len(),
                                finish_reasons: data.metadata.finish_reasons.unwrap_or_default(),
                                success: data.metadata.success,
                            })
                        })
                        .collect();
                    return Ok(result);
                }
            }
        }
    }

    /// Run multiple turns in sequence
    pub async fn run_conversation(&self, messages: &[&str]) -> Result<Vec<TurnResult>> {
        let mut results = Vec::with_capacity(messages.len());
        for msg in messages {
            results.push(self.run_turn(*msg).await?);
        }
        Ok(results)
    }

    /// Get all messages in the session
    pub async fn messages(&self) -> Result<Vec<Message>> {
        self.message_retriever.load(self.session_id).await
    }

    /// Get all emitted events
    pub async fn events(&self) -> Vec<Event> {
        let limit = EventReadLimit::default();
        let mut request = EventReadRequest::new(self.session_id, limit);
        let mut events = Vec::new();
        loop {
            let page = self
                .event_log
                .read_page(request)
                .await
                .expect("in-memory canonical event reads are infallible");
            events.extend(page.events);
            let Some(cursor) = page.next_cursor else {
                return events;
            };
            request = EventReadRequest::from_cursor(cursor, limit);
        }
    }

    /// Get events of a specific type
    pub async fn events_by_type(&self, event_type: &str) -> Vec<Event> {
        self.events()
            .await
            .into_iter()
            .filter(|event| event.event_type == event_type)
            .collect()
    }

    /// Get the count of messages
    pub async fn message_count(&self) -> Result<usize> {
        self.message_retriever.count(self.session_id).await
    }

    /// Get the count of events
    pub async fn event_count(&self) -> usize {
        self.events().await.len()
    }

    /// Get conversation as a formatted string
    pub async fn conversation_string(&self) -> Result<String> {
        let messages = self.messages().await?;
        let mut result = String::new();
        for msg in messages {
            let role = format!("{:?}", msg.role);
            let text = msg.text().unwrap_or("[non-text content]");
            result.push_str(&format!("[{}] {}\n", role, text));
        }
        Ok(result)
    }

    /// Access the message retriever directly
    pub fn message_retriever(&self) -> &EventHistory {
        &self.message_retriever
    }

    /// Replay pre-recorded conversation envelopes into this session's log.
    ///
    /// Conversation state is event-sourced and [`EventHistory`] is a read
    /// model, so fixtures that need a prior conversation cannot write messages
    /// directly. They emit the envelopes through the same emitter the loop
    /// uses, which makes the projected history identical to live traffic.
    /// Only [`EventData`] variants that project to messages contribute.
    pub async fn seed_events(&self, events: impl IntoIterator<Item = EventData>) -> Result<()> {
        for data in events {
            self.event_emitter
                .emit(EventRequest::new(
                    self.session_id,
                    EventContext::empty(),
                    data,
                ))
                .await?;
        }
        Ok(())
    }

    /// Access the tool registry directly
    pub fn tool_registry(&self) -> &ToolRegistry {
        &self.tool_registry
    }
}

// ============================================================================
// Quick constructors
// ============================================================================

impl InMemoryAgenticLoop {
    /// Create a simple loop with a fixed simulated response
    ///
    /// # Example
    ///
    /// ```ignore
    /// let runner = InMemoryAgenticLoop::with_fixed_response("Hello!").await?;
    /// let result = runner.run_turn("Hi").await?;
    /// assert_eq!(result.response, "Hello!");
    /// ```
    pub async fn with_fixed_response(response: impl Into<String>) -> Result<Self> {
        Self::builder()
            .with_simulated_response(response)
            .build()
            .await
    }

    /// Create a loop that echoes user input
    ///
    /// # Example
    ///
    /// ```ignore
    /// let runner = InMemoryAgenticLoop::with_echo().await?;
    /// let result = runner.run_turn("Hello").await?;
    /// assert!(result.response.contains("Hello"));
    /// ```
    pub async fn with_echo() -> Result<Self> {
        Self::builder()
            .with_llm_sim(LlmSimConfig::echo())
            .build()
            .await
    }

    /// Create a loop with sequence of responses
    ///
    /// # Example
    ///
    /// ```ignore
    /// let runner = InMemoryAgenticLoop::with_sequence(vec![
    ///     "First response",
    ///     "Second response",
    /// ]).await?;
    ///
    /// let r1 = runner.run_turn("msg1").await?;
    /// let r2 = runner.run_turn("msg2").await?;
    /// assert_eq!(r1.response, "First response");
    /// assert_eq!(r2.response, "Second response");
    /// ```
    pub async fn with_sequence(responses: Vec<impl Into<String>>) -> Result<Self> {
        let responses: Vec<String> = responses.into_iter().map(|s| s.into()).collect();
        Self::builder()
            .with_llm_sim(LlmSimConfig::sequence(responses))
            .build()
            .await
    }

    /// Create a loop with tool call simulation
    ///
    /// # Example
    ///
    /// ```ignore
    /// use everruns_provider::tool_types::ToolCall;
    /// use serde_json::json;
    ///
    /// let tool_call = ToolCall {
    ///     id: "call_1".to_string(),
    ///     name: "get_weather".to_string(),
    ///     arguments: json!({"city": "NYC"}),
    /// };
    ///
    /// let runner = InMemoryAgenticLoop::with_tool_calls(
    ///     "Let me check that.",
    ///     vec![tool_call],
    /// ).await?;
    /// ```
    pub async fn with_tool_calls(
        response: impl Into<String>,
        tool_calls: Vec<ToolCall>,
    ) -> Result<Self> {
        Self::builder()
            .with_llm_sim(LlmSimConfig::fixed(response).with_tool_calls(tool_calls))
            .build()
            .await
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_simple_turn() {
        let runner = InMemoryAgenticLoop::with_fixed_response("Hello from the assistant!")
            .await
            .unwrap();

        let result = runner.run_turn("Hi there").await.unwrap();

        assert!(result.success);
        assert_eq!(result.response, "Hello from the assistant!");
        assert_eq!(result.iterations, 1);
        assert_eq!(result.tool_calls_count, 0);
    }

    #[tokio::test]
    async fn test_echo_turn() {
        let runner = InMemoryAgenticLoop::with_echo().await.unwrap();

        let result = runner.run_turn("Test message").await.unwrap();

        assert!(result.success);
        assert!(result.response.contains("Test message"));
    }

    #[tokio::test]
    async fn test_sequence_turns() {
        let runner = InMemoryAgenticLoop::with_sequence(vec!["First", "Second", "Third"])
            .await
            .unwrap();

        let r1 = runner.run_turn("msg1").await.unwrap();
        let r2 = runner.run_turn("msg2").await.unwrap();
        let r3 = runner.run_turn("msg3").await.unwrap();

        assert_eq!(r1.response, "First");
        assert_eq!(r2.response, "Second");
        assert_eq!(r3.response, "Third");
    }

    #[tokio::test]
    async fn test_conversation() {
        let runner = InMemoryAgenticLoop::with_sequence(vec!["Hello!", "How can I help?"])
            .await
            .unwrap();

        let results = runner
            .run_conversation(&["Hi", "I need help"])
            .await
            .unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].response, "Hello!");
        assert_eq!(results[1].response, "How can I help?");

        // Check message history
        let messages = runner.messages().await.unwrap();
        assert_eq!(messages.len(), 4); // 2 user + 2 assistant
    }

    #[tokio::test]
    async fn test_events_captured() {
        let runner = InMemoryAgenticLoop::with_fixed_response("Response")
            .await
            .unwrap();

        runner.run_turn("Test").await.unwrap();

        let events = runner.events().await;
        assert!(!events.is_empty());

        // Should have reason.* events (input.message is emitted by API layer, not InputAtom)
        let reason_events = runner.events_by_type("reason.started").await;
        assert_eq!(reason_events.len(), 1);
    }

    #[tokio::test]
    async fn turn_result_summarizes_llm_generation_contract() {
        use everruns_builtins::CurrentTimeCapability;

        let runner = InMemoryAgenticLoop::builder()
            .with_simulated_response("It is noon.")
            .capability(CurrentTimeCapability)
            .build()
            .await
            .unwrap();

        let result = runner.run_turn("What time is it?").await.unwrap();

        assert_eq!(result.llm_generations.len(), 1);
        assert_eq!(
            result.llm_generations[0].available_tools,
            ["get_current_time"]
        );
        assert_eq!(result.llm_generations[0].output_tool_calls_count, 0);
        assert_eq!(result.llm_generations[0].finish_reasons, ["stop"]);
        assert!(result.llm_generations[0].success);
    }

    #[tokio::test]
    async fn conversation_messages_are_projected_from_single_canonical_writes() {
        let runner = InMemoryAgenticLoop::with_fixed_response("Response")
            .await
            .unwrap();

        runner.run_turn("Question").await.unwrap();

        let messages = runner.messages().await.unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].text(), Some("Question"));
        assert_eq!(messages[1].text(), Some("Response"));

        let events = runner.events().await;
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event_type == everruns_core::events::INPUT_MESSAGE)
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event_type == everruns_core::events::OUTPUT_MESSAGE_COMPLETED)
                .count(),
            1
        );
        assert!(events.iter().all(|event| event.sequence.is_some()));
    }

    #[tokio::test]
    async fn test_builder_with_custom_config() {
        let runner = InMemoryAgenticLoop::builder()
            .agent_name("Custom Agent")
            .system_prompt("You are a custom assistant.")
            .with_simulated_response("Custom response")
            .max_iterations(5)
            .build()
            .await
            .unwrap();

        let result = runner.run_turn("Test").await.unwrap();
        assert_eq!(result.response, "Custom response");
    }

    #[tokio::test]
    async fn test_conversation_string() {
        let runner = InMemoryAgenticLoop::with_fixed_response("Hello!")
            .await
            .unwrap();

        runner.run_turn("Hi").await.unwrap();

        let conv = runner.conversation_string().await.unwrap();
        assert!(conv.contains("[User]"));
        assert!(conv.contains("[Agent]"));
        assert!(conv.contains("Hi"));
        assert!(conv.contains("Hello!"));
    }
}
