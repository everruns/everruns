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

use async_trait::async_trait;
use chrono::Utc;

use crate::agent::{Agent, AgentStatus};
use crate::atoms::{
    ActAtom, ActInput, Atom, AtomContext, InputAtom, InputAtomInput, ReasonAtom, ReasonInput,
};
use crate::capabilities::{AgentCapabilityConfig, Capability, CapabilityRegistry};
use crate::error::Result;
use crate::events::{Event, EventData, EventRequest, OUTPUT_MESSAGE_COMPLETED};
use crate::llm_driver_registry::{DriverRegistry, ProviderType};
use crate::llm_models::LlmProviderType;
use crate::llmsim_driver::{LlmSimConfig, LlmSimDriver};
use crate::memory::{
    InMemoryAgentStore, InMemoryEventEmitter, InMemoryHarnessStore, InMemoryLlmProviderStore,
    InMemoryMessageRetriever, InMemorySessionStore,
};
use crate::message::Message;
use crate::message_retriever::{InputMessage, MessageRetriever};
use crate::session::{Session, SessionStatus};
use crate::tool_types::ToolCall;
use crate::tools::{Tool, ToolRegistry, ToolRegistryBuilder};
use crate::traits::{EventEmitter, ModelWithProvider};
use crate::turn::{TurnAction, TurnContext, TurnOutcome, TurnStateMachine};
use crate::typed_id::{AgentId, HarnessId, SessionId, TurnId};

// ============================================================================
// Bridging Event Emitter
// ============================================================================

/// Event emitter that bridges events to message storage
///
/// When a `output.message.completed` event is emitted, it also stores the message
/// in the provided message retriever. This enables full agentic loops
/// in memory without the database layer.
#[derive(Clone)]
struct BridgingEventEmitter {
    inner: InMemoryEventEmitter,
    message_retriever: InMemoryMessageRetriever,
}

impl BridgingEventEmitter {
    fn new(message_retriever: InMemoryMessageRetriever) -> Self {
        Self {
            inner: InMemoryEventEmitter::new(),
            message_retriever,
        }
    }

    async fn events(&self) -> Vec<Event> {
        self.inner.events().await
    }

    async fn events_by_type(&self, event_type: &str) -> Vec<Event> {
        self.inner.events_by_type(event_type).await
    }

    async fn event_count(&self) -> usize {
        self.inner.event_count().await
    }

    async fn clear(&self) {
        self.inner.clear().await;
    }
}

#[async_trait]
impl EventEmitter for BridgingEventEmitter {
    async fn emit(&self, request: EventRequest) -> Result<Event> {
        // If this is an output.message.completed event, also store the message
        if request.data.event_type() == OUTPUT_MESSAGE_COMPLETED
            && let EventData::OutputMessageCompleted(data) = &request.data
        {
            // Store the message in the retriever
            let _ = self
                .message_retriever
                .store(request.session_id, data.message.clone())
                .await;
        }

        // Delegate to the inner emitter
        self.inner.emit(request).await
    }
}

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
    /// Turn ID for this turn
    pub turn_id: TurnId,
}

impl TurnResult {
    /// Check if the response contains a specific substring
    pub fn contains(&self, text: &str) -> bool {
        self.response.contains(text)
    }

    /// Create a TurnResult from a TurnOutcome and turn_id.
    fn from_outcome(outcome: TurnOutcome, turn_id: TurnId) -> Self {
        match outcome {
            TurnOutcome::Success {
                response,
                iterations,
                tool_calls_count,
            } => Self {
                response,
                iterations,
                tool_calls_count,
                success: true,
                error: None,
                turn_id,
            },
            TurnOutcome::Failed { error, iterations } => Self {
                response: String::new(),
                iterations,
                tool_calls_count: 0,
                success: false,
                error: Some(error),
                turn_id,
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
                turn_id,
            },
        }
    }
}

// ============================================================================
// Builder
// ============================================================================

/// Builder for creating an `InMemoryAgenticLoop`
pub struct InMemoryAgenticLoopBuilder {
    agent_name: String,
    system_prompt: String,
    model: Option<ModelWithProvider>,
    driver_registry: Option<DriverRegistry>,
    llm_sim_config: Option<LlmSimConfig>,
    tools: Vec<Box<dyn Tool>>,
    capabilities: Vec<Box<dyn Capability>>,
    max_iterations: usize,
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
            driver_registry: None,
            llm_sim_config: Some(LlmSimConfig::default()),
            tools: vec![],
            capabilities: vec![],
            max_iterations: 10,
        }
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
    /// use everruns_core::traits::ModelWithProvider;
    /// use everruns_core::llm_models::LlmProviderType;
    ///
    /// let model = ModelWithProvider {
    ///     model: "claude-sonnet-4-20250514".to_string(),
    ///     provider_type: LlmProviderType::Anthropic,
    ///     api_key: Some(std::env::var("ANTHROPIC_API_KEY").unwrap()),
    ///     base_url: None,
    /// };
    ///
    /// let runner = InMemoryAgenticLoop::builder()
    ///     .model(model)
    ///     .driver_registry(driver_registry)
    ///     .build()
    ///     .await?;
    /// ```
    pub fn model(mut self, model: ModelWithProvider) -> Self {
        self.model = Some(model);
        self.llm_sim_config = None;
        self
    }

    /// Set the driver registry for LLM providers
    ///
    /// # Example
    ///
    /// ```ignore
    /// use everruns_core::llm_driver_registry::DriverRegistry;
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
    /// use everruns_core::capabilities::current_time::CurrentTimeCapability;
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

    /// Build the agentic loop
    pub async fn build(self) -> Result<InMemoryAgenticLoop> {
        // Create stores
        let harness_store = InMemoryHarnessStore::new();
        let agent_store = InMemoryAgentStore::new();
        let session_store = InMemorySessionStore::new();
        let message_retriever = InMemoryMessageRetriever::new();
        let event_emitter = BridgingEventEmitter::new(message_retriever.clone());

        // Build capability configs for the agent from capabilities
        let agent_capability_configs: Vec<AgentCapabilityConfig> = self
            .capabilities
            .iter()
            .map(|cap| AgentCapabilityConfig::new(cap.id()))
            .collect();

        // Create harness
        let harness_id = HarnessId::new();
        let now = Utc::now();
        let harness = crate::harness::Harness {
            id: harness_id,
            name: "In-Memory Harness".to_string(),
            description: None,
            system_prompt: self.system_prompt.clone(),
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            status: crate::harness::HarnessStatus::Active,
            created_at: now,
            updated_at: now,
        };
        harness_store.add_harness(harness).await;

        // Create agent
        let agent_id = AgentId::new();
        let agent = Agent {
            public_id: agent_id,
            internal_id: agent_id.uuid(),
            name: self.agent_name,
            description: None,
            system_prompt: self.system_prompt,
            default_model_id: None,
            tags: vec![],
            capabilities: agent_capability_configs,
            tools: vec![],
            status: AgentStatus::Active,
            created_at: now,
            updated_at: now,
            usage: None,
        };
        agent_store.add_agent(agent).await;

        // Create session
        let session_id = SessionId::new();
        let session = Session {
            id: session_id,
            organization_id: crate::DEFAULT_ORG_PUBLIC_ID.to_string(),
            harness_id,
            agent_id: Some(agent_id),
            title: Some("In-Memory Session".to_string()),
            preview: None,
            output_preview: None,
            tags: vec![],
            model_id: None,
            capabilities: vec![],
            tools: vec![],
            status: SessionStatus::Started,
            created_at: now,
            updated_at: now,
            started_at: None,
            finished_at: None,
            usage: None,
        };
        session_store.add_session(session).await;

        // Create provider store and driver registry
        let provider_store = InMemoryLlmProviderStore::new();
        let driver_registry =
            if let (Some(model), Some(registry)) = (self.model, self.driver_registry) {
                // Use provided model and driver registry
                provider_store.set_default_model(model).await;
                registry
            } else {
                // Use LlmSim (default or explicitly configured)
                let config = self.llm_sim_config.unwrap_or_default();
                let model = ModelWithProvider {
                    model: "llmsim-model".to_string(),
                    provider_type: LlmProviderType::LlmSim,
                    api_key: Some("fake-key".to_string()),
                    base_url: None,
                };
                provider_store.set_default_model(model).await;

                // Create the driver once and share it across calls.
                // This ensures sequence-based responses work correctly
                // because the Arc counters are shared.
                let driver = LlmSimDriver::new(config);
                let mut registry = DriverRegistry::new();
                registry.register(ProviderType::LlmSim, move |_api_key, _base_url| {
                    Box::new(driver.clone())
                });
                registry
            };

        // Build tool registry - include tools from capabilities
        let mut tool_builder = ToolRegistryBuilder::new();

        // Add tools from capabilities first
        for capability in &self.capabilities {
            for tool in capability.tools() {
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
        let reason_atom = ReasonAtom::new(
            harness_store.clone(),
            agent_store.clone(),
            session_store.clone(),
            message_retriever.clone(),
            provider_store.clone(),
            capability_registry,
            driver_registry,
            event_emitter.clone(),
        );
        let act_atom = ActAtom::new(tool_registry.clone(), event_emitter.clone());

        Ok(InMemoryAgenticLoop {
            harness_id,
            agent_id,
            session_id,
            harness_store,
            agent_store,
            session_store,
            message_retriever,
            provider_store,
            event_emitter,
            tool_registry,
            input_atom: Arc::new(input_atom),
            reason_atom: Arc::new(reason_atom),
            act_atom: Arc::new(act_atom),
            max_iterations: self.max_iterations,
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
/// use everruns_core::in_memory_loop::InMemoryAgenticLoop;
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
    message_retriever: InMemoryMessageRetriever,
    #[allow(dead_code)]
    provider_store: InMemoryLlmProviderStore,
    event_emitter: BridgingEventEmitter,
    tool_registry: ToolRegistry,
    input_atom: Arc<InputAtom<InMemoryMessageRetriever>>,
    reason_atom: Arc<
        ReasonAtom<
            InMemoryHarnessStore,
            InMemoryAgentStore,
            InMemorySessionStore,
            InMemoryMessageRetriever,
            InMemoryLlmProviderStore,
            BridgingEventEmitter,
        >,
    >,
    act_atom: Arc<ActAtom<ToolRegistry, BridgingEventEmitter>>,
    max_iterations: usize,
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
        // Add user message
        let message = self
            .message_retriever
            .add(self.session_id, input.into())
            .await?;

        // Create turn context and state machine
        let turn_context = TurnContext::new(self.session_id, message.id, self.agent_id, 0);
        let mut state_machine = TurnStateMachine::new(turn_context, self.max_iterations);

        // Track last reason result for ActAtom
        let mut last_reason_result: Option<crate::atoms::ReasonResult> = None;

        // Execute the turn using the state machine
        loop {
            match state_machine.next_action() {
                TurnAction::ExecuteInput => {
                    let base_context = AtomContext::new(
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
                    let base_context = AtomContext::new(
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
                        })
                        .await?;

                    let tool_call_count = reason_result.tool_calls.len();
                    state_machine.on_reason_completed(
                        reason_result.text.clone(),
                        reason_result.has_tool_calls,
                        tool_call_count,
                        reason_result.success,
                        reason_result.error.clone(),
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
                    let base_context = AtomContext::new(
                        state_machine.context().session_id,
                        state_machine.context().turn_id,
                        state_machine.context().input_message_id,
                    );
                    self.act_atom
                        .execute(ActInput {
                            context: base_context.next_exec(),
                            harness_id: self.harness_id,
                            agent_id: Some(self.agent_id),
                            tool_calls: reason_result.tool_calls,
                            tool_definitions: reason_result.tool_definitions,
                        })
                        .await?;
                    state_machine.on_act_completed();
                }

                TurnAction::Complete(outcome) => {
                    return Ok(TurnResult::from_outcome(
                        outcome,
                        state_machine.context().turn_id,
                    ));
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
        self.event_emitter.events().await
    }

    /// Get events of a specific type
    pub async fn events_by_type(&self, event_type: &str) -> Vec<Event> {
        self.event_emitter.events_by_type(event_type).await
    }

    /// Get the count of messages
    pub async fn message_count(&self) -> Result<usize> {
        self.message_retriever.count(self.session_id).await
    }

    /// Get the count of events
    pub async fn event_count(&self) -> usize {
        self.event_emitter.event_count().await
    }

    /// Clear all events (useful between tests)
    pub async fn clear_events(&self) {
        self.event_emitter.clear().await;
    }

    /// Clear all messages (starts a fresh conversation)
    pub async fn clear_messages(&self) {
        self.message_retriever.clear_session(self.session_id).await;
    }

    /// Reset the loop (clear messages and events)
    pub async fn reset(&self) {
        self.clear_messages().await;
        self.clear_events().await;
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
    pub fn message_retriever(&self) -> &InMemoryMessageRetriever {
        &self.message_retriever
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
    /// use everruns_core::ToolCall;
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
    async fn test_reset() {
        let runner = InMemoryAgenticLoop::with_fixed_response("Response")
            .await
            .unwrap();

        runner.run_turn("Test").await.unwrap();
        assert!(runner.message_count().await.unwrap() > 0);
        assert!(runner.event_count().await > 0);

        runner.reset().await;
        assert_eq!(runner.message_count().await.unwrap(), 0);
        assert_eq!(runner.event_count().await, 0);
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
