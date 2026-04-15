// In-process runtime builder and runner.
// Decision: the public runtime is in-memory today, but uses the same core atoms
// and capability resolution path as the durable worker so behavior stays close.

use crate::backends::{
    DynAgentStore, DynEventEmitter, DynFileStore, DynHarnessStore, DynMessageStore,
    DynProviderStore, DynSessionStore, RuntimeAgentStore, RuntimeBackends, RuntimeEventCollector,
    RuntimeFileStore, RuntimeHarnessStore, RuntimeMessageStore, RuntimeProviderStore,
    RuntimeSessionStore,
};
use crate::in_memory::{
    InMemorySessionFileStore, InMemorySessionStorageStore, InMemorySessionStore,
};
use async_trait::async_trait;
use everruns_core::agent::Agent;
use everruns_core::atoms::{
    ActAtom, ActInput, Atom, AtomContext, InputAtom, InputAtomInput, ReasonAtom, ReasonInput,
};
use everruns_core::capabilities::{
    Capability, CapabilityRegistry, SystemPromptContext, collect_capabilities_with_configs,
    resolve_capability_configs,
};
use everruns_core::config_layer::AgentConfigOverlay;
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::events::{
    Event, EventContext, EventData, EventRequest, InputMessageData, OutputMessageCompletedData,
    ToolCompletedData,
};
use everruns_core::harness::Harness;
use everruns_core::llm_driver_registry::{DriverRegistry, ProviderType};
use everruns_core::llm_models::LlmProviderType;
use everruns_core::llmsim_driver::{LlmSimConfig, LlmSimDriver};
use everruns_core::memory::{
    InMemoryAgentStore, InMemoryEventEmitter, InMemoryHarnessStore, InMemoryLlmProviderStore,
    InMemoryMemoryStore, InMemoryMessageRetriever,
};
use everruns_core::message::{ContentPart, Message};
use everruns_core::platform_definition::PlatformDefinition;
use everruns_core::runtime_agent::default_max_iterations;
use everruns_core::session::Session;
use everruns_core::session_file::{InitialFile, SessionFile};
use everruns_core::tools::{ToolRegistry, ToolResultImage};
use everruns_core::traits::{EventEmitter, ModelWithProvider, SessionStorageStore};
use everruns_core::turn::{TurnAction, TurnContext, TurnOutcome, TurnStateMachine};
use everruns_core::typed_id::{AgentId, OrgId, SessionId};
use everruns_core::{InputMessage, MemoryStoreBackend};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct TurnResult {
    /// Final text response produced by the turn.
    pub response: String,
    /// Number of reason iterations executed.
    pub iterations: usize,
    /// Total number of tool calls executed during the turn.
    pub tool_calls_count: usize,
    /// Whether the turn completed without an unrecoverable failure.
    pub success: bool,
    /// Failure message when `success` is false.
    pub error: Option<String>,
    /// Turn identifier used to correlate emitted events.
    pub turn_id: everruns_core::typed_id::TurnId,
}

impl TurnResult {
    fn from_outcome(outcome: TurnOutcome, turn_id: everruns_core::typed_id::TurnId) -> Self {
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
                success: true,
                error: None,
                turn_id,
            },
        }
    }
}

/// Builder for the public in-process runtime.
///
/// The builder owns a standalone runtime bundle:
/// - `PlatformDefinition` for capabilities and drivers
/// - in-memory stores for sessions, files, storage, memory, and messages
/// - seeded harness/agent/session entities
///
/// `build()` returns an [`InProcessRuntime`] that can execute turns in-process
/// without the durable engine or the control-plane server.
pub struct InProcessRuntimeBuilder {
    platform_definition: PlatformDefinition,
    llm_sim_config: Option<LlmSimConfig>,
    default_model: Option<ModelWithProvider>,
    backends: Option<RuntimeBackends>,
    harnesses: Vec<Harness>,
    agents: Vec<Agent>,
    sessions: Vec<Session>,
    seeded_files: Vec<(SessionId, InitialFile)>,
}

impl Default for InProcessRuntimeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl InProcessRuntimeBuilder {
    /// Create a builder with built-in capabilities and no implicit LLM driver.
    ///
    /// Embedders must either:
    /// - call [`Self::llm_sim`] for deterministic local examples/tests, or
    /// - register their own driver(s) on the platform definition and set a
    ///   default model via [`Self::default_model`].
    pub fn new() -> Self {
        Self {
            platform_definition: PlatformDefinition::new(
                CapabilityRegistry::with_builtins(),
                DriverRegistry::new(),
            ),
            llm_sim_config: None,
            default_model: None,
            backends: None,
            harnesses: Vec::new(),
            agents: Vec::new(),
            sessions: Vec::new(),
            seeded_files: Vec::new(),
        }
    }

    /// Replace the platform definition used by the runtime.
    pub fn platform_definition(mut self, platform_definition: PlatformDefinition) -> Self {
        self.platform_definition = platform_definition;
        self
    }

    /// Register an additional capability on the runtime platform.
    pub fn capability<C: Capability + 'static>(mut self, capability: C) -> Self {
        self.platform_definition
            .capability_registry_mut()
            .register(capability);
        self
    }

    /// Replace the platform driver registry.
    pub fn driver_registry(mut self, driver_registry: DriverRegistry) -> Self {
        *self.platform_definition.driver_registry_mut() = driver_registry;
        self
    }

    /// Register the built-in `llmsim` driver for deterministic local execution.
    pub fn llm_sim(mut self, config: LlmSimConfig) -> Self {
        self.llm_sim_config = Some(config);
        self
    }

    /// Set the runtime default model used when sessions/agents do not override it.
    pub fn default_model(mut self, model: ModelWithProvider) -> Self {
        self.default_model = Some(model);
        self
    }

    /// Supply a custom backend bundle instead of the built-in in-memory stores.
    pub fn backends(mut self, backends: RuntimeBackends) -> Self {
        self.backends = Some(backends);
        self
    }

    /// Seed a harness into the runtime store.
    pub fn harness(mut self, harness: Harness) -> Self {
        self.harnesses.push(harness);
        self
    }

    /// Seed an agent into the runtime store.
    pub fn agent(mut self, agent: Agent) -> Self {
        self.agents.push(agent);
        self
    }

    /// Seed a session into the runtime store.
    pub fn session(mut self, session: Session) -> Self {
        self.sessions.push(session);
        self
    }

    /// Seed an additional text file directly into a session workspace.
    ///
    /// This is applied after harness/agent/session `initial_files` are merged.
    pub fn seed_text_file(
        mut self,
        session_id: SessionId,
        path: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        self.seeded_files.push((
            session_id,
            InitialFile {
                path: path.into(),
                content: content.into(),
                encoding: "text".to_string(),
                is_readonly: false,
            },
        ));
        self
    }

    /// Build the in-process runtime.
    ///
    /// Returns a configuration error when no default model is available after
    /// applying explicit configuration and any requested `llmsim` setup.
    pub async fn build(mut self) -> Result<InProcessRuntime> {
        let backends = self
            .backends
            .take()
            .unwrap_or_else(default_in_memory_backends);

        if let Some(config) = self.llm_sim_config.take() {
            let driver = LlmSimDriver::new(config);
            self.platform_definition
                .driver_registry_mut()
                .register(ProviderType::LlmSim, move |_api_key, _base_url| {
                    Box::new(driver.clone())
                });

            if self.default_model.is_none() {
                self.default_model = Some(ModelWithProvider {
                    model: "llmsim-model".to_string(),
                    provider_type: LlmProviderType::LlmSim,
                    api_key: Some("fake-key".to_string()),
                    base_url: None,
                });
            }
        }

        let default_model = self.default_model.ok_or_else(|| {
            AgentLoopError::config(
                "in-process runtime requires a default model; call \
                 InProcessRuntimeBuilder::default_model(...) or \
                 InProcessRuntimeBuilder::llm_sim(...)",
            )
        })?;

        backends
            .provider_store
            .set_default_model(default_model)
            .await?;

        for harness in &self.harnesses {
            backends.harness_store.add_harness(harness.clone()).await?;
        }
        for agent in &self.agents {
            backends.agent_store.add_agent(agent.clone()).await?;
        }
        for session in &self.sessions {
            backends.session_store.add_session(session.clone()).await?;
        }

        for session in &self.sessions {
            seed_runtime_initial_files(
                backends.harness_store.as_ref(),
                backends.agent_store.as_ref(),
                backends.file_store.as_ref(),
                session,
            )
            .await?;
        }

        for (session_id, file) in &self.seeded_files {
            backends
                .file_store
                .seed_initial_file(*session_id, file)
                .await?;
        }

        let event_emitter = PersistingEventEmitter::new(
            backends.event_emitter.clone(),
            backends.message_store.clone(),
        );

        Ok(InProcessRuntime {
            platform_definition: Arc::new(self.platform_definition),
            harness_store: backends.harness_store,
            agent_store: backends.agent_store,
            session_store: backends.session_store,
            message_store: backends.message_store,
            provider_store: backends.provider_store,
            event_emitter,
            raw_event_emitter: backends.event_emitter,
            event_collector: backends.event_collector,
            file_store: backends.file_store,
            storage_store: backends.storage_store,
            memory_store: backends.memory_store,
        })
    }
}

fn default_in_memory_backends() -> RuntimeBackends {
    let harness_store: Arc<dyn RuntimeHarnessStore> = Arc::new(InMemoryHarnessStore::new());
    let agent_store: Arc<dyn RuntimeAgentStore> = Arc::new(InMemoryAgentStore::new());
    let session_store: Arc<dyn RuntimeSessionStore> = Arc::new(InMemorySessionStore::new());
    let message_store: Arc<dyn RuntimeMessageStore> = Arc::new(InMemoryMessageRetriever::new());
    let file_store: Arc<dyn RuntimeFileStore> = Arc::new(InMemorySessionFileStore::new());
    let storage_store: Arc<dyn SessionStorageStore> = Arc::new(InMemorySessionStorageStore::new());
    let memory_store: Arc<dyn MemoryStoreBackend> = Arc::new(InMemoryMemoryStore::new());
    let provider_store: Arc<dyn RuntimeProviderStore> = Arc::new(InMemoryLlmProviderStore::new());
    let event_emitter_impl = InMemoryEventEmitter::new();
    let event_emitter: Arc<dyn EventEmitter> = Arc::new(event_emitter_impl.clone());
    let event_collector: Arc<dyn RuntimeEventCollector> = Arc::new(event_emitter_impl);

    RuntimeBackends {
        harness_store,
        agent_store,
        session_store,
        message_store,
        provider_store,
        event_emitter,
        event_collector: Some(event_collector),
        file_store,
        storage_store,
        memory_store,
    }
}

#[derive(Clone)]
/// Public in-process runtime backed by either in-memory or custom stores.
///
/// This runtime is intended for embedders who want to execute Everruns
/// harnesses inside their own process while controlling capabilities,
/// harness definitions, and driver registrations directly in Rust.
pub struct InProcessRuntime {
    platform_definition: Arc<PlatformDefinition>,
    harness_store: Arc<dyn RuntimeHarnessStore>,
    agent_store: Arc<dyn RuntimeAgentStore>,
    session_store: Arc<dyn RuntimeSessionStore>,
    message_store: Arc<dyn RuntimeMessageStore>,
    provider_store: Arc<dyn RuntimeProviderStore>,
    event_emitter: PersistingEventEmitter,
    raw_event_emitter: Arc<dyn EventEmitter>,
    event_collector: Option<Arc<dyn RuntimeEventCollector>>,
    file_store: Arc<dyn RuntimeFileStore>,
    storage_store: Arc<dyn SessionStorageStore>,
    memory_store: Arc<dyn MemoryStoreBackend>,
}

impl InProcessRuntime {
    /// Create a builder for the in-process runtime.
    pub fn builder() -> InProcessRuntimeBuilder {
        InProcessRuntimeBuilder::new()
    }

    /// Execute one turn for an existing session.
    ///
    /// The input message is stored in the runtime history, an `input.message`
    /// event is emitted, and the turn then executes the shared core
    /// `input -> reason -> act` state machine.
    pub async fn run_turn(
        &self,
        session_id: SessionId,
        input: impl Into<InputMessage>,
    ) -> Result<TurnResult> {
        let session = self
            .session_store
            .get_session(session_id)
            .await?
            .ok_or_else(|| AgentLoopError::store(format!("session not found: {session_id}")))?;

        let input_message = self
            .message_store
            .add_input_message(session_id, input.into())
            .await?;
        self.raw_event_emitter
            .emit(EventRequest::new(
                session_id,
                EventContext::empty(),
                InputMessageData::new(input_message.clone()),
            ))
            .await?;
        let harness_chain = self
            .harness_store
            .get_harness_chain(session.harness_id)
            .await?;
        if harness_chain.is_empty() {
            return Err(AgentLoopError::store(format!(
                "harness not found: {}",
                session.harness_id
            )));
        }

        let agent = match session.agent_id {
            Some(agent_id) => self
                .agent_store
                .get_agent(agent_id)
                .await?
                .ok_or_else(|| AgentLoopError::store(format!("agent not found: {agent_id}")))
                .map(Some)?,
            None => None,
        };

        let overlay = effective_overlay(&harness_chain, agent.as_ref(), &session);
        let capability_registry = self.platform_definition.capability_registry().clone();
        let driver_registry = self.platform_definition.driver_registry().clone();
        let resolved_configs =
            resolve_capability_configs(&overlay.capabilities, &capability_registry).unwrap_or_else(
                |error| {
                    tracing::warn!(
                        error = ?error,
                        "failed to resolve capability configs; falling back to overlay capabilities"
                    );
                    overlay.capabilities.clone()
                },
            );
        let system_prompt_ctx = SystemPromptContext {
            session_id,
            locale: session.locale.clone(),
            file_store: Some(Arc::new(DynFileStore(self.file_store.clone()))),
        };
        let collected = collect_capabilities_with_configs(
            &resolved_configs,
            &capability_registry,
            &system_prompt_ctx,
        )
        .await;
        let post_tool_hooks = resolved_configs
            .iter()
            .flat_map(|config| {
                capability_registry
                    .get(config.capability_id())
                    .map(|capability| capability.post_tool_exec_hooks())
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>();

        let tool_registry = build_tool_registry(collected.tools);
        let synthetic_agent_id = session
            .agent_id
            .unwrap_or_else(|| AgentId::from_uuid(session.id.uuid()));
        let org_id = 0;
        let org_public_id = session
            .organization_id
            .parse::<OrgId>()
            .unwrap_or_else(|_| {
                everruns_core::DEFAULT_ORG_PUBLIC_ID
                    .parse()
                    .expect("valid org id")
            });

        let input_atom = InputAtom::new(DynMessageStore(self.message_store.clone()));
        let reason_atom = ReasonAtom::new(
            DynHarnessStore(self.harness_store.clone()),
            DynAgentStore(self.agent_store.clone()),
            DynSessionStore(self.session_store.clone()),
            DynMessageStore(self.message_store.clone()),
            DynProviderStore(self.provider_store.clone()),
            capability_registry.clone(),
            driver_registry,
            DynEventEmitter(Arc::new(self.event_emitter.clone())),
        )
        .with_file_store(Arc::new(DynFileStore(self.file_store.clone())));

        let act_atom = ActAtom::with_file_store(
            tool_registry.clone(),
            DynEventEmitter(Arc::new(self.event_emitter.clone())),
            Arc::new(DynFileStore(self.file_store.clone())),
        )
        .with_storage_store(self.storage_store.clone())
        .with_session_store(Arc::new(DynSessionStore(self.session_store.clone())))
        .with_session_mutator(Arc::new(DynSessionStore(self.session_store.clone())))
        .with_memory_store(self.memory_store.clone())
        .with_org_id(org_public_id)
        .with_capability_registry(capability_registry)
        .with_post_tool_hooks(post_tool_hooks);

        let mut previous_response_id: Option<String> = None;
        let mut last_reason_result: Option<everruns_core::ReasonResult> = None;
        let max_iterations = overlay
            .max_iterations
            .unwrap_or_else(default_max_iterations);
        let mut state_machine = TurnStateMachine::new(
            TurnContext::new(session_id, input_message.id, synthetic_agent_id, org_id),
            max_iterations,
        );

        loop {
            match state_machine.next_action() {
                TurnAction::ExecuteInput => {
                    let base_context = AtomContext::new(
                        state_machine.context().session_id,
                        state_machine.context().turn_id,
                        state_machine.context().input_message_id,
                    );
                    input_atom
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
                    let reason_result = reason_atom
                        .execute(ReasonInput {
                            context: base_context.next_exec(),
                            harness_id: session.harness_id,
                            agent_id: session.agent_id,
                            org_id,
                            mcp_tool_definitions: vec![],
                            previous_response_id: previous_response_id.take(),
                            iteration: state_machine.current_iteration() as u32 + 1,
                        })
                        .await?;

                    let tool_call_count = reason_result.tool_calls.len();
                    previous_response_id = reason_result.response_id.clone();
                    state_machine.on_reason_completed(
                        reason_result.text.clone(),
                        reason_result.has_tool_calls,
                        tool_call_count,
                        reason_result.success,
                        reason_result.error.clone(),
                        false,
                    );
                    if reason_result.has_tool_calls {
                        last_reason_result = Some(reason_result);
                    }
                }
                TurnAction::ExecuteAct => {
                    let reason_result = last_reason_result
                        .take()
                        .expect("ExecuteAct requires a prior ReasonResult");
                    let base_context = AtomContext::new(
                        state_machine.context().session_id,
                        state_machine.context().turn_id,
                        state_machine.context().input_message_id,
                    );
                    act_atom
                        .execute(ActInput {
                            org_id: Some(org_id),
                            context: base_context.next_exec(),
                            harness_id: session.harness_id,
                            agent_id: session.agent_id,
                            tool_calls: reason_result.tool_calls,
                            tool_definitions: reason_result.tool_definitions,
                            locale: reason_result.locale,
                            blueprint_id: None,
                            network_access: reason_result.network_access,
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

    pub async fn run_text_turn(
        &self,
        session_id: SessionId,
        text: impl Into<String>,
    ) -> Result<TurnResult> {
        self.run_turn(session_id, InputMessage::user(text)).await
    }

    /// Load the current message history for a session.
    pub async fn messages(&self, session_id: SessionId) -> Result<Vec<Message>> {
        self.message_store.load(session_id).await
    }

    /// Read a file from the in-memory session filesystem.
    pub async fn read_file(
        &self,
        session_id: SessionId,
        path: &str,
    ) -> Result<Option<SessionFile>> {
        self.file_store.read_file(session_id, path).await
    }

    /// Return all emitted events when the backend exposes an event collector.
    pub async fn events(&self) -> Result<Vec<Event>> {
        let collector = self.event_collector.as_ref().ok_or_else(|| {
            AgentLoopError::config("events are not available for this runtime backend")
        })?;
        Ok(collector.events().await)
    }
}

#[derive(Clone)]
struct PersistingEventEmitter {
    inner: Arc<dyn EventEmitter>,
    message_store: Arc<dyn RuntimeMessageStore>,
}

impl PersistingEventEmitter {
    fn new(inner: Arc<dyn EventEmitter>, message_store: Arc<dyn RuntimeMessageStore>) -> Self {
        Self {
            inner,
            message_store,
        }
    }
}

#[async_trait]
impl EventEmitter for PersistingEventEmitter {
    async fn emit(&self, request: EventRequest) -> Result<Event> {
        let event = self.inner.emit(request.clone()).await?;
        if let Some(message) = message_from_event(&event.data) {
            self.message_store
                .store_message(request.session_id, message)
                .await?;
        }
        Ok(event)
    }
}

fn effective_overlay(
    harness_chain: &[Harness],
    agent: Option<&Agent>,
    session: &Session,
) -> AgentConfigOverlay {
    let harness_layers = harness_chain.iter().map(AgentConfigOverlay::from);
    let agent_layers = agent.into_iter().map(AgentConfigOverlay::from);
    AgentConfigOverlay::fold(
        harness_layers
            .chain(agent_layers)
            .chain([AgentConfigOverlay::from(session)]),
    )
}

async fn seed_runtime_initial_files(
    harness_store: &dyn RuntimeHarnessStore,
    agent_store: &dyn RuntimeAgentStore,
    file_store: &dyn RuntimeFileStore,
    session: &Session,
) -> Result<()> {
    let harness_chain = harness_store.get_harness_chain(session.harness_id).await?;
    if harness_chain.is_empty() {
        return Err(AgentLoopError::store(format!(
            "harness not found while seeding files: {}",
            session.harness_id
        )));
    }
    let agent = match session.agent_id {
        Some(agent_id) => Some(
            agent_store
                .get_agent(agent_id)
                .await?
                .ok_or_else(|| AgentLoopError::store(format!("agent not found: {agent_id}")))?,
        ),
        None => None,
    };
    let overlay = effective_overlay(&harness_chain, agent.as_ref(), session);
    for file in &overlay.initial_files {
        file_store.seed_initial_file(session.id, file).await?;
    }
    Ok(())
}

fn build_tool_registry(tools: Vec<Box<dyn everruns_core::tools::Tool>>) -> ToolRegistry {
    let mut registry = ToolRegistry::with_defaults();
    for tool in tools {
        registry.register_boxed(tool);
    }
    registry
}

fn message_from_event(data: &EventData) -> Option<Message> {
    match data {
        EventData::InputMessage(data) => Some(data.message.clone()),
        EventData::OutputMessageCompleted(OutputMessageCompletedData { message, .. }) => {
            Some(message.clone())
        }
        EventData::ToolCompleted(data) => Some(tool_completed_to_message(data.clone())),
        _ => None,
    }
}

fn tool_completed_to_message(data: ToolCompletedData) -> Message {
    let mut images: Vec<ToolResultImage> = Vec::new();
    let result = data.result.map(|parts| {
        for part in &parts {
            if let ContentPart::Image(img) = part
                && let (Some(base64), Some(media_type)) = (&img.base64, &img.media_type)
            {
                images.push(ToolResultImage {
                    base64: base64.clone(),
                    media_type: media_type.clone(),
                });
            }
        }

        let text_parts: Vec<&ContentPart> = parts
            .iter()
            .filter(|part| matches!(part, ContentPart::Text(_)))
            .collect();
        if text_parts.len() == 1
            && let ContentPart::Text(text) = text_parts[0]
        {
            return serde_json::Value::String(text.text.clone());
        }
        if !text_parts.is_empty() {
            serde_json::to_value(&text_parts).unwrap_or_default()
        } else {
            serde_json::Value::Null
        }
    });

    if images.is_empty() {
        Message::tool_result(&data.tool_call_id, result, data.error)
    } else {
        Message::tool_result_with_images(&data.tool_call_id, result, images)
    }
}
