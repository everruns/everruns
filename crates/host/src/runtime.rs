// In-process host builder and runner.
// Decision: the public runtime is in-memory today, but uses the same core atoms
// and capability resolution path as the durable worker so behavior stays close.

use crate::HostComposition;
use crate::SessionFileSystemFactoryContext;
use crate::backends::{
    HostBackends, RuntimeAgentStore, RuntimeHarnessStore, RuntimeProviderStore, RuntimeSessionStore,
};
use crate::builders::SingleSessionBuilder;
use crate::events::{EventHistory, EventLog, EventReadLimit, EventReadRequest, HostEventEmitter};
use crate::host::{
    ResolvedTurnInputs, RuntimeHostAdapter, execute_act_activity, execute_input_activity,
    execute_reason_activity_with_prompt_messages,
};
use crate::in_memory::{InMemorySessionFileStore, InMemorySessionFileSystemFactory};
use crate::turn_strategy::{RuntimeTurnPlan, RuntimeTurnState};
use async_trait::async_trait;
use chrono::Utc;
use everruns_capability::plugin_capability_id;
use everruns_core::agent_definition::AgentDefinition;
use everruns_core::atoms::{AtomContext, InputAtomInput, ReasonInput};
#[cfg(feature = "mcp")]
use everruns_core::capabilities::collect_capability_mcp_servers;
use everruns_core::capabilities::{
    Capability, CapabilityRegistry, CapabilityStatus, resolve_capability_configs,
};
use everruns_core::config_layer::AgentConfigOverlay;
use everruns_core::events::{
    Event, EventContext, EventRequest, InputMessageData, SessionStartedData,
};
use everruns_core::harness_definition::HarnessDefinition;
use everruns_core::message::Message;
use everruns_core::plugins::{PluginFileSet, compile_plugin};
#[cfg(feature = "mcp")]
use everruns_core::resolve_runtime_capabilities;
use everruns_core::runtime_context::AssembledTurnContext;
use everruns_core::session::{ExecutionSession, SessionExecutionState};
use everruns_core::session_file::{InitialFile, SessionFile};
use everruns_core::turn::TurnStopReason;
use everruns_core::{
    InputMessage, MessageRetriever, ResolvedExecutionSnapshot, session_files::SessionFileSystem,
};
use everruns_core::{
    connection_services::UserConnectionResolver, event_emitter::EventEmitter,
    execution_loading::AgentStore, execution_loading::HarnessStore,
    execution_loading::SessionStore, provider_resolution::ProviderStore,
    session_services::SessionStorageStore,
};
use everruns_engine::{
    ActOutcome, plan_after_act, plan_after_process_input, plan_after_reason, reason_schedules_act,
};
use everruns_platform::SessionMutator;
use everruns_provider::driver_registry::DriverRegistry;
use everruns_provider::error::{AgentLoopError, Result};
use everruns_provider::typed_id::{AgentId, MessageId, OrgId, SessionId, TurnId};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Cap on the input length hashed by [`hash_public_org_id`].
///
/// Legitimate org public ids are `org_<32hex>` (36 bytes). Bounding the
/// hashed prefix keeps worst-case cost predictable when an attacker-controlled
/// session carries an oversize string.
const HASH_INPUT_CAP_BYTES: usize = 128;

/// Derive an internal `i64` org id from the public `org_<32hex>` form on a
/// [`ExecutionSession`].
///
/// Round-trip with [`everruns_core::org_public_id_from_internal`]: when the
/// public id was produced by that helper (i.e. the upper bits are zero and
/// the value fits in a positive `i64`), this returns the original internal
/// id unchanged. Other values are mapped into `[2, i64::MAX]` by hashing the
/// original public id string so runtime namespaces do not fail open to the
/// shared default org and avoid arithmetic collision gadgets.
///
/// Exposed so embedders (e.g. `everruns-local`) can scope per-org stores to the
/// same internal id the act path resolves a session's org to.
pub fn in_process_internal_org_id(public_org_id: &str) -> i64 {
    if public_org_id == everruns_core::DEFAULT_ORG_PUBLIC_ID {
        return everruns_core::DEFAULT_ORG_ID;
    }

    let Ok(parsed) = public_org_id.parse::<OrgId>() else {
        return hash_public_org_id(public_org_id);
    };
    let raw: u128 = parsed.uuid().as_u128();
    if raw == 0 {
        return hash_public_org_id(public_org_id);
    }

    // Synthetic ids from `org_public_id_from_internal(i64)` always fit here,
    // so the in-process runtime sees the same `org_id` the server used.
    if raw <= i64::MAX as u128 {
        return raw as i64;
    }

    hash_public_org_id(public_org_id)
}

// Use SHA-256 with a fixed truncation scheme so the mapping is stable across
// Rust/binary upgrades and predictable for any embedder. Input is bounded to
// `HASH_INPUT_CAP_BYTES` so attacker-controlled oversize org strings cannot
// drive unbounded hashing work.
fn hash_public_org_id(public_org_id: &str) -> i64 {
    let bytes = public_org_id.as_bytes();
    let bounded = &bytes[..bytes.len().min(HASH_INPUT_CAP_BYTES)];
    let digest = Sha256::digest(bounded);
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&digest[..8]);
    let raw = u64::from_be_bytes(buf);
    ((raw % ((i64::MAX - 1) as u64)) as i64) + 2
}

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
    /// Structured reason the turn stopped.
    pub stop_reason: TurnStopReason,
    /// Turn identifier used to correlate emitted events.
    pub turn_id: everruns_provider::typed_id::TurnId,
}

/// An application message accepted for a specific in-process turn.
///
/// The caller creates this value before dispatch so it can acknowledge the
/// stable message id without waiting for execution to begin.
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct AcceptedTurnInput {
    message_id: MessageId,
    input: InputMessage,
}

impl AcceptedTurnInput {
    pub fn new(input: impl Into<InputMessage>) -> Self {
        Self {
            message_id: MessageId::new(),
            input: input.into(),
        }
    }

    pub fn message_id(&self) -> MessageId {
        self.message_id
    }

    pub fn input(&self) -> &InputMessage {
        &self.input
    }

    fn into_message(self) -> Message {
        message_from_input_with_id(self.message_id, self.input)
    }
}

/// Concurrency-safe ingress for messages sent while an in-process turn runs.
///
/// Closing and observing an empty queue is one atomic operation. A sender can
/// therefore never be told that it steered a turn after that turn committed to
/// completion; rejected input belongs to the next turn instead.
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct TurnSteering {
    state: Arc<Mutex<TurnSteeringState>>,
}

#[doc(hidden)]
#[derive(Debug)]
pub enum TurnSteeringPushError {
    Closed(Box<AcceptedTurnInput>),
    Full(Box<AcceptedTurnInput>),
}

/// Bounds user input retained between reason boundaries when a model or tool is slow.
// THREAT[TM-DOS-036]: reject overflow before accepting more steering input.
const TURN_STEERING_CAPACITY: usize = 256;

#[derive(Debug, Default)]
struct TurnSteeringState {
    open: bool,
    inputs: VecDeque<AcceptedTurnInput>,
}

impl TurnSteering {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(TurnSteeringState {
                open: true,
                inputs: VecDeque::new(),
            })),
        }
    }

    pub fn try_push(
        &self,
        input: AcceptedTurnInput,
    ) -> std::result::Result<(), TurnSteeringPushError> {
        let mut state = self.state.lock().expect("turn steering lock poisoned");
        if !state.open {
            return Err(TurnSteeringPushError::Closed(Box::new(input)));
        }
        if state.inputs.len() >= TURN_STEERING_CAPACITY {
            return Err(TurnSteeringPushError::Full(Box::new(input)));
        }
        state.inputs.push_back(input);
        Ok(())
    }

    fn drain(&self) -> Vec<AcceptedTurnInput> {
        let mut state = self.state.lock().expect("turn steering lock poisoned");
        state.inputs.drain(..).collect()
    }

    /// Drain accepted input, or close the ingress when there is none.
    fn drain_or_close(&self) -> Vec<AcceptedTurnInput> {
        let mut state = self.state.lock().expect("turn steering lock poisoned");
        if state.inputs.is_empty() {
            state.open = false;
            return vec![];
        }
        state.inputs.drain(..).collect()
    }

    pub fn close(&self) {
        self.state.lock().expect("turn steering lock poisoned").open = false;
    }

    pub fn close_and_drain(&self) -> Vec<AcceptedTurnInput> {
        let mut state = self.state.lock().expect("turn steering lock poisoned");
        state.open = false;
        state.inputs.drain(..).collect()
    }
}

impl Default for TurnSteering {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of changing the session-scoped capability set of a live runtime.
///
/// A changed result is the refresh seam for embedders: every subsequent reason
/// or act boundary reassembles model and execution surfaces from the updated
/// session overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityDelta {
    /// Canonical capability id.
    pub capability_id: String,
    /// Whether the session overlay changed.
    pub changed: bool,
    /// Whether the capability is active after the operation.
    pub active: bool,
    /// Whether prompt, tool, hook, command, or MCP surfaces must be refreshed.
    pub surfaces_dirty: bool,
}

/// Summarize a terminal engine plan into the public [`TurnResult`].
///
/// The engine reports the stop reason and the carried error; the host supplies
/// the running summaries it accumulated while executing the planned steps. A
/// carried error is the failure signal (it is exactly what the pre-engine state
/// machine kept in `pending_error`), and a failed turn reports no response and
/// no tool calls, matching the previous `TurnOutcome::Failed` mapping.
fn finish_turn(
    turn_id: everruns_provider::typed_id::TurnId,
    stop_reason: TurnStopReason,
    error: Option<String>,
    response: String,
    iterations: usize,
    tool_calls_count: usize,
) -> TurnResult {
    if error.is_some() {
        return TurnResult {
            response: String::new(),
            iterations,
            tool_calls_count: 0,
            success: false,
            error,
            stop_reason,
            turn_id,
        };
    }

    TurnResult {
        response,
        iterations,
        tool_calls_count,
        success: true,
        error: None,
        stop_reason,
        turn_id,
    }
}

/// Builder for the public in-process runtime.
///
/// The builder owns a standalone runtime bundle:
/// - `HostComposition` for capabilities and drivers
/// - in-memory stores for sessions, files, storage, memory, and messages
/// - seeded harness/agent/session entities
///
/// `build()` returns an [`InProcessRuntime`] that can execute turns in-process
/// without the durable engine or the control-plane server.
pub struct InProcessRuntimeBuilder {
    host_composition: HostComposition,
    /// Provider registered at build time (replacing any same-name provider)
    /// whose named model becomes the runtime default when nothing else set one.
    /// See [`Self::provider_with_default_model`].
    default_provider: Option<(everruns_provider::runtime_provider::Provider, String)>,
    providers: Vec<everruns_provider::runtime_provider::Provider>,
    model_spec: Option<everruns_provider::model_spec::ModelSpec>,
    backends: Option<HostBackends>,
    workspace_policy: Option<everruns_core::WorkspacePolicy>,
    session_file_system_factory_context: SessionFileSystemFactoryContext,
    harnesses: Vec<crate::builders::SeededHarness>,
    agents: Vec<AgentDefinition>,
    sessions: Vec<ExecutionSession>,
    default_session_id: Option<SessionId>,
    seeded_files: Vec<(SessionId, InitialFile)>,
    #[cfg(feature = "mcp")]
    mcp_auth_provider: Option<Arc<dyn everruns_mcp::McpAuthProvider>>,
    provider_retry_config: Option<everruns_provider::llm_retry::LlmRetryConfig>,
    provider_stall_timeout: Option<std::time::Duration>,
    /// Hydrated capability configs for plugins loaded via [`Self::with_plugin_dir`].
    ///
    /// Keyed by `plugin:{name}`. Agents and harnesses reference these by the
    /// same `plugin:{name}` capability ref; the hydrated config carries the
    /// compiled `DeclarativeCapabilityDefinition` so no registry entry is needed.
    plugin_capability_configs: Vec<everruns_capability::CapabilityRef>,
    /// Non-fatal warnings collected during plugin compilation.
    plugin_warnings: Vec<String>,
}

impl Default for InProcessRuntimeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl InProcessRuntimeBuilder {
    /// Create a builder with runtime-safe built-in capabilities and no implicit
    /// LLM driver.
    ///
    /// Embedders must either:
    /// - call [`Self::provider_with_default_model`] (e.g. via the
    ///   `everruns-test-support` `.llm_sim(...)` extension) for deterministic
    ///   local examples/tests, or
    /// - register their own driver(s) on the platform definition and set a
    ///   default model via [`Self::default_model`].
    pub fn new() -> Self {
        Self {
            host_composition: HostComposition::builder()
                .capability_registry(crate::runtime_capability_registry())
                .driver_registry(DriverRegistry::new())
                .egress_service(crate::runtime_egress_service())
                .session_file_system_factory(Arc::new(InMemorySessionFileSystemFactory))
                .build(),
            default_provider: None,
            providers: Vec::new(),
            model_spec: None,
            backends: None,
            workspace_policy: None,
            session_file_system_factory_context: SessionFileSystemFactoryContext::new(),
            harnesses: Vec::new(),
            agents: Vec::new(),
            sessions: Vec::new(),
            default_session_id: None,
            seeded_files: Vec::new(),
            #[cfg(feature = "mcp")]
            mcp_auth_provider: None,
            provider_retry_config: None,
            provider_stall_timeout: None,
            plugin_capability_configs: Vec::new(),
            plugin_warnings: Vec::new(),
        }
    }

    /// Set the auth provider used to acquire credentials for scoped MCP
    /// servers (knowledge/integrations/runtime-mcp.md D3). Defaults to no credentials, suitable
    /// for unauthenticated servers or servers carrying literal auth headers.
    #[cfg(feature = "mcp")]
    pub fn mcp_auth_provider(mut self, provider: Arc<dyn everruns_mcp::McpAuthProvider>) -> Self {
        self.mcp_auth_provider = Some(provider);
        self
    }

    /// Replace the platform definition used by the runtime.
    pub fn host_composition(mut self, host_composition: HostComposition) -> Self {
        self.host_composition = host_composition;
        self
    }

    /// Register an additional capability on the runtime platform.
    pub fn capability<C: Capability + 'static>(mut self, capability: C) -> Self {
        self.host_composition
            .capability_registry_mut()
            .register(capability);
        self
    }

    /// Replace the platform driver registry.
    pub fn driver_registry(mut self, driver_registry: DriverRegistry) -> Self {
        *self.host_composition.driver_registry_mut() = driver_registry;
        self
    }

    /// Register a canonical runtime provider selected by [`ModelSpec`](everruns_provider::model_spec::ModelSpec).
    pub fn provider(mut self, provider: everruns_provider::runtime_provider::Provider) -> Self {
        self.providers.push(provider);
        self
    }

    /// Register `provider` at build time — replacing any provider already
    /// registered under the same name — and default the runtime model to
    /// `model_id` on that provider when no other default was configured.
    ///
    /// This is the seam behind deterministic simulated runtimes: the
    /// `everruns-test-support` crate's `.llm_sim(...)` extension builds an
    /// `llmsim` provider and routes it through here.
    pub fn provider_with_default_model(
        mut self,
        provider: everruns_provider::runtime_provider::Provider,
        model_id: impl Into<String>,
    ) -> Self {
        self.default_provider = Some((provider, model_id.into()));
        self
    }

    /// Select the runtime's default credential-free model specification.
    pub fn default_model(mut self, model: everruns_provider::model_spec::ModelSpec) -> Self {
        self.model_spec = Some(model);
        self
    }

    /// Select a credential-free model served by a provider registered on this builder.
    pub fn model_spec(mut self, model: everruns_provider::model_spec::ModelSpec) -> Self {
        self.model_spec = Some(model);
        self
    }

    /// Supply a custom backend bundle instead of the built-in in-memory stores.
    pub fn backends(mut self, backends: HostBackends) -> Self {
        self.backends = Some(backends);
        self
    }

    /// Apply a backend-independent policy to the resolved session filesystem.
    ///
    /// The policy wraps whichever factory the platform selected, so in-memory,
    /// host-disk, database, and custom providers receive identical access
    /// checks without changing their implementations.
    pub fn workspace_policy(mut self, policy: everruns_core::WorkspacePolicy) -> Self {
        self.workspace_policy = Some(policy);
        self
    }

    /// Inject a session-task registry. Convenience over `backends(...)` that
    /// initializes an in-memory backend bundle on first use, so embedders can
    /// add the registry without assembling a full `HostBackends`.
    pub fn with_session_task_registry(
        mut self,
        registry: Arc<dyn everruns_core::session_task::SessionTaskRegistry>,
    ) -> Self {
        let backends = self.backends.take().unwrap_or_else(HostBackends::in_memory);
        self.backends = Some(backends.with_session_task_registry(registry));
        self
    }

    /// Inject a per-org schedule store factory (see [`HostBackends`]).
    pub fn with_schedule_store_factory(
        mut self,
        factory: crate::backends::ScheduleStoreFactory,
    ) -> Self {
        let backends = self.backends.take().unwrap_or_else(HostBackends::in_memory);
        self.backends = Some(backends.with_schedule_store_factory(factory));
        self
    }

    /// Inject a per-(org, session) platform store factory (see [`HostBackends`]).
    pub fn with_platform_store_factory(
        mut self,
        factory: crate::backends::PlatformStoreFactory,
    ) -> Self {
        let backends = self.backends.take().unwrap_or_else(HostBackends::in_memory);
        self.backends = Some(backends.with_platform_store_factory(factory));
        self
    }

    /// Supply host dependencies needed by the platform session filesystem factory.
    pub fn session_file_system_factory_context(
        mut self,
        context: SessionFileSystemFactoryContext,
    ) -> Self {
        self.session_file_system_factory_context = context;
        self
    }

    /// Override the bounded provider-recovery policy for this runtime.
    pub fn provider_retry_config(
        mut self,
        config: everruns_provider::llm_retry::LlmRetryConfig,
    ) -> Self {
        self.provider_retry_config = Some(config);
        self
    }

    /// Override the no-output provider stream stall timeout.
    pub fn provider_stall_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.provider_stall_timeout = Some(timeout);
        self
    }

    /// Seed a harness definition (under its embedder-chosen id) into the
    /// runtime store.
    pub fn harness(mut self, harness: crate::builders::SeededHarness) -> Self {
        self.harnesses.push(harness);
        self
    }

    /// Seed an agent definition into the runtime store.
    pub fn agent(mut self, agent: AgentDefinition) -> Self {
        self.agents.push(agent);
        self
    }

    /// Seed a session into the runtime store.
    pub fn session(mut self, session: ExecutionSession) -> Self {
        self.sessions.push(session);
        self
    }

    /// Seed one harness, one agent, and one session with a compact sub-builder.
    ///
    /// The generated session id is exposed from the built runtime via
    /// [`InProcessRuntime::default_session_id`].
    pub fn single_session<F>(mut self, configure: F) -> Self
    where
        F: FnOnce(SingleSessionBuilder) -> SingleSessionBuilder,
    {
        let (harness, agent, session, session_id) =
            configure(SingleSessionBuilder::default()).build();
        self.harnesses.push(harness);
        self.agents.push(agent);
        self.sessions.push(session);
        self.default_session_id = Some(session_id);
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

    /// Load a plugin from a local directory and make it available as a
    /// `plugin:{name}` capability.
    ///
    /// Reads the plugin directory via [`PluginFileSet::from_dir`] and compiles
    /// it with [`compile_plugin`] at call time. A compilation failure is
    /// surfaced immediately as a configuration error so the problem is visible
    /// before the runtime is built. Non-fatal compilation warnings are logged
    /// via `tracing::warn!` and also collected so they can be inspected on the
    /// built runtime via [`InProcessRuntime::plugin_warnings`].
    ///
    /// After loading, agents and harnesses can reference the plugin by its
    /// `plugin:{name}` capability ref. The hydrated config carries the compiled
    /// `DeclarativeCapabilityDefinition`, which the core capability resolution
    /// path recognises without a registry entry (same path as declarative
    /// capabilities).
    ///
    /// When using [`Self::single_session`], call
    /// [`SingleSessionBuilder::agent_plugin`] to add the capability ref to the
    /// seeded agent, or use [`AgentBuilder::capability`] / `with_capability`
    /// directly.
    pub fn with_plugin_dir(mut self, path: &Path) -> Result<Self> {
        let file_set = PluginFileSet::from_dir(path)
            .map_err(|e| AgentLoopError::config(format!("plugin directory load failed: {e}")))?;
        let compiled = compile_plugin(&file_set)
            .map_err(|e| AgentLoopError::config(format!("plugin compilation failed: {e}")))?;

        for warning in &compiled.warnings {
            tracing::warn!(plugin = %compiled.definition.name, warning = %warning, "plugin compile warning");
        }
        self.plugin_warnings.extend(compiled.warnings);

        let cap_id = plugin_capability_id(&compiled.definition.name);
        let hydrated_config = serde_json::to_value(&compiled.definition)
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
        self.plugin_capability_configs
            .push(everruns_capability::CapabilityRef::with_config(
                cap_id,
                hydrated_config,
            ));

        Ok(self)
    }

    /// Return a hydrated `everruns_capability::CapabilityRef` for a previously loaded plugin.
    ///
    /// Returns `None` when no plugin with that name was loaded via
    /// [`Self::with_plugin_dir`]. Primarily used by callers that need the
    /// hydrated config to seed it onto a harness or agent before building.
    pub fn plugin_capability(&self, name: &str) -> Option<everruns_capability::CapabilityRef> {
        let cap_id = plugin_capability_id(name);
        self.plugin_capability_configs
            .iter()
            .find(|c| c.capability_id() == cap_id)
            .cloned()
    }

    /// Build the in-process runtime.
    ///
    /// Returns a configuration error when no default model is available after
    /// applying explicit configuration and any requested `llmsim` setup.
    pub async fn build(mut self) -> Result<InProcessRuntime> {
        let backends = match self.backends.take() {
            Some(backends) => backends,
            None => HostBackends::in_memory(),
        };
        let mut file_store = resolve_session_file_system(
            &self.host_composition,
            self.session_file_system_factory_context.clone(),
        )
        .await?;
        if let Some(policy) = self.workspace_policy.take() {
            file_store = Arc::new(crate::PolicyFileStore::new(file_store, policy));
        }

        if let Some((provider, model_id)) = self.default_provider.take() {
            let provider_key = provider.id().clone();
            // This convenience adapts into the canonical provider path.
            self.host_composition
                .driver_registry_mut()
                .replace_provider(provider);

            if self.model_spec.is_none() {
                self.model_spec = Some(everruns_provider::model_spec::ModelSpec::on(
                    provider_key,
                    model_id,
                ));
            }
        }

        for provider in self.providers {
            self.host_composition
                .driver_registry_mut()
                .register_provider(provider)?;
        }

        let model_spec = self.model_spec.ok_or_else(|| {
            AgentLoopError::config(
                "in-process runtime requires a default model; call \
                 InProcessRuntimeBuilder::model_spec(...) or \
                 InProcessRuntimeBuilder::provider_with_default_model(...) \
                 (e.g. the everruns-test-support `.llm_sim(...)` extension)",
            )
        })?;

        backends
            .provider_store
            .set_default_model_spec(model_spec)
            .await?;

        // Hydrate bare plugin: refs in harnesses/agents/sessions with the
        // compiled definition config so the capability resolution path can
        // deserialise them without a registry entry (same path as declarative:).
        for harness in &mut self.harnesses {
            hydrate_plugin_refs(
                &mut harness.definition.capabilities,
                &self.plugin_capability_configs,
            );
        }
        for agent in &mut self.agents {
            hydrate_plugin_refs(&mut agent.capabilities, &self.plugin_capability_configs);
        }
        for session in &mut self.sessions {
            hydrate_plugin_refs(&mut session.capabilities, &self.plugin_capability_configs);
        }

        for harness in &self.harnesses {
            backends
                .harness_store
                .add_harness(harness.id, harness.definition.clone())
                .await?;
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
                file_store.as_ref(),
                session,
            )
            .await?;
        }

        for (session_id, file) in &self.seeded_files {
            file_store.seed_initial_file(*session_id, file).await?;
        }

        let event_log = backends.event_log.clone();
        let event_history = Arc::new(EventHistory::new(event_log.clone()));
        let event_emitter = Arc::new(HostEventEmitter::new(
            event_log.clone(),
            backends.event_sink.clone(),
        ));

        // Mid-turn wake delivery (EVE-681, part A): when a task registry is
        // present, wrap it so qualifying task transitions fan out to a
        // per-session `SessionWakeQueue`. The turn loop drains that queue at
        // each reason iteration boundary. Without a registry there is no
        // background work to wake on, so the queue is left absent (inert).
        let (session_task_registry, session_wake_queue) = match backends.session_task_registry {
            Some(inner) => {
                let wake_queue = Arc::new(everruns_core::SessionWakeQueue::new());
                let observing = everruns_core::ObservingTaskRegistry::new(inner)
                    .with_observer(wake_queue.clone());
                let wrapped: Arc<dyn everruns_core::session_task::SessionTaskRegistry> =
                    Arc::new(observing);
                (Some(wrapped), Some(wake_queue))
            }
            None => (None, None),
        };

        let seeded_session_ids = self.sessions.iter().map(|session| session.id).collect();
        let runtime = InProcessRuntime {
            host_composition: Arc::new(self.host_composition),
            harness_store: backends.harness_store,
            agent_store: backends.agent_store,
            session_store: backends.session_store,
            default_session_id: self.default_session_id,
            seeded_session_ids,
            event_log,
            event_history,
            compaction_checkpoint_store: backends.compaction_checkpoint_store,
            provider_store: backends.provider_store,
            event_emitter,
            file_store,
            storage_store: backends.storage_store,
            connection_resolver: backends.connection_resolver,
            session_task_registry,
            session_wake_queue,
            schedule_store_factory: backends.schedule_store_factory,
            platform_store_factory: backends.platform_store_factory,
            #[cfg(feature = "mcp")]
            mcp_auth_provider: self
                .mcp_auth_provider
                .unwrap_or_else(|| Arc::new(everruns_mcp::NoAuthProvider)),
            provider_retry_config: self.provider_retry_config,
            provider_stall_timeout: self.provider_stall_timeout,
            #[cfg(feature = "mcp")]
            mcp_discovery_cache: Arc::new(crate::mcp_cache::McpDiscoveryCache::new()),
            plugin_warnings: self.plugin_warnings,
        };
        for session in &self.sessions {
            runtime.ensure_session_started(session).await?;
        }
        Ok(runtime)
    }
}

async fn resolve_session_file_system(
    host_composition: &HostComposition,
    file_system_factory_context: SessionFileSystemFactoryContext,
) -> Result<Arc<dyn SessionFileSystem>> {
    let file_system_factory = host_composition.session_file_system_factory();
    if file_system_factory.is_disabled() {
        Ok(Arc::new(InMemorySessionFileStore::new()))
    } else {
        Ok(file_system_factory
            .create_session_file_system(file_system_factory_context)
            .await?)
    }
}

#[derive(Clone)]
/// Public in-process runtime backed by either in-memory or custom stores.
///
/// This runtime is intended for embedders who want to execute Everruns
/// harnesses inside their own process while controlling capabilities,
/// harness definitions, and driver registrations directly in Rust.
pub struct InProcessRuntime {
    host_composition: Arc<HostComposition>,
    harness_store: Arc<dyn RuntimeHarnessStore>,
    agent_store: Arc<dyn RuntimeAgentStore>,
    session_store: Arc<dyn RuntimeSessionStore>,
    default_session_id: Option<SessionId>,
    seeded_session_ids: Vec<SessionId>,
    event_log: Arc<dyn EventLog>,
    event_history: Arc<EventHistory>,
    compaction_checkpoint_store: Arc<dyn everruns_core::CompactionCheckpointStore>,
    provider_store: Arc<dyn RuntimeProviderStore>,
    event_emitter: Arc<HostEventEmitter>,
    file_store: Arc<dyn SessionFileSystem>,
    storage_store: Arc<dyn SessionStorageStore>,
    connection_resolver: Option<Arc<dyn UserConnectionResolver>>,
    session_task_registry: Option<Arc<dyn everruns_core::session_task::SessionTaskRegistry>>,
    /// Mid-turn wake queue fed by `session_task_registry` transitions and
    /// drained at each reason iteration boundary (EVE-681, part A). Present iff
    /// a task registry was configured.
    session_wake_queue: Option<Arc<everruns_core::SessionWakeQueue>>,
    schedule_store_factory: Option<crate::backends::ScheduleStoreFactory>,
    platform_store_factory: Option<crate::backends::PlatformStoreFactory>,
    #[cfg(feature = "mcp")]
    mcp_auth_provider: Arc<dyn everruns_mcp::McpAuthProvider>,
    provider_retry_config: Option<everruns_provider::llm_retry::LlmRetryConfig>,
    provider_stall_timeout: Option<std::time::Duration>,
    #[cfg(feature = "mcp")]
    mcp_discovery_cache: Arc<crate::mcp_cache::McpDiscoveryCache>,
    /// Non-fatal warnings collected during plugin compilation (see
    /// [`InProcessRuntimeBuilder::with_plugin_dir`]).
    plugin_warnings: Vec<String>,
}

impl InProcessRuntime {
    async fn ensure_session_started(&self, session: &ExecutionSession) -> Result<()> {
        if self
            .event_history
            .contains_event_type(session.id, everruns_core::events::SESSION_STARTED)
            .await
            .map_err(|error| AgentLoopError::store(error.to_string()))?
        {
            return Ok(());
        }
        self.event_emitter
            .emit(EventRequest::new(
                session.id,
                EventContext::empty(),
                SessionStartedData {
                    harness_id: session.harness_id,
                    agent_id: session.agent_id,
                    model_id: session.model_id,
                },
            ))
            .await?;
        Ok(())
    }

    /// Clone the canonical emitter used by this runtime.
    ///
    /// Facades may retain it to append a correlated terminal event after an
    /// in-flight turn future is dropped. Calls still commit before observation.
    pub fn host_event_emitter(&self) -> Arc<HostEventEmitter> {
        self.event_emitter.clone()
    }

    /// Coherent canonical event log backing this runtime.
    pub fn event_log(&self) -> Arc<dyn EventLog> {
        self.event_log.clone()
    }

    /// Build the shared MCP client over the platform egress boundary and the
    /// configured auth provider.
    #[cfg(feature = "mcp")]
    fn mcp_client(&self) -> Arc<everruns_mcp::McpClient> {
        Arc::new(everruns_mcp::McpClient::new(
            self.host_composition.egress_service(),
            self.mcp_auth_provider.clone(),
        ))
    }

    /// Resolve the effective scoped MCP servers for a session.
    #[cfg(feature = "mcp")]
    async fn session_mcp_servers(
        &self,
        session: &ExecutionSession,
        agent: Option<&AgentDefinition>,
    ) -> everruns_core::ScopedMcpServers {
        let harness = self
            .harness_store
            .get_harness(session.harness_id)
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        let resolved = resolve_runtime_capabilities(
            &harness,
            agent,
            session,
            self.host_composition.capability_registry(),
        );
        let contributed = collect_capability_mcp_servers(
            &resolved.resolved_capability_configs,
            self.host_composition.capability_registry(),
        );
        let explicit = crate::mcp::merge_session_scoped_servers(&harness, agent, session);
        everruns_core::merge_scoped_mcp_servers(&contributed, &explicit)
    }
    /// Create a builder for the in-process runtime.
    pub fn builder() -> InProcessRuntimeBuilder {
        InProcessRuntimeBuilder::new()
    }

    /// Return the default session id seeded by
    /// [`InProcessRuntimeBuilder::single_session`], if one was configured.
    pub fn default_session_id(&self) -> Option<SessionId> {
        self.default_session_id
    }

    /// Return non-fatal warnings collected during plugin compilation.
    ///
    /// Warnings are also emitted at `tracing::warn!` level when
    /// [`InProcessRuntimeBuilder::with_plugin_dir`] is called.
    pub fn plugin_warnings(&self) -> &[String] {
        &self.plugin_warnings
    }

    /// Activate a registered capability on a running session.
    ///
    /// The capability is validated and dependency-resolved before the session
    /// overlay changes. Conversation history and the session identity are
    /// untouched. Re-activating an already-effective capability is a no-op.
    pub async fn activate_capability(
        &self,
        session_id: SessionId,
        capability: impl Into<everruns_capability::CapabilityRef>,
    ) -> Result<CapabilityDelta> {
        let mut capability = capability.into();
        let registry = self.host_composition.capability_registry();
        let registered = registry.get(capability.capability_id()).ok_or_else(|| {
            AgentLoopError::config(format!(
                "unknown capability: {}",
                capability.capability_id()
            ))
        })?;
        if registered.status() != CapabilityStatus::Available {
            return Err(AgentLoopError::config(format!(
                "capability is not available: {}",
                capability.capability_id()
            )));
        }
        registered
            .validate_config(capability.config_value())
            .map_err(|error| {
                AgentLoopError::config(format!("invalid capability config: {error}"))
            })?;

        let canonical_id = registered.id().to_string();
        capability.set_id(canonical_id.clone());
        let context = self.load_context(session_id).await?;
        if context
            .resolved_capability_configs
            .iter()
            .any(|config| config.capability_id() == canonical_id)
        {
            return Ok(CapabilityDelta {
                capability_id: canonical_id,
                changed: false,
                active: true,
                surfaces_dirty: false,
            });
        }

        let mut candidate = context.snapshot.capabilities;
        candidate.push(capability.clone());
        resolve_capability_configs(&candidate, registry)
            .map_err(|error| AgentLoopError::config(error.to_string()))?;

        self.session_store
            .upsert_session_capability(session_id, capability)
            .await?;
        #[cfg(feature = "mcp")]
        self.mcp_discovery_cache
            .invalidate_session(session_id.uuid());
        Ok(CapabilityDelta {
            capability_id: canonical_id,
            changed: true,
            active: true,
            surfaces_dirty: true,
        })
    }

    /// Deactivate a capability previously activated on this running session.
    ///
    /// Capabilities inherited from the agent or harness cannot be removed by a
    /// session-scoped operation; callers must change their owning layer.
    pub async fn deactivate_capability(
        &self,
        session_id: SessionId,
        capability_id: &str,
    ) -> Result<CapabilityDelta> {
        let registry = self.host_composition.capability_registry();
        let registered = registry.get(capability_id).ok_or_else(|| {
            AgentLoopError::config(format!("unknown capability: {capability_id}"))
        })?;
        let canonical_id = registered.id().to_string();
        let context = self.load_context(session_id).await?;
        if !context
            .resolved_capability_configs
            .iter()
            .any(|config| config.capability_id() == canonical_id)
        {
            return Ok(CapabilityDelta {
                capability_id: canonical_id,
                changed: false,
                active: false,
                surfaces_dirty: false,
            });
        }

        let session = self
            .session_store
            .get_session(session_id)
            .await?
            .ok_or_else(|| AgentLoopError::session_not_found(session_id))?;
        let session_capability_id = session
            .capabilities
            .iter()
            .find(|config| {
                registry
                    .get(config.capability_id())
                    .is_some_and(|capability| capability.id() == canonical_id)
            })
            .map(|config| config.capability_id().to_string())
            .ok_or_else(|| {
                AgentLoopError::config(format!(
                    "capability {canonical_id} is inherited and cannot be deactivated at the session layer"
                ))
            })?;

        self.session_store
            .remove_session_capability(session_id, &session_capability_id)
            .await?;
        #[cfg(feature = "mcp")]
        self.mcp_discovery_cache
            .invalidate_session(session_id.uuid());
        Ok(CapabilityDelta {
            capability_id: canonical_id,
            changed: true,
            active: false,
            surfaces_dirty: true,
        })
    }

    /// Execute one turn for an existing session.
    ///
    /// The input message is appended as the canonical `input.message` event;
    /// [`EventHistory`] derives the read projection from that one write. The
    /// turn then runs `input -> reason -> act` as planned step-by-step by
    /// [`everruns_engine`] — the same planner the durable worker drives.
    pub async fn run_turn(
        &self,
        session_id: SessionId,
        input: impl Into<InputMessage>,
    ) -> Result<TurnResult> {
        self.run_steerable_turn(
            session_id,
            AcceptedTurnInput::new(input),
            TurnId::new(),
            TurnSteering::new(),
        )
        .await
    }

    /// Execute one turn while accepting additional user messages at reason
    /// boundaries.
    #[doc(hidden)]
    pub async fn run_steerable_turn(
        &self,
        session_id: SessionId,
        input: AcceptedTurnInput,
        turn_id: TurnId,
        steering: TurnSteering,
    ) -> Result<TurnResult> {
        // EVE-872: construct the canonical resolved execution snapshot before
        // turn planning. Missing or inactive records fail here, during
        // platform projection, and stored records never feed planning.
        let snapshot = self.resolved_execution_snapshot(session_id).await?;

        // The canonical input envelope is the only write. EventHistory rebuilds
        // the message projection from this accepted append.
        let input_message = input.into_message();
        self.event_emitter
            .emit(EventRequest::new(
                session_id,
                EventContext::empty(),
                InputMessageData::new(input_message.clone()),
            ))
            .await?;

        let org_id = in_process_internal_org_id(&snapshot.organization_id);

        // Engine-planned turn loop (EVE-842). Every reason-vs-act-vs-complete
        // decision comes from `everruns-engine`; this loop only executes the
        // host operation each plan names and performs the lifecycle effects the
        // engine returns as data. There is no second copy of the planning brain
        // in the runtime.
        let mut state = RuntimeTurnState {
            org_id,
            session_id,
            harness_id: snapshot.harness_id,
            agent_id: snapshot.agent_id,
            input_message_id: input_message.id,
            turn_id: None,
            previous_response_id: None,
            iteration: 1,
            request_id: None,
            started_at: None,
            cumulative_usage: None,
            tool_call_count: 0,
            llm_call_count: 0,
            time_to_first_token_ms: None,
            final_message_id: None,
            final_answer_preview: None,
        };

        let base_context = |exec: bool| {
            let context = AtomContext::new(session_id, turn_id, input_message.id)
                .with_workspace_id(snapshot.workspace_id);
            if exec { context.next_exec() } else { context }
        };

        // `process_input` is the turn's fixed entry step: the durable host
        // enqueues it before any planning, and it is what mints the turn id the
        // planner then carries.
        execute_input_activity(
            self,
            org_id,
            InputAtomInput {
                context: base_context(false),
            },
        )
        .await?;
        let mut plan = plan_after_process_input(&state, Some(turn_id), Utc::now());

        // Host-side bookkeeping for the returned `TurnResult`. These are
        // summaries of what the host executed, not inputs to any decision.
        let mut iterations: usize = 0;
        let mut tool_calls_count: usize = 0;
        let mut last_response = String::new();
        let mut pending_prompt_message_ids = Vec::new();

        loop {
            match plan {
                RuntimeTurnPlan::ScheduleReason(next_state) => {
                    state = next_state;
                    let mut prompt_message_ids = std::mem::take(&mut pending_prompt_message_ids);
                    prompt_message_ids.extend(
                        self.inject_steering_inputs(session_id, steering.drain())
                            .await?,
                    );
                    // Iteration boundary: drain queued task wakes and inject
                    // them before the LLM call so this reason reacts to them
                    // (EVE-681, part A). Draining here also delivers wakes that
                    // arrived while the session was idle, on the next turn's
                    // first iteration (between-turn fallback).
                    prompt_message_ids.extend(self.drain_and_inject_wakes(session_id).await?);
                    if state.iteration == 1 {
                        prompt_message_ids.insert(0, input_message.id);
                    }
                    let reason_result = execute_reason_activity_with_prompt_messages(
                        self,
                        org_id,
                        ReasonInput {
                            context: base_context(true),
                            harness_id: snapshot.harness_id,
                            agent_id: snapshot.agent_id,
                            org_id,
                            mcp_tool_definitions: vec![],
                            previous_response_id: state.previous_response_id.clone(),
                            iteration: state.iteration,
                        },
                        prompt_message_ids,
                    )
                    .await?;

                    iterations += 1;
                    if !reason_result.text.is_empty() {
                        last_response = reason_result.text.clone();
                    }

                    // Only a reason that would otherwise finish may close user
                    // ingress. The queue check and close are atomic, so a send
                    // is always classified as either steering this turn or
                    // starting the next one.
                    let pending_wake_count = usize::from(
                        self.session_wake_queue
                            .as_ref()
                            .is_some_and(|q| q.has_pending(session_id)),
                    );
                    let can_continue = reason_result.success
                        && state.iteration < reason_result.max_iterations as u32;
                    let pending_steering = if !reason_schedules_act(&state, &reason_result)
                        && can_continue
                        && pending_wake_count == 0
                    {
                        steering.drain_or_close()
                    } else {
                        vec![]
                    };
                    let pending_steering_count = pending_steering.len();
                    if !pending_steering.is_empty() {
                        pending_prompt_message_ids.extend(
                            self.inject_steering_inputs(session_id, pending_steering)
                                .await?,
                        );
                    }
                    if !can_continue {
                        steering.close();
                    }
                    let pending_user_message_count = pending_wake_count + pending_steering_count;

                    let act_scheduling =
                        crate::turn_strategy::resolve_act_scheduling(self, &state, &reason_result)
                            .await?;
                    let (next, effects) = plan_after_reason(
                        &state,
                        reason_result,
                        pending_user_message_count,
                        Utc::now(),
                        act_scheduling,
                    );
                    crate::turn_strategy::perform_effects(self, org_id, session_id, effects).await;
                    plan = next;
                }
                RuntimeTurnPlan::ScheduleAct(act_plan) => {
                    tool_calls_count += act_plan.input.tool_calls.len();
                    // Same resume contract the durable host applies when it
                    // dequeues the act task: the plan's response id / iteration
                    // / request id override the carried state.
                    state = *act_plan.resume_state;
                    state.previous_response_id = act_plan.previous_response_id;
                    state.iteration = act_plan.iteration;
                    state.request_id = act_plan.request_id;

                    let act_result = execute_act_activity(self, act_plan.input).await?;
                    let outcome = ActOutcome {
                        blocked: act_result.blocked,
                        waiting_for_tool_results: act_result.waiting_for_tool_results,
                    };
                    let setup_connection_hint_enabled =
                        crate::turn_strategy::resolve_setup_connection_hint(
                            self, org_id, session_id, outcome,
                        )
                        .await;
                    let (next, effects) =
                        plan_after_act(&state, outcome, setup_connection_hint_enabled);
                    crate::turn_strategy::perform_effects(self, org_id, session_id, effects).await;
                    plan = next;
                }
                RuntimeTurnPlan::Complete { stop_reason, error } => {
                    steering.close();
                    return Ok(finish_turn(
                        turn_id,
                        stop_reason,
                        error,
                        last_response,
                        iterations,
                        tool_calls_count,
                    ));
                }
                // The in-process runtime has no external tool-result delivery
                // path, so a pause resolves the turn here. The session has
                // already been marked `waiting_for_tool_results` by the effect.
                RuntimeTurnPlan::WaitForToolResults { .. } => {
                    steering.close();
                    self.inject_steering_inputs(session_id, steering.drain())
                        .await?;
                    return Ok(finish_turn(
                        turn_id,
                        TurnStopReason::EndTurn,
                        None,
                        last_response,
                        iterations,
                        tool_calls_count,
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

    async fn inject_steering_inputs(
        &self,
        session_id: SessionId,
        inputs: Vec<AcceptedTurnInput>,
    ) -> Result<Vec<MessageId>> {
        let mut message_ids = Vec::with_capacity(inputs.len());
        for input in inputs {
            let message = input.into_message();
            message_ids.push(message.id);
            self.event_emitter
                .emit(EventRequest::new(
                    session_id,
                    EventContext::empty(),
                    InputMessageData::new(message),
                ))
                .await?;
        }
        Ok(message_ids)
    }

    /// Persist accepted steering that could not reach another reason boundary.
    #[doc(hidden)]
    pub async fn append_accepted_inputs(
        &self,
        session_id: SessionId,
        inputs: Vec<AcceptedTurnInput>,
    ) -> Result<()> {
        self.inject_steering_inputs(session_id, inputs)
            .await
            .map(|_| ())
    }

    /// Drain any queued task wakes for `session_id` and inject them into the
    /// conversation as user messages so the next reason reacts to them
    /// (EVE-681, part A). Returns the ids of the injected messages so prompt
    /// hooks can inspect them before the next provider call.
    ///
    /// Called at the top of every reason iteration — before the LLM call — so a
    /// task completion that landed during the previous act (or while idle) is
    /// visible to the very next iteration. `SessionWakeQueue::drain` is the
    /// exactly-once claim point: a drained wake is removed and never delivered
    /// twice, so a wake is delivered mid-turn XOR on the next turn's first
    /// drain, never both.
    async fn drain_and_inject_wakes(&self, session_id: SessionId) -> Result<Vec<MessageId>> {
        let Some(queue) = &self.session_wake_queue else {
            return Ok(vec![]);
        };
        let wakes = queue.drain(session_id);
        if wakes.is_empty() {
            return Ok(vec![]);
        }
        let mut message_ids = Vec::with_capacity(wakes.len());
        for wake in wakes {
            // Append one canonical input event; history is reconstructed from it.
            let message = message_from_input(InputMessage::user(wake.text));
            message_ids.push(message.id);
            self.event_emitter
                .emit(EventRequest::new(
                    session_id,
                    EventContext::empty(),
                    InputMessageData::new(message),
                ))
                .await?;
        }
        Ok(message_ids)
    }

    /// Load the current message history for a session.
    pub async fn messages(&self, session_id: SessionId) -> Result<Vec<Message>> {
        self.event_history.load(session_id).await
    }

    /// Read a file from the in-memory session filesystem.
    pub async fn read_file(
        &self,
        session_id: SessionId,
        path: &str,
    ) -> Result<Option<SessionFile>> {
        self.file_store.read_file(session_id, path).await
    }

    /// Assemble the current runtime context for a session without executing a turn.
    pub async fn load_context(&self, session_id: SessionId) -> Result<AssembledTurnContext> {
        let session = self
            .session_store
            .get_session(session_id)
            .await?
            .ok_or_else(|| AgentLoopError::store(format!("session not found: {session_id}")))?;
        #[cfg(feature = "mcp")]
        let agent = match session.agent_id {
            Some(agent_id) => self.agent_store.get_agent(agent_id).await?,
            None => None,
        };
        #[cfg(feature = "mcp")]
        let scoped_servers = self.session_mcp_servers(&session, agent.as_ref()).await;
        #[cfg(feature = "mcp")]
        let mcp_tool_definitions = if scoped_servers.is_empty() {
            vec![]
        } else {
            crate::mcp::discover_tool_definitions(
                &self.mcp_discovery_cache,
                self.mcp_client(),
                session_id.uuid(),
                &scoped_servers,
            )
            .await
        };
        #[cfg(not(feature = "mcp"))]
        let mcp_tool_definitions = vec![];
        self.inspect_context_with_ids(
            session_id,
            session.harness_id,
            session.agent_id,
            &mcp_tool_definitions,
        )
        .await
    }

    /// Return canonical durable events collected by this runtime's log.
    ///
    /// This 0.17 convenience paginates through the bounded host SPI and fails
    /// once [`crate::events::MAX_EVENT_HISTORY_REPLAY`] envelopes are reached.
    pub async fn events(&self) -> Result<Vec<Event>> {
        let limit = EventReadLimit::default();
        let session_id = self
            .default_session_id
            .or_else(|| (self.seeded_session_ids.len() == 1).then(|| self.seeded_session_ids[0]))
            .ok_or_else(|| {
                AgentLoopError::config(
                    "events() requires exactly one seeded session; use event_log() for bounded per-session replay",
                )
            })?;
        let mut request = EventReadRequest::new(session_id, limit);
        let mut events = Vec::new();
        loop {
            let page = self
                .event_log
                .read_page(request)
                .await
                .map_err(|error| AgentLoopError::store(error.to_string()))?;
            if events.len().saturating_add(page.events.len())
                > crate::events::MAX_EVENT_HISTORY_REPLAY
            {
                return Err(AgentLoopError::store("event replay bound exceeded"));
            }
            events.extend(page.events);
            let Some(cursor) = page.next_cursor else {
                return Ok(events);
            };
            request = EventReadRequest::from_cursor(cursor, limit);
        }
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
        request: everruns_core::command::ExecuteCommandRequest,
    ) -> Result<everruns_core::command::CommandResult> {
        let ctx = self.load_context(session_id).await?;
        let registry = self.host_composition.capability_registry();
        // Context-aware commands (e.g. /btw) get the same store-backed host
        // facilities the server provides; the already-assembled context seeds
        // the host so dispatch and execution assemble it once.
        let host = crate::StoreCommandHost::new(
            session_id,
            self.harness_store.clone(),
            self.agent_store.clone(),
            self.session_store.clone(),
            self.event_history.clone(),
            self.provider_store.clone(),
            registry.clone(),
            self.host_composition.driver_registry().clone(),
        )
        .with_file_store(self.file_store.clone())
        .with_assembled_context(ctx.clone());
        let exec_ctx =
            everruns_core::command::CommandExecutionContext::new(session_id, Arc::new(host));
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
    ) -> Result<Vec<everruns_core::command::CommandDescriptor>> {
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

    /// Load a session record and fold runtime ARD attachments into its config
    /// layer before capabilities / scoped MCP servers are resolved
    /// (resource_discovery).
    async fn attached_session(&self, session_id: SessionId) -> Result<ExecutionSession> {
        let mut session = self
            .session_store
            .get_session(session_id)
            .await?
            .ok_or_else(|| AgentLoopError::store(format!("session not found: {session_id}")))?;
        everruns_core::ard_attachment::apply_session_attachments(
            self.storage_store.as_ref(),
            &mut session,
        )
        .await;
        Ok(session)
    }

    /// Project the canonical resolved execution snapshot for a session
    /// (EVE-872). This is the same platform projection the hosted workers
    /// apply, so Framework and hosted execution consume one execution value.
    async fn resolved_execution_snapshot(
        &self,
        session_id: SessionId,
    ) -> Result<ResolvedExecutionSnapshot> {
        let session = self.attached_session(session_id).await?;
        crate::load_execution_snapshot_for_session(
            self.harness_store.as_ref(),
            self.agent_store.as_ref(),
            &session,
        )
        .await
    }

    async fn inspect_context_with_ids(
        &self,
        session_id: SessionId,
        harness_id: everruns_provider::typed_id::HarnessId,
        agent_id: Option<AgentId>,
        mcp_tool_definitions: &[everruns_provider::tool_types::ToolDefinition],
    ) -> Result<AssembledTurnContext> {
        crate::inspect_turn_context(
            self.harness_store.as_ref(),
            self.agent_store.as_ref(),
            self.session_store.as_ref(),
            self.event_history.as_ref(),
            self.provider_store.as_ref(),
            self.host_composition.capability_registry(),
            self.host_composition.driver_registry(),
            session_id,
            harness_id,
            agent_id,
            mcp_tool_definitions,
            Some(self.file_store.clone()),
        )
        .await
    }
}

#[async_trait]
impl RuntimeHostAdapter for InProcessRuntime {
    async fn set_session_status(
        &self,
        _org_id: i64,
        session_id: SessionId,
        _status: SessionExecutionState,
    ) -> Result<()> {
        // The in-process runtime does not persist status. Lifecycle callers
        // still emit their events; downstream consumers in-process don't
        // observe session.status. Keep the existence check so lifecycle
        // warnings still surface a missing session.
        self.session_store
            .get_session(session_id)
            .await?
            .ok_or_else(|| AgentLoopError::store(format!("session not found: {session_id}")))?;
        Ok(())
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
        let messages = self.event_history.load(session_id).await?;

        // Discover tools from the session's scoped MCP servers so they appear
        // to the LLM alongside built-in tools (knowledge/integrations/runtime-mcp.md D4).
        #[cfg(feature = "mcp")]
        let scoped_servers = self.session_mcp_servers(&session, agent.as_ref()).await;
        #[cfg(feature = "mcp")]
        let mcp_tool_definitions = if scoped_servers.is_empty() {
            vec![]
        } else {
            crate::mcp::discover_tool_definitions(
                &self.mcp_discovery_cache,
                self.mcp_client(),
                session_id.uuid(),
                &scoped_servers,
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

    #[cfg(feature = "mcp")]
    async fn mcp_executor(
        &self,
        _org_id: i64,
        session_id: SessionId,
    ) -> Option<Arc<dyn everruns_core::McpToolInvoker>> {
        let session = self.session_store.get_session(session_id).await.ok()??;
        let agent = match session.agent_id {
            Some(agent_id) => self.agent_store.get_agent(agent_id).await.ok().flatten(),
            None => None,
        };
        let scoped_servers = self.session_mcp_servers(&session, agent.as_ref()).await;
        crate::mcp::build_executor(self.mcp_client(), &scoped_servers)
            .map(|executor| executor as Arc<dyn everruns_core::McpToolInvoker>)
    }

    fn capability_registry(&self) -> CapabilityRegistry {
        self.host_composition.capability_registry().clone()
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

    fn compaction_checkpoint_store(
        &self,
    ) -> Option<Arc<dyn everruns_core::CompactionCheckpointStore>> {
        Some(self.compaction_checkpoint_store.clone())
    }

    fn event_emitter(&self) -> Arc<dyn EventEmitter> {
        self.event_emitter.clone()
    }

    fn file_store(&self) -> Arc<dyn SessionFileSystem> {
        self.file_store.clone()
    }

    fn storage_store(&self) -> Option<Arc<dyn SessionStorageStore>> {
        Some(self.storage_store.clone())
    }

    fn connection_resolver(&self) -> Option<Arc<dyn UserConnectionResolver>> {
        self.connection_resolver.clone()
    }

    fn session_task_registry(
        &self,
    ) -> Option<Arc<dyn everruns_core::session_task::SessionTaskRegistry>> {
        self.session_task_registry.clone()
    }

    fn schedule_store(
        &self,
        org_id: i64,
    ) -> Option<Arc<dyn everruns_core::session_services::SessionScheduleStore>> {
        self.schedule_store_factory
            .as_ref()
            .map(|factory| factory(org_id))
    }

    fn platform_store(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Option<Arc<dyn everruns_platform::PlatformStore>> {
        self.platform_store_factory
            .as_ref()
            .map(|factory| factory(org_id, session_id))
    }

    fn utility_llm_service(&self) -> Option<Arc<dyn everruns_core::UtilityLlmService>> {
        Some(self.host_composition.utility_llm_service())
    }

    fn egress_service(&self) -> Option<Arc<dyn everruns_core::EgressService>> {
        Some(self.host_composition.egress_service())
    }

    fn provider_retry_config(&self) -> Option<everruns_provider::llm_retry::LlmRetryConfig> {
        self.provider_retry_config.clone()
    }

    fn provider_stall_timeout(&self) -> Option<std::time::Duration> {
        self.provider_stall_timeout
    }
}

fn effective_overlay(
    harness: &HarnessDefinition,
    agent: Option<&AgentDefinition>,
    session: &ExecutionSession,
) -> AgentConfigOverlay {
    let agent_layers = agent.into_iter().map(AgentConfigOverlay::from);
    AgentConfigOverlay::fold(
        [AgentConfigOverlay::from(harness)]
            .into_iter()
            .chain(agent_layers)
            .chain([AgentConfigOverlay::from(session)]),
    )
}

/// Replace bare `plugin:{name}` refs (empty config) with the hydrated version
/// (config = serialised `DeclarativeCapabilityDefinition`) so that the core
/// capability resolution path can deserialise them without a registry entry.
///
/// Only replaces entries whose config is empty / `null`; entries that already
/// carry a non-empty config are left unchanged so explicit overrides are honoured.
fn hydrate_plugin_refs(
    capabilities: &mut [everruns_capability::CapabilityRef],
    plugin_configs: &[everruns_capability::CapabilityRef],
) {
    for cap in capabilities.iter_mut() {
        let cap_id = cap.capability_id();
        if !everruns_capability::is_plugin_capability(cap_id) {
            continue;
        }
        // Only replace if the config is missing / empty so explicit overrides are honoured.
        let is_bare = cap.config_value().is_null()
            || cap
                .config_value()
                .clone()
                .as_object()
                .map(|o| o.is_empty())
                .unwrap_or(false);
        if !is_bare {
            continue;
        }
        if let Some(hydrated) = plugin_configs.iter().find(|c| c.capability_id() == cap_id) {
            cap.set_config(hydrated.config_value().clone());
        }
    }
}

async fn seed_runtime_initial_files(
    harness_store: &dyn RuntimeHarnessStore,
    agent_store: &dyn RuntimeAgentStore,
    file_store: &dyn SessionFileSystem,
    session: &ExecutionSession,
) -> Result<()> {
    let harness = harness_store
        .get_harness(session.harness_id)
        .await?
        .ok_or_else(|| {
            AgentLoopError::store(format!(
                "harness not found while seeding files: {}",
                session.harness_id
            ))
        })?;
    let agent = match session.agent_id {
        Some(agent_id) => Some(
            agent_store
                .get_agent(agent_id)
                .await?
                .ok_or_else(|| AgentLoopError::store(format!("agent not found: {agent_id}")))?,
        ),
        None => None,
    };
    let overlay = effective_overlay(&harness, agent.as_ref(), session);
    // Seed into the session's workspace (a shared workspace differs from the
    // session id; the default 1:1 case is equal).
    let seed_key = SessionId::from_uuid(session.workspace_id.uuid());
    for file in &overlay.initial_files {
        file_store.seed_initial_file(seed_key, file).await?;
    }
    Ok(())
}

fn message_from_input(input: InputMessage) -> Message {
    message_from_input_with_id(MessageId::new(), input)
}

fn message_from_input_with_id(message_id: MessageId, input: InputMessage) -> Message {
    Message {
        id: message_id,
        role: input.role,
        content: input.content,
        phase: None,
        thinking: None,
        thinking_signature: None,
        controls: input.controls,
        metadata: input.metadata,
        external_actor: None,
        created_at: Utc::now(),
    }
}

#[cfg(test)]
mod org_id_mapping_tests {
    use super::*;
    use everruns_core::{DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, org_public_id_from_internal};

    #[test]
    fn default_public_id_maps_to_default_org() {
        assert_eq!(
            in_process_internal_org_id(DEFAULT_ORG_PUBLIC_ID),
            DEFAULT_ORG_ID
        );
    }

    #[test]
    fn invalid_public_id_does_not_fall_back_to_default() {
        for invalid in [
            "",
            "not-an-org",
            "org_short",
            "org_ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ",
            "ORG_00000000000000000000000000000001",
        ] {
            let mapped = in_process_internal_org_id(invalid);
            assert_ne!(mapped, everruns_core::DEFAULT_ORG_ID);
            assert!(
                mapped >= 2,
                "invalid input {invalid:?} should not map to default"
            );
        }
    }

    #[test]
    fn zero_public_id_does_not_fall_back_to_default() {
        // org_public_id_from_internal never produces this; a hand-crafted
        // all-zeros id is treated as invalid (raw == 0).
        let mapped = in_process_internal_org_id("org_00000000000000000000000000000000");
        assert_ne!(mapped, everruns_core::DEFAULT_ORG_ID);
        assert!(mapped >= 2, "all-zero id should not map to default");
    }

    #[test]
    fn synthetic_public_id_round_trips_with_internal_helper() {
        for internal in [1_i64, 2, 42, 1_000_000, i64::MAX - 1, i64::MAX] {
            let public = org_public_id_from_internal(internal);
            assert_eq!(
                in_process_internal_org_id(&public),
                internal,
                "round-trip failed for internal={internal}"
            );
        }
    }

    #[test]
    fn distinct_synthetic_ids_map_to_distinct_internal_ids() {
        let a = org_public_id_from_internal(7);
        let b = org_public_id_from_internal(8);
        assert_ne!(a, b);
        assert_ne!(
            in_process_internal_org_id(&a),
            in_process_internal_org_id(&b)
        );
    }

    #[test]
    fn high_entropy_uuid_style_id_hashes_into_reserved_range() {
        // First valid UUID-style id whose raw u128 exceeds i64::MAX
        // (top bit of the u128 set). It must hash to a positive i64 that
        // is neither 0 nor DEFAULT_ORG_ID.
        let high = "org_80000000000000000000000000000000";
        let mapped = in_process_internal_org_id(high);
        assert!(mapped >= 2, "mapped id {mapped} must be >= 2");
        assert_ne!(mapped, DEFAULT_ORG_ID);

        // Mapping is deterministic.
        assert_eq!(mapped, in_process_internal_org_id(high));
    }

    #[test]
    fn high_entropy_ids_are_isolated_from_each_other() {
        let a = in_process_internal_org_id("org_80000000000000000000000000000001");
        let b = in_process_internal_org_id("org_80000000000000000000000000000002");
        assert_ne!(a, b);
        assert_ne!(a, DEFAULT_ORG_ID);
        assert_ne!(b, DEFAULT_ORG_ID);
    }

    #[test]
    fn hash_uses_stable_sha256_truncation() {
        // SHA-256 with fixed big-endian first-8-byte truncation gives a value
        // we can pin. If this assertion ever breaks, callers depending on
        // build-stable mapping must be re-audited.
        let mapped = in_process_internal_org_id("org_80000000000000000000000000000000");
        let expected = {
            let digest = sha2::Sha256::digest(b"org_80000000000000000000000000000000");
            let mut buf = [0u8; 8];
            buf.copy_from_slice(&digest[..8]);
            let raw = u64::from_be_bytes(buf);
            ((raw % ((i64::MAX - 1) as u64)) as i64) + 2
        };
        assert_eq!(mapped, expected);
    }

    #[test]
    fn oversize_input_is_bounded_and_does_not_collide_silently() {
        // Inputs past HASH_INPUT_CAP_BYTES are truncated before hashing, so
        // two oversize strings that agree on the first cap bytes map to the
        // same internal id. We only assert the result stays in the safe
        // [2, i64::MAX] range and is not DEFAULT_ORG_ID — the cap exists to
        // bound work, not to widen the input space.
        let oversize = "x".repeat(super::HASH_INPUT_CAP_BYTES * 4);
        let mapped = in_process_internal_org_id(&oversize);
        assert!(mapped >= 2);
        assert_ne!(mapped, DEFAULT_ORG_ID);
    }
}
