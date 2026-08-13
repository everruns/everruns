//! Value-first agent description (EVE-832).
//!
//! [`Agent::builder`] lets a library user describe an agent — instructions, a
//! model, optional tools and files — without constructing stored `Harness`,
//! `Agent`, `Session`, IDs, timestamps, statuses, registries, or a
//! `HostComposition`. The builder validates the value-first configuration and
//! adapts it, inside [`AgentBuilder::build`], to the existing runtime builders.
//!
//! The built Agent owns the configured session lifecycle: it opens independent
//! sessions, shares their private catalog and canonical event log across Agent
//! clones, and can resume typed session identities while that lifecycle remains
//! available.

use std::collections::HashSet;
use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use everruns_core::{AgentCapabilityConfig, InitialFile, ModelSpec, Provider, SessionId};
use everruns_host::{
    AgentBuilder as RuntimeAgentBuilder, EventLogError, EventSink, HarnessBuilder, HostBackends,
    InProcessRuntime, InProcessRuntimeBuilder, SessionBuilder,
};
use everruns_test_support::{LlmSimConfig, LlmSimDriver};
use tokio::sync::OnceCell;

use crate::tool::{FunctionTool, IntoTool, Tool, validate_tool_name, validate_tool_schema};

/// A model selected for an [`Agent`].
///
/// Live models are named with a plain provider-visible id and configured with a
/// separate [`AgentBuilder::provider`] call. [`Model::simulated`] bundles its
/// private offline provider so deterministic agents need no extra setup.
#[derive(Clone)]
pub struct Model {
    id: String,
    bundled_provider: Option<Provider>,
}

impl Model {
    /// A deterministic in-process model that always replies with `response`.
    ///
    /// Backed by the `llmsim` driver, so an agent using it runs entirely
    /// offline — no credentials, no network.
    pub fn simulated(response: impl Into<String>) -> Self {
        Self::bundled(
            "llmsim-model",
            Provider::new("llmsim", LlmSimDriver::new(LlmSimConfig::fixed(response))),
        )
    }

    /// A deterministic in-process model configured with a scripted simulator.
    ///
    /// This keeps examples and evaluations offline while allowing multiple
    /// responses, tool-call sequences, and the other controls exposed by
    /// [`LlmSimConfig`].
    pub fn simulated_with_config(config: LlmSimConfig) -> Self {
        Self::bundled(
            "llmsim-model",
            Provider::new("llmsim", LlmSimDriver::new(config)),
        )
    }

    fn bundled(id: impl Into<String>, provider: Provider) -> Self {
        Self {
            id: id.into(),
            bundled_provider: Some(provider),
        }
    }
}

impl From<&str> for Model {
    fn from(id: &str) -> Self {
        Self {
            id: id.to_string(),
            bundled_provider: None,
        }
    }
}

impl From<String> for Model {
    fn from(id: String) -> Self {
        Self {
            id,
            bundled_provider: None,
        }
    }
}

#[cfg(test)]
impl Model {
    /// Test-only: a simulated model that records the provider-visible messages
    /// of every LLM call into `capture`, in call order. Lets session tests
    /// assert what history reached the provider on the second turn.
    pub(crate) fn simulated_capturing(
        response: impl Into<String>,
        capture: std::sync::Arc<std::sync::Mutex<Vec<Vec<everruns_core::LlmMessage>>>>,
    ) -> Self {
        let mut sim = LlmSimConfig::fixed(response);
        sim.message_capture = Some(capture);
        Self::bundled(
            "llmsim-model",
            Provider::new("llmsim", LlmSimDriver::new(sim)),
        )
    }

    /// Test-only: a simulated model that waits `delay` (a TTFT delay) before
    /// producing `response`. The open window lets a facade test cancel a turn
    /// while it is parked, exercising the cancellation path deterministically.
    pub(crate) fn simulated_delayed(
        response: impl Into<String>,
        delay: std::time::Duration,
    ) -> Self {
        let sim = LlmSimConfig::fixed(response).with_response_delay(delay);
        Self::bundled(
            "llmsim-model",
            Provider::new("llmsim", LlmSimDriver::new(sim)),
        )
    }

    /// Test-only: a simulated provider failure used to prove the facade event
    /// bridge retains failure lifecycle and diagnostic payloads end to end.
    pub(crate) fn simulated_error(message: impl Into<String>) -> Self {
        Self::bundled(
            "llmsim-model",
            Provider::new("llmsim", LlmSimDriver::new(LlmSimConfig::error(message))),
        )
    }

    /// Test-only: a simulated model that emits the given per-turn tool-call
    /// sequence before replying with `response`. Lets a facade test drive the
    /// end-to-end tool-execution loop for a function tool.
    pub(crate) fn simulated_scripted(
        response: impl Into<String>,
        tool_call_sequence: Vec<Vec<everruns_core::ToolCall>>,
    ) -> Self {
        let sim = LlmSimConfig::fixed(response).with_tool_call_sequence(tool_call_sequence);
        Self::bundled(
            "llmsim-model",
            Provider::new("llmsim", LlmSimDriver::new(sim)),
        )
    }
}

impl fmt::Debug for Model {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Model")
            .field("id", &self.id)
            .field("bundled_provider", &self.bundled_provider)
            .finish()
    }
}

/// Why an [`AgentBuilder`] could not produce an [`Agent`].
///
/// Stable, typed, and cheap to match on — no backend error leaks through.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BuildError {
    /// `instructions` was empty or only whitespace.
    BlankInstructions,
    /// No model was selected.
    MissingModel,
    /// A tool name is not a valid model-facing identifier.
    InvalidToolName {
        /// The rejected tool name.
        name: String,
        /// Why the name was rejected.
        reason: String,
    },
    /// A tool's JSON argument schema is invalid.
    InvalidToolSchema {
        /// The tool whose schema was rejected.
        name: String,
        /// Why the schema was rejected.
        reason: String,
    },
    /// Two tools were registered under the same name.
    DuplicateTool {
        /// The colliding tool name.
        name: String,
    },
    /// A capability identifier, configuration, or public descriptor is invalid.
    InvalidCapability {
        /// The rejected capability id.
        id: String,
        /// Why the descriptor was rejected.
        reason: String,
    },
    /// Two capability inputs resolve to the same stable implementation id.
    DuplicateCapability {
        /// The colliding capability id.
        id: String,
    },
    /// A live model was selected without a provider.
    MissingProvider,
    /// More than one provider was attached to the agent.
    MultipleProviders { registered: Vec<String> },
    /// MCP server configuration was invalid or duplicated.
    InvalidMcpServer { reason: String },
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BuildError::BlankInstructions => {
                write!(f, "agent instructions must not be blank")
            }
            BuildError::MissingModel => write!(f, "agent requires a model"),
            BuildError::InvalidToolName { name, reason } => {
                write!(f, "invalid tool name {name:?}: {reason}")
            }
            BuildError::InvalidToolSchema { name, reason } => {
                write!(f, "invalid JSON schema for tool {name:?}: {reason}")
            }
            BuildError::DuplicateTool { name } => {
                write!(f, "duplicate tool name {name:?}")
            }
            BuildError::InvalidCapability { id, reason } => {
                write!(f, "invalid capability {id:?}: {reason}")
            }
            BuildError::DuplicateCapability { id } => {
                write!(f, "duplicate capability id {id:?}")
            }
            BuildError::MissingProvider => write!(f, "agent requires a provider for a live model"),
            BuildError::MultipleProviders { registered } => write!(
                f,
                "agent accepts one provider; registered providers: [{}]",
                registered.join(", ")
            ),
            BuildError::InvalidMcpServer { reason } => {
                write!(f, "invalid MCP server configuration: {reason}")
            }
        }
    }
}

impl std::error::Error for BuildError {}

/// The immutable, validated description of an agent.
///
/// Produced by [`AgentBuilder::build`]. It holds the value-first configuration
/// and materializes independent in-process runtimes from it, one per
/// [`session`](Agent::session); the underlying runtime composition is kept in
/// private fields.
#[derive(Clone)]
pub struct Agent {
    name: String,
    instructions: String,
    model: ModelSpec,
    provider: Provider,
    capabilities: Vec<AgentCapabilityConfig>,
    capability_implementations: Vec<CapabilityImplementation>,
    initial_files: Vec<InitialFile>,
    max_iterations: Option<usize>,
    parallel_tool_calls: Option<bool>,
    workspace_root: Option<PathBuf>,
    workspace_policy: everruns_core::WorkspacePolicy,
    mcp_servers: everruns_core::ScopedMcpServers,
    plugin_warnings: Vec<String>,
    #[cfg(feature = "local")]
    local: Option<crate::LocalConfig>,
    lifecycle_hooks: crate::hooks::LifecycleHooks,
    state: Arc<AgentState>,
}

#[derive(Clone)]
enum CapabilityImplementation {
    Function(FunctionTool),
    #[cfg(feature = "capabilities")]
    Definition(crate::capability::Definition),
}

impl CapabilityImplementation {
    fn register(&self, builder: InProcessRuntimeBuilder) -> InProcessRuntimeBuilder {
        match self {
            Self::Function(tool) => builder.capability(tool.clone().into_capability()),
            #[cfg(feature = "capabilities")]
            Self::Definition(definition) => {
                builder.capability(crate::capability::runtime_adapter(definition))
            }
        }
    }
}

impl fmt::Debug for CapabilityImplementation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Function(tool) => formatter
                .debug_tuple("Function")
                .field(&tool.name())
                .finish(),
            #[cfg(feature = "capabilities")]
            Self::Definition(definition) => formatter
                .debug_tuple("Definition")
                .field(&definition.id())
                .finish(),
        }
    }
}

struct AgentState {
    backends: OnceCell<HostBackends>,
    issued_sessions: Mutex<HashSet<SessionId>>,
}

impl AgentState {
    fn new() -> Self {
        Self {
            backends: OnceCell::new(),
            issued_sessions: Mutex::new(HashSet::new()),
        }
    }

    fn remember(&self, session_id: SessionId) {
        self.issued_sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(session_id);
    }

    fn issued(&self, session_id: SessionId) -> bool {
        self.issued_sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(&session_id)
    }
}

impl fmt::Debug for AgentState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let issued_sessions = self
            .issued_sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len();
        f.debug_struct("AgentState")
            .field("backends_initialized", &self.backends.initialized())
            .field("issued_sessions", &issued_sessions)
            .finish()
    }
}

impl fmt::Debug for Agent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let capability_ids = self
            .capabilities
            .iter()
            .map(AgentCapabilityConfig::capability_id)
            .collect::<Vec<_>>();
        f.debug_struct("Agent")
            .field("name", &self.name)
            .field("instructions", &self.instructions)
            .field("model", &self.model)
            .field("provider", &self.provider)
            .field("capabilities", &capability_ids)
            .field(
                "capability_implementations",
                &self.capability_implementations,
            )
            .field("initial_files", &self.initial_files)
            .field("max_iterations", &self.max_iterations)
            .field("parallel_tool_calls", &self.parallel_tool_calls)
            .field("workspace_root", &self.workspace_root)
            .field("mcp_servers", &self.mcp_servers)
            .field("plugin_warnings", &self.plugin_warnings)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

#[allow(dead_code)] // Only constructed by the optional local persistence edge.
pub(crate) enum BackendInitError {
    Event(EventLogError),
    Host(everruns_core::AgentLoopError),
}

impl BackendInitError {
    fn into_agent_loop(self) -> everruns_core::AgentLoopError {
        match self {
            Self::Event(error) => everruns_core::AgentLoopError::store(error.to_string()),
            Self::Host(error) => error,
        }
    }

    pub(crate) fn history_error(&self) -> crate::HistoryError {
        match self {
            Self::Event(EventLogError::Corruption { .. }) => crate::HistoryError::Corrupt,
            Self::Event(_) | Self::Host(_) => crate::HistoryError::Unavailable,
        }
    }

    fn resume_error(&self) -> crate::ResumeError {
        match self {
            Self::Event(EventLogError::Corruption { .. }) => crate::ResumeError::Corrupt,
            Self::Event(_) | Self::Host(_) => crate::ResumeError::Unavailable,
        }
    }
}

impl Agent {
    /// Start describing an agent.
    ///
    /// # Example
    ///
    /// ```
    /// use everruns::{Agent, Model};
    ///
    /// let agent = Agent::builder()
    ///     .instructions("You are concise.")
    ///     .model(Model::simulated("Sure."))
    ///     .name("assistant")
    ///     .capability("test_math")
    ///     .parallel_tool_calls(true)
    ///     .build()?;
    /// # Ok::<(), everruns::BuildError>(())
    /// ```
    pub fn builder() -> AgentBuilder {
        AgentBuilder::default()
    }

    /// Open a new, independent multi-turn [`Session`](crate::Session) with this
    /// agent.
    ///
    /// The session is lazy: the in-process runtime is assembled on the first
    /// [`Session::send`](crate::Session::send) or
    /// [`Session::inspect`](crate::Session::inspect). Each session gets a fresh
    /// id and isolated history. The Agent and its clones share the private
    /// session catalog and event log so a dropped session can be resumed while
    /// the Agent's configured persistence lifecycle remains available.
    pub fn session(&self) -> crate::Session {
        let session_id = SessionId::new();
        self.state.remember(session_id);
        crate::Session::new(self.clone(), session_id)
    }

    /// Resume a session through this Agent's configured session catalog.
    ///
    /// The default catalog is Agent-lifetime memory and is shared by Agent
    /// clones. With [`LocalConfig`](crate::LocalConfig), the catalog and
    /// canonical event log survive a new Agent and process. The resumed session
    /// uses this Agent's current model, instructions, tools, hooks, and
    /// workspace configuration.
    ///
    /// # Errors
    ///
    /// Returns [`ResumeError`](crate::ResumeError) when the id is unknown, the
    /// configured catalog is unavailable, or persisted local state is corrupt.
    pub async fn resume(
        &self,
        session_id: SessionId,
    ) -> Result<crate::Session, crate::ResumeError> {
        if !self.state.issued(session_id) {
            let backends = self
                .shared_backends()
                .await
                .map_err(|error| error.resume_error())?;
            let exists = backends
                .session_store
                .get_session(session_id)
                .await
                .map_err(|_| crate::ResumeError::Unavailable)?
                .is_some();
            if !exists {
                return Err(crate::ResumeError::SessionNotFound { session_id });
            }
            self.state.remember(session_id);
        }
        Ok(crate::Session::new(self.clone(), session_id))
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn lifecycle_hooks(&self) -> crate::hooks::LifecycleHooks {
        self.lifecycle_hooks.clone()
    }

    pub(crate) async fn shared_backends(&self) -> Result<&HostBackends, BackendInitError> {
        self.state
            .backends
            .get_or_try_init(|| async {
                let backends = HostBackends::in_memory();
                #[cfg(feature = "local")]
                let backends = if let Some(config) = &self.local {
                    let profile = config.profile();
                    profile.ensure_dirs().map_err(|error| {
                        BackendInitError::Host(everruns_core::AgentLoopError::config(
                            error.to_string(),
                        ))
                    })?;
                    let event_log = Arc::new(
                        everruns_host::JsonlEventLog::open(config.data_dir().join("events.jsonl"))
                            .await
                            .map_err(BackendInitError::Event)?,
                    );
                    let local = everruns_local::LocalBackends::new(
                        profile,
                        backends.with_event_log(event_log),
                    )
                    .map_err(BackendInitError::Host)?;
                    let session_store = Arc::new(
                        everruns_local::LocalSessionStore::new(local.db.clone())
                            .map_err(BackendInitError::Host)?,
                    );
                    local.runtime_backends.with_session_store(session_store)
                } else {
                    backends
                };
                Ok(backends)
            })
            .await
    }

    pub(crate) async fn ensure_session_cataloged(
        &self,
        session_id: SessionId,
    ) -> Result<(), crate::HistoryError> {
        let backends = self
            .shared_backends()
            .await
            .map_err(|error| error.history_error())?;
        if backends
            .session_store
            .get_session(session_id)
            .await
            .map_err(|_| crate::HistoryError::Unavailable)?
            .is_some()
        {
            return Ok(());
        }
        if !self.state.issued(session_id) {
            return Err(crate::HistoryError::SessionNotFound { session_id });
        }
        // Catalog identity is intentionally independent from event presence.
        // A zero-message session becomes resumable without inventing a history
        // event or treating orphan events as a session record.
        let session = SessionBuilder::new(everruns_core::HarnessId::new())
            .id(session_id)
            .build();
        backends
            .session_store
            .add_session(session)
            .await
            .map_err(|_| crate::HistoryError::Unavailable)
    }

    /// Materialize a fresh in-process runtime for this agent, seeded with the
    /// given session id, routing post-commit canonical events through the
    /// supplied facade sink so a [`Session`](crate::Session) can stream them.
    ///
    /// Each call assembles a new `Harness`/`Agent`/`Session` composition, so
    /// distinct session ids yield independent sessions. This is the private seam
    /// [`Session`](crate::Session) builds on. The sink observes finalized event
    /// envelopes; the host log remains the sole write path and sequence owner.
    pub(crate) async fn build_runtime_with_event_sink(
        &self,
        session_id: SessionId,
        event_sink: Arc<dyn EventSink>,
        hook_state: Arc<crate::hooks::HookRunState>,
    ) -> Result<InProcessRuntime, everruns_core::AgentLoopError> {
        self.build_runtime_with_backends(session_id, Some(event_sink), Some(hook_state))
            .await
    }

    pub(crate) fn plugin_warnings(&self) -> Vec<String> {
        self.plugin_warnings.clone()
    }

    async fn build_runtime_with_backends(
        &self,
        session_id: SessionId,
        event_sink: Option<Arc<dyn EventSink>>,
        hook_state: Option<Arc<crate::hooks::HookRunState>>,
    ) -> Result<InProcessRuntime, everruns_core::AgentLoopError> {
        let hook_capability = hook_state.and_then(|state| state.capability());
        let mut capabilities = self.capabilities.clone();
        if hook_capability.is_some() {
            capabilities.retain(|capability| {
                capability.capability_id() != crate::hooks::LIFECYCLE_HOOK_CAPABILITY_ID
            });
            capabilities.push(AgentCapabilityConfig::new(
                crate::hooks::LIFECYCLE_HOOK_CAPABILITY_ID,
            ));
        }
        let mut harness =
            HarnessBuilder::new(&self.name, &self.instructions).capabilities(capabilities.clone());
        if let Some(parallel) = self.parallel_tool_calls {
            harness = harness.parallel_tool_calls(parallel);
        }
        for file in &self.initial_files {
            harness = harness.initial_file(file.clone());
        }
        let harness_id = harness.harness_id();
        let harness = harness.build();

        // EVE-877: the builder produces a portable AgentDefinition; the
        // harness link lives on the session, not the definition.
        let mut agent = RuntimeAgentBuilder::new(&self.name, &self.instructions)
            .capabilities(capabilities.clone());
        if let Some(parallel) = self.parallel_tool_calls {
            agent = agent.parallel_tool_calls(parallel);
        }
        if let Some(max_iterations) = self.max_iterations {
            agent = agent.max_iterations(max_iterations);
        }
        let agent_id = agent.agent_id();
        let agent = agent.build();

        let mut session = SessionBuilder::new(harness_id)
            .id(session_id)
            .agent(agent_id)
            .capabilities(capabilities)
            .mcp_servers(self.mcp_servers.clone());
        if let Some(parallel) = self.parallel_tool_calls {
            session = session.parallel_tool_calls(parallel);
        }
        for file in &self.initial_files {
            session = session.initial_file(file.clone());
        }
        let session = session.build();

        let mut builder = InProcessRuntimeBuilder::new()
            .harness(harness)
            .agent(agent)
            .session(session)
            .model_spec(self.model.clone())
            .workspace_policy(self.workspace_policy.clone());

        let mut backends = self
            .shared_backends()
            .await
            .map_err(BackendInitError::into_agent_loop)?
            .clone();
        if let Some(event_sink) = event_sink {
            backends = backends.with_event_sink(event_sink);
        }
        builder = builder.backends(backends);

        let workspace_root = self.workspace_root.as_ref().or({
            #[cfg(feature = "local")]
            {
                self.local.as_ref().map(|config| &config.workspace_root)
            }
            #[cfg(not(feature = "local"))]
            {
                None
            }
        });
        if let Some(root) = workspace_root {
            // THREAT[TM-BASH-001] / THREAT[TM-FS-013]: reuse the canonical
            // real-disk factory so every operation retains containment and
            // symlink rejection; the facade does not implement a second path
            // mapper.
            let registry = {
                #[cfg(feature = "local")]
                if self.local.is_some() {
                    // The local profile serves the hosted catalog from SQLite
                    // (EVE-885), so validation and execution see one set.
                    everruns_local::local_capability_registry()
                } else {
                    framework_capability_registry(false)
                }
                #[cfg(not(feature = "local"))]
                {
                    framework_capability_registry(false)
                }
            };
            let platform = everruns_host::HostComposition::builder()
                .capability_registry(registry)
                .driver_registry(everruns_core::DriverRegistry::new())
                .egress_service(everruns_host::runtime_egress_service())
                .session_file_system_factory(Arc::new(
                    everruns_host::RealDiskSessionFileSystemFactory::new(root),
                ))
                .build();
            builder = builder.host_composition(platform);
        }
        // References and code-defined implementations were normalized and
        // collision-checked together at Agent build time. Register each
        // implementation once on this session's private runtime; the matching
        // activation ref is already attached at every configuration layer.
        for implementation in &self.capability_implementations {
            builder = implementation.register(builder);
        }
        if let Some(capability) = hook_capability {
            builder = builder.capability(capability);
        }
        builder = builder.provider(self.provider.clone());
        builder.build().await
    }
}

/// Fluent builder behind [`Agent::builder`].
///
/// Instructions and a model are required. A live string model also requires
/// exactly one provider; deterministic [`Model::simulated`] values supply their
/// own offline provider. Invalid configurations fail [`build`](Self::build)
/// with a typed [`BuildError`].
///
/// Lifecycle methods such as [`on_turn_start`](Self::on_turn_start) accept
/// async `Fn` handlers. The Framework awaits each per-point chain in builder
/// registration order. Handler contexts are owned snapshots and cannot mutate
/// execution. The same handlers are shared by every session, so different
/// sessions—and parallel tool calls—may invoke them concurrently. Returned
/// errors follow each lifecycle method's contract; handler panics are not
/// caught.
#[derive(Clone, Debug, Default)]
pub struct AgentBuilder {
    name: Option<String>,
    instructions: Option<String>,
    model: Option<Model>,
    providers: Vec<Provider>,
    capabilities: Vec<crate::CapabilitySpec>,
    tools: Vec<Tool>,
    initial_files: Vec<InitialFile>,
    max_iterations: Option<usize>,
    parallel_tool_calls: Option<bool>,
    workspace_root: Option<PathBuf>,
    workspace_policy: everruns_core::WorkspacePolicy,
    mcp_servers: Vec<crate::McpServer>,
    plugin_warnings: Vec<String>,
    #[cfg(feature = "local")]
    local: Option<crate::LocalConfig>,
    lifecycle_hooks: crate::hooks::LifecycleHooks,
}

impl AgentBuilder {
    /// Set the agent's system instructions. Required.
    pub fn instructions(mut self, instructions: impl Into<String>) -> Self {
        self.instructions = Some(instructions.into());
        self
    }

    /// Select the model the agent talks to. Required.
    ///
    /// Live models use their provider-visible string id. Configure how to reach
    /// that model separately with [`provider`](Self::provider). A
    /// [`Model::simulated`] value needs no provider.
    pub fn model(mut self, model: impl Into<Model>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Configure the service provider used by the selected live model.
    ///
    /// Accepts a raw [`Provider`] or a provider convenience such as
    /// [`OpenAI`](crate::providers::openai::OpenAI) with the `openai` feature.
    /// An agent currently accepts exactly one provider.
    pub fn provider(mut self, provider: impl Into<Provider>) -> Self {
        self.providers.push(provider.into());
        self
    }

    /// Set a human-readable name. Optional; defaults to `"agent"`.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Add a tool the agent can call.
    ///
    /// Accepts anything that is [`IntoTool`](crate::IntoTool), including a
    /// [`FunctionTool`](crate::FunctionTool) backed by an async function or
    /// closure. Capability references use [`capability`](Self::capability)
    /// instead. Tool names and JSON schemas are validated, and duplicate names
    /// rejected, at [`build`](Self::build).
    ///
    /// # Example
    ///
    /// ```
    /// use everruns::{Agent, FunctionTool, Model};
    /// use serde_json::json;
    ///
    /// let agent = Agent::builder()
    ///     .instructions("You are concise.")
    ///     .model(Model::simulated("done"))
    ///     .capability("test_math")
    ///     .tool(FunctionTool::new(
    ///         "roll",
    ///         "Roll a die.",
    ///         json!({ "type": "object", "properties": {} }),
    ///         |_args: serde_json::Value| async move { Ok::<_, String>(json!({ "value": 4 })) },
    ///     ))
    ///     .build()?;
    /// # let _ = agent;
    /// # Ok::<(), everruns::BuildError>(())
    /// ```
    pub fn tool(mut self, tool: impl IntoTool) -> Self {
        self.tools.push(tool.into_tool());
        self
    }

    /// Run an async handler before the first turn attempted by each session.
    ///
    /// Handlers run sequentially in registration order and are awaited. An
    /// error stops the turn before execution; the next call retries the full
    /// agent-start chain. Infallible handlers may return `()`. Run cancellation
    /// drops the active handler future and skips the rest. Inspecting a session
    /// neither starts the agent nor invokes this handler.
    pub fn on_agent_start<F, Fut, O>(mut self, handler: F) -> Self
    where
        F: Fn(crate::AgentStartContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = O> + Send + 'static,
        O: crate::IntoHookResult + 'static,
    {
        self.lifecycle_hooks.on_agent_start(handler);
        self
    }

    /// Run an async handler before every turn enters the runtime.
    ///
    /// Handlers see the exact input but cannot mutate it. They run sequentially
    /// in registration order; the first error prevents the turn. Run
    /// cancellation drops the active handler future and skips the rest.
    pub fn on_turn_start<F, Fut, O>(mut self, handler: F) -> Self
    where
        F: Fn(crate::TurnStartContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = O> + Send + 'static,
        O: crate::IntoHookResult + 'static,
    {
        self.lifecycle_hooks.on_turn_start(handler);
        self
    }

    /// Run an async handler before every model-requested tool call.
    ///
    /// Multiple handlers run sequentially for one call. Tool calls in a
    /// parallel batch may run their independent hook chains concurrently. A
    /// handler error blocks only that tool call and becomes a model-visible
    /// generic tool error; application error details stay on
    /// [`Turn::hook_failures`](crate::Turn::hook_failures). Later start
    /// handlers for that call are skipped. Run cancellation drops an active
    /// handler future.
    pub fn on_tool_start<F, Fut, O>(mut self, handler: F) -> Self
    where
        F: Fn(crate::ToolStartContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = O> + Send + 'static,
        O: crate::IntoHookResult + 'static,
    {
        self.lifecycle_hooks.on_tool_start(handler);
        self
    }

    /// Run an async handler after every tool call reaches a terminal result.
    ///
    /// This includes calls blocked by another pre-tool hook; their context
    /// carries the resulting error. The call has already settled, so handler
    /// errors cannot undo it. All handlers still run; failures are returned on
    /// [`Turn::hook_failures`](crate::Turn::hook_failures). Run cancellation
    /// may drop the active chain with the rest of the in-flight turn.
    pub fn on_tool_end<F, Fut, O>(mut self, handler: F) -> Self
    where
        F: Fn(crate::ToolEndContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = O> + Send + 'static,
        O: crate::IntoHookResult + 'static,
    {
        self.lifecycle_hooks.on_tool_end(handler);
        self
    }

    /// Run an async handler after a non-cancelled turn reaches an outcome.
    ///
    /// Completion handlers run sequentially after the runtime settles. Their
    /// errors are isolated because the turn is already committed; all handlers
    /// run, and failures are appended to
    /// [`Turn::hook_failures`](crate::Turn::hook_failures). Once completion
    /// begins, cancelling that turn's run token does not interrupt the chain.
    pub fn on_completion<F, Fut, O>(mut self, handler: F) -> Self
    where
        F: Fn(crate::CompletionContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = O> + Send + 'static,
        O: crate::IntoHookResult + 'static,
    {
        self.lifecycle_hooks.on_completion(handler);
        self
    }

    /// Add one capability through the Framework's open conversion contract.
    ///
    /// Accepts typed built-ins such as [`CompactionConfig`](crate::CompactionConfig)
    /// and [`ToolSearch`](crate::ToolSearch), a code-defined
    /// `capability::Definition` (with the `capabilities` feature), a dynamic
    /// [`CapabilityRef`](crate::CapabilityRef), or any third-party value that
    /// implements [`IntoCapability`](crate::IntoCapability). Plain strings are
    /// concise references with default configuration.
    ///
    /// Conversion happens immediately and is infallible. Identifier and JSON
    /// validation, built-in schema validation, duplicate detection, and
    /// implementation/reference collision checks happen together in
    /// [`build`](Self::build). A stable ID may be activated only once; duplicate
    /// inputs are errors rather than last-write-wins overrides.
    pub fn capability(mut self, capability: impl crate::IntoCapability) -> Self {
        self.capabilities.push(capability.into_capability());
        self
    }

    /// Limit the number of model/tool iterations allowed in one turn.
    pub fn max_iterations(mut self, max_iterations: usize) -> Self {
        self.max_iterations = Some(max_iterations);
        self
    }

    /// Prefer (or forbid) parallel tool calls within a single reasoning step.
    pub fn parallel_tool_calls(mut self, parallel: bool) -> Self {
        self.parallel_tool_calls = Some(parallel);
        self
    }

    /// Seed a file into the agent's initial workspace.
    pub fn initial_file(mut self, file: InitialFile) -> Self {
        self.initial_files.push(file);
        self
    }

    /// Seed an editable UTF-8 text file into the initial workspace.
    pub fn file(mut self, path: impl Into<String>, content: impl Into<String>) -> Self {
        self.initial_files.push(InitialFile {
            path: path.into(),
            content: content.into(),
            encoding: "text".to_string(),
            is_readonly: false,
        });
        self
    }

    /// Seed a read-only UTF-8 text file into the initial workspace.
    pub fn readonly_file(mut self, path: impl Into<String>, content: impl Into<String>) -> Self {
        self.initial_files.push(InitialFile {
            path: path.into(),
            content: content.into(),
            encoding: "text".to_string(),
            is_readonly: true,
        });
        self
    }

    /// Use a real host directory as this agent's `/workspace`.
    ///
    /// The runtime rejects traversal and symlink escapes at every filesystem
    /// operation. The directory must exist before the session is first used and
    /// must be selected by trusted application configuration, not model input.
    pub fn workspace(mut self, root: impl Into<PathBuf>) -> Self {
        self.workspace_root = Some(root.into());
        if !self
            .capabilities
            .iter()
            .any(|capability| capability.capability_ref().id() == "session_file_system")
        {
            self.capabilities
                .push(crate::CapabilityRef::new("session_file_system").into());
        }
        self
    }

    /// Set the read and write policy for this agent's workspace.
    ///
    /// The default is [`WorkspacePolicy::read_only`](crate::WorkspacePolicy::read_only):
    /// ordinary files are readable, writes and hidden or sensitive paths are
    /// denied, and framework-managed `.agents` content remains readable.
    /// Select [`WorkspacePolicy::read_write`](crate::WorkspacePolicy::read_write)
    /// or build narrower scopes for an explicit write opt-in.
    pub fn workspace_policy(mut self, policy: crate::WorkspacePolicy) -> Self {
        self.workspace_policy = policy;
        self
    }

    /// Add a scoped MCP server to this agent.
    pub fn mcp_server(mut self, server: crate::McpServer) -> Self {
        self.mcp_servers.push(server);
        self
    }

    /// Load and enable a local plugin directory.
    ///
    /// Reading and compilation happen immediately so invalid or unsafe plugin
    /// input fails before the agent is built. Non-fatal warnings are available
    /// from [`Session::inspect`](crate::Session::inspect). Plugins contribute
    /// instructions and capabilities, so select the directory from trusted
    /// application configuration rather than model or request input.
    pub fn plugin(mut self, path: impl AsRef<Path>) -> Result<Self, crate::PluginError> {
        let loaded = crate::plugin::load(path.as_ref())?;
        self.capabilities.push(loaded.capability);
        self.plugin_warnings.extend(loaded.warnings);
        Ok(self)
    }

    /// Enable restart-survivable local state and a real workspace.
    ///
    /// Task, schedule, and session identity state is stored in SQLite.
    /// Conversation history is reconstructed from the canonical local event
    /// log. Requires the `local` feature.
    #[cfg(feature = "local")]
    pub fn local(mut self, config: crate::LocalConfig) -> Self {
        self.local = Some(config);
        if !self
            .capabilities
            .iter()
            .any(|capability| capability.capability_ref().id() == "session_file_system")
        {
            self.capabilities
                .push(crate::CapabilityRef::new("session_file_system").into());
        }
        self
    }

    /// Validate the description and produce an [`Agent`].
    ///
    /// # Errors
    ///
    /// - [`BuildError::BlankInstructions`] if instructions are missing or only
    ///   whitespace.
    /// - [`BuildError::MissingModel`] if no model was set.
    /// - [`BuildError::MissingProvider`] if a live model has no provider.
    /// - [`BuildError::MultipleProviders`] if more than one provider was set.
    /// - [`BuildError::InvalidToolName`] if a function tool's name is not a
    ///   valid model-facing identifier.
    /// - [`BuildError::InvalidToolSchema`] if a function tool's JSON schema is
    ///   not a valid arguments schema.
    /// - [`BuildError::DuplicateTool`] if two `.tool(..)` calls share a name.
    /// - [`BuildError::InvalidMcpServer`] if an MCP server is unnamed,
    ///   incomplete, or duplicates another server name.
    /// - [`BuildError::InvalidCapability`] if a capability ID, JSON config, or
    ///   code-defined descriptor is invalid.
    /// - [`BuildError::DuplicateCapability`] if two inputs resolve to the same
    ///   capability implementation, including aliases and reference/implementation
    ///   collisions.
    pub fn build(self) -> Result<Agent, BuildError> {
        let instructions = self.instructions.unwrap_or_default();
        if instructions.trim().is_empty() {
            return Err(BuildError::BlankInstructions);
        }
        let model = self.model.ok_or(BuildError::MissingModel)?;
        let mut providers = self.providers;
        if let Some(provider) = model.bundled_provider {
            providers.push(provider);
        }
        if providers.is_empty() {
            return Err(BuildError::MissingProvider);
        }
        if providers.len() > 1 {
            let mut registered = providers
                .iter()
                .map(|provider| provider.id().to_string())
                .collect::<Vec<_>>();
            registered.sort();
            return Err(BuildError::MultipleProviders { registered });
        }
        let provider = providers.pop().expect("provider count checked above");
        let model = ModelSpec::on(provider.id().clone(), model.id);
        let name = self.name.unwrap_or_else(|| "agent".to_string());

        // Function tools retain their own public entrypoint. Privately they are
        // registered as single-tool capabilities, so their names participate in
        // the same implementation-ID collision checks as every other input.
        let mut function_tools = Vec::new();
        let mut seen_tool_names: HashSet<String> = HashSet::new();
        for tool in self.tools {
            let tool_name = tool.name().to_string();
            if !seen_tool_names.insert(tool_name.clone()) {
                return Err(BuildError::DuplicateTool { name: tool_name });
            }
            let function_tool = tool.into_function();
            validate_tool_name(function_tool.name()).map_err(|reason| {
                BuildError::InvalidToolName {
                    name: tool_name.clone(),
                    reason,
                }
            })?;
            validate_tool_schema(function_tool.schema()).map_err(|reason| {
                BuildError::InvalidToolSchema {
                    name: tool_name,
                    reason,
                }
            })?;
            function_tools.push(function_tool);
        }

        let mcp_servers = crate::mcp::into_scoped(self.mcp_servers)
            .map_err(|reason| BuildError::InvalidMcpServer { reason })?;

        // Use the same built-in set that this Agent will place on each private
        // runtime. This makes build-time schema checks and implementation
        // collision detection match execution instead of relying on a closed
        // facade-owned list.
        let capability_registry = {
            #[cfg(feature = "local")]
            if self.local.is_some() {
                everruns_local::local_capability_registry()
            } else {
                framework_capability_registry(false)
            }
            #[cfg(not(feature = "local"))]
            {
                framework_capability_registry(false)
            }
        };

        let mut capabilities = Vec::new();
        let mut capability_implementations = Vec::new();
        // Neutral duplicate/collision rejection shared with product registries.
        let mut activated_capabilities = everruns_capability::ActivationSet::new();

        for input in self.capabilities {
            let parts = input.into_parts();
            let input_id = parts.reference.id().to_string();
            everruns_capability::validate_capability_id(&input_id).map_err(|error| {
                BuildError::InvalidCapability {
                    id: input_id.clone(),
                    reason: error.reason(),
                }
            })?;
            everruns_capability::validate_capability_config(
                &input_id,
                parts.reference.config_value(),
            )
            .map_err(|error| BuildError::InvalidCapability {
                id: input_id.clone(),
                reason: error.reason(),
            })?;

            let canonical_id = capability_registry
                .canonical_id(&input_id)
                .unwrap_or(&input_id)
                .to_string();
            if activated_capabilities.contains(&canonical_id) {
                return Err(BuildError::DuplicateCapability { id: canonical_id });
            }

            #[cfg(feature = "capabilities")]
            if let Some(definition) = parts.definition {
                definition
                    .validate()
                    .map_err(|error| BuildError::InvalidCapability {
                        id: input_id.clone(),
                        reason: error.reason(),
                    })?;
                if capability_registry.get(definition.id()).is_some() {
                    return Err(BuildError::DuplicateCapability {
                        id: definition.id().to_string(),
                    });
                }
                for tool in definition.tools() {
                    let spec = tool.spec();
                    validate_tool_name(spec.name()).map_err(|reason| {
                        BuildError::InvalidToolName {
                            name: spec.name().to_string(),
                            reason,
                        }
                    })?;
                    validate_tool_schema(spec.input_schema()).map_err(|reason| {
                        BuildError::InvalidToolSchema {
                            name: spec.name().to_string(),
                            reason,
                        }
                    })?;
                    if !seen_tool_names.insert(spec.name().to_string()) {
                        return Err(BuildError::DuplicateTool {
                            name: spec.name().to_string(),
                        });
                    }
                }
                capability_implementations.push(CapabilityImplementation::Definition(definition));
            }

            validate_registered_capability_config(
                &capability_registry,
                &canonical_id,
                parts.reference.config_value(),
            )?;
            activated_capabilities
                .activate(canonical_id.clone())
                .map_err(|error| BuildError::DuplicateCapability {
                    id: error.id().to_string(),
                })?;
            capabilities.push(AgentCapabilityConfig::with_config(
                canonical_id,
                parts.reference.config_value().clone(),
            ));
        }

        for function_tool in function_tools {
            let id = function_tool.name().to_string();
            everruns_capability::validate_capability_id(&id).map_err(|error| {
                BuildError::InvalidCapability {
                    id: id.clone(),
                    reason: error.reason(),
                }
            })?;
            if capability_registry.get(&id).is_some()
                || activated_capabilities.activate(id.clone()).is_err()
            {
                return Err(BuildError::DuplicateCapability { id });
            }
            capabilities.push(AgentCapabilityConfig::new(id.as_str()));
            capability_implementations.push(CapabilityImplementation::Function(function_tool));
        }

        Ok(Agent {
            name,
            instructions,
            model,
            provider,
            capabilities,
            capability_implementations,
            initial_files: self.initial_files,
            max_iterations: self.max_iterations,
            parallel_tool_calls: self.parallel_tool_calls,
            workspace_root: self.workspace_root,
            workspace_policy: self.workspace_policy,
            mcp_servers,
            plugin_warnings: self.plugin_warnings,
            #[cfg(feature = "local")]
            local: self.local,
            lifecycle_hooks: self.lifecycle_hooks,
            state: Arc::new(AgentState::new()),
        })
    }
}

fn framework_capability_registry(hosted_base: bool) -> everruns_core::CapabilityRegistry {
    let registry = everruns_core::CapabilityRegistry::new();
    #[cfg(feature = "builtins")]
    let registry = {
        let mut registry = registry;
        if hosted_base {
            everruns_builtins::register_portable_capabilities(&mut registry)
                .expect("portable built-in catalog must have unique capability IDs");
        } else {
            everruns_builtins::register_runtime_capabilities(&mut registry)
                .expect("portable runtime catalog must have unique capability IDs");
        }
        registry
    };
    everruns_host::compose_runtime_capability_registry(registry)
}

fn validate_registered_capability_config(
    registry: &everruns_core::CapabilityRegistry,
    id: &str,
    config: &serde_json::Value,
) -> Result<(), BuildError> {
    if let Some(capability) = registry.get(id) {
        capability
            .validate_config(config)
            .map_err(|reason| BuildError::InvalidCapability {
                id: id.to_string(),
                reason,
            })?;
    }

    #[cfg(feature = "builtins")]
    if id == everruns_builtins::AUTO_TOOL_SEARCH_CAPABILITY_ID {
        validate_tool_search_config(config).map_err(|reason| BuildError::InvalidCapability {
            id: id.to_string(),
            reason,
        })?;
    }

    if everruns_core::is_declarative_capability(id) || everruns_core::is_plugin_capability(id) {
        let definition = serde_json::from_value::<everruns_core::DeclarativeCapabilityDefinition>(
            config.clone(),
        )
        .map_err(|error| BuildError::InvalidCapability {
            id: id.to_string(),
            reason: format!("invalid declarative capability config: {error}"),
        })?;
        everruns_core::validate_declarative_capability_definition(&definition).map_err(
            |reason| BuildError::InvalidCapability {
                id: id.to_string(),
                reason,
            },
        )?;
    }
    Ok(())
}

#[cfg(feature = "builtins")]
fn validate_tool_search_config(config: &serde_json::Value) -> Result<(), String> {
    let object = config
        .as_object()
        .expect("capability config object validated before built-in schema");
    for key in object.keys() {
        if key != "threshold" && key != "never_defer" {
            return Err(format!("unknown tool-search config field {key:?}"));
        }
    }
    if let Some(threshold) = object.get("threshold") {
        let value = threshold
            .as_u64()
            .ok_or_else(|| "threshold must be a non-negative integer".to_string())?;
        usize::try_from(value)
            .map_err(|_| "threshold is too large for this platform".to_string())?;
    }
    if let Some(never_defer) = object.get("never_defer") {
        let names = never_defer
            .as_array()
            .ok_or_else(|| "never_defer must be an array of tool names".to_string())?;
        for name in names {
            let name = name
                .as_str()
                .ok_or_else(|| "never_defer must contain only tool names".to_string())?;
            validate_tool_name(name)
                .map_err(|reason| format!("invalid never_defer tool {name:?}: {reason}"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use everruns_core::ToolCall;
    use serde_json::{Value, json};

    use super::*;
    use crate::FunctionTool;

    fn obj_schema() -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    #[test]
    fn build_rejects_invalid_tool_name() {
        let err = Agent::builder()
            .instructions("You are concise.")
            .model(Model::simulated("ok"))
            .tool(FunctionTool::new(
                "bad name",
                "desc",
                obj_schema(),
                |_: Value| async move { Ok::<_, String>(json!({})) },
            ))
            .build()
            .unwrap_err();
        assert!(
            matches!(err, BuildError::InvalidToolName { ref name, .. } if name == "bad name"),
            "got {err:?}"
        );
    }

    #[test]
    fn build_rejects_invalid_tool_schema() {
        let err = Agent::builder()
            .instructions("You are concise.")
            .model(Model::simulated("ok"))
            .tool(FunctionTool::new(
                "arr",
                "desc",
                json!({ "type": "array" }),
                |_: Value| async move { Ok::<_, String>(json!({})) },
            ))
            .build()
            .unwrap_err();
        assert!(
            matches!(err, BuildError::InvalidToolSchema { ref name, .. } if name == "arr"),
            "got {err:?}"
        );
    }

    #[test]
    fn build_rejects_duplicate_tool_names() {
        let make = || {
            FunctionTool::new("dup", "desc", obj_schema(), |_: Value| async move {
                Ok::<_, String>(json!({}))
            })
        };
        let err = Agent::builder()
            .instructions("You are concise.")
            .model(Model::simulated("ok"))
            .tool(make())
            .tool(make())
            .build()
            .unwrap_err();
        assert_eq!(
            err,
            BuildError::DuplicateTool {
                name: "dup".to_string()
            }
        );
    }

    #[tokio::test]
    async fn function_tool_executes_end_to_end() {
        // Capture what the handler received to prove args flowed in.
        let received: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));
        let sink = received.clone();
        let tool = FunctionTool::new(
            "greet",
            "Greet a person by name.",
            json!({
                "type": "object",
                "properties": { "name": { "type": "string" } },
                "required": ["name"],
            }),
            move |args: Value| {
                let sink = sink.clone();
                async move {
                    *sink.lock().unwrap() = Some(args.clone());
                    let name = args["name"].as_str().unwrap_or("world");
                    Ok::<_, String>(json!({ "greeting": format!("Hello, {name}!") }))
                }
            },
        );

        let agent = Agent::builder()
            .instructions("Call greet when asked to greet someone.")
            .model(Model::simulated_scripted(
                "All done.",
                vec![
                    vec![ToolCall {
                        id: "call_greet_1".into(),
                        name: "greet".into(),
                        arguments: json!({ "name": "Ada" }),
                    }],
                    vec![],
                ],
            ))
            .tool(tool)
            .build()
            .expect("valid agent");

        let session = agent.session();
        let turn = session.run("Please greet Ada.").await.expect("turn runs");

        assert!(turn.success, "turn should succeed: {:?}", turn.error);
        assert_eq!(turn.tool_calls, 1, "the function tool must have executed");
        assert_eq!(turn.response, "All done.");
        assert_eq!(
            received.lock().unwrap().as_ref().expect("handler ran")["name"],
            json!("Ada"),
            "handler must receive the model's call arguments",
        );
    }

    #[tokio::test]
    async fn function_tool_handler_error_is_model_visible_not_a_panic() {
        let tool = FunctionTool::new(
            "always_fails",
            "Always returns an error.",
            obj_schema(),
            |_: Value| async move { Err::<Value, String>("boom".to_string()) },
        );

        let agent = Agent::builder()
            .instructions("Call the tool.")
            .model(Model::simulated_scripted(
                "Handled.",
                vec![
                    vec![ToolCall {
                        id: "call_fail_1".into(),
                        name: "always_fails".into(),
                        arguments: json!({}),
                    }],
                    vec![],
                ],
            ))
            .tool(tool)
            .build()
            .expect("valid agent");

        let session = agent.session();
        // The handler error becomes a tool result the model consumes; the turn
        // still completes rather than panicking.
        let turn = session.run("go").await.expect("turn runs");
        assert!(turn.success, "turn should recover from a tool error");
        assert_eq!(turn.tool_calls, 1);
    }

    #[test]
    fn build_rejects_blank_instructions() {
        let err = Agent::builder()
            .instructions("   ")
            .model(Model::simulated("hi"))
            .build()
            .unwrap_err();
        assert_eq!(err, BuildError::BlankInstructions);
    }

    #[test]
    fn build_rejects_missing_model() {
        let err = Agent::builder()
            .instructions("You are concise.")
            .build()
            .unwrap_err();
        assert_eq!(err, BuildError::MissingModel);
    }

    #[test]
    fn build_succeeds_with_simulator() {
        let agent = Agent::builder()
            .instructions("You are concise.")
            .model(Model::simulated("Sure."))
            .name("assistant")
            .build()
            .expect("valid agent");
        assert_eq!(agent.name, "assistant");
    }

    #[test]
    fn build_resolves_plain_model_id_against_provider() {
        let agent = Agent::builder()
            .instructions("You are concise.")
            .provider(Provider::new(
                "acme",
                LlmSimDriver::new(LlmSimConfig::fixed("Sure.")),
            ))
            .model("assistant-v1")
            .build()
            .expect("valid agent");

        assert_eq!(agent.model, ModelSpec::on("acme", "assistant-v1"));
        assert_eq!(agent.provider.id().as_str(), "acme");
    }

    #[test]
    fn build_rejects_live_model_without_provider() {
        let err = Agent::builder()
            .instructions("You are concise.")
            .model("assistant-v1")
            .build()
            .unwrap_err();

        assert_eq!(err, BuildError::MissingProvider);
    }

    #[test]
    fn build_rejects_multiple_providers() {
        let provider = |id| Provider::new(id, LlmSimDriver::new(LlmSimConfig::fixed("Sure.")));
        let err = Agent::builder()
            .instructions("You are concise.")
            .provider(provider("zeta"))
            .provider(provider("alpha"))
            .model("assistant-v1")
            .build()
            .unwrap_err();

        assert_eq!(
            err,
            BuildError::MultipleProviders {
                registered: vec!["alpha".to_string(), "zeta".to_string()],
            }
        );
    }

    #[test]
    fn workspace_policy_defaults_to_read_only() {
        let agent = Agent::builder()
            .instructions("You are concise.")
            .model(Model::simulated("Sure."))
            .build()
            .expect("valid agent");

        assert!(agent.workspace_policy.permits_read("notes.txt"));
        assert!(!agent.workspace_policy.permits_write("notes.txt"));
        assert!(!agent.workspace_policy.permits_read(".env"));
    }

    #[tokio::test]
    async fn custom_workspace_policy_reaches_the_high_level_runtime() {
        let workspace = tempfile::tempdir().expect("workspace tempdir");
        let policy = crate::WorkspacePolicy::builder()
            .allow_read("public")
            .build()
            .expect("valid policy");
        let agent = Agent::builder()
            .instructions("Read the workspace.")
            .model(Model::simulated("Sure."))
            .workspace(workspace.path())
            .workspace_policy(policy)
            .file("public/note.txt", "visible")
            .file("private/note.txt", "hidden")
            .build()
            .expect("valid agent");

        let session_id = SessionId::new();
        let runtime = agent
            .build_runtime_with_backends(session_id, None, None)
            .await
            .expect("runtime builds");
        let visible = runtime
            .read_file(session_id, "public/note.txt")
            .await
            .expect("allowed read")
            .expect("seeded file");
        assert_eq!(visible.content.as_deref(), Some("visible"));

        let denied = runtime.read_file(session_id, "private/note.txt").await;
        assert!(denied.is_err());
    }

    #[cfg(feature = "openai")]
    #[test]
    fn openai_provider_converts_without_leaking_key() {
        use crate::providers::openai::OpenAI;

        let provider: Provider = OpenAI::new("sk-super-secret").into();
        assert_eq!(provider.id().as_str(), "openai");
        let rendered = format!("{provider:?}");
        assert!(!rendered.contains("sk-super-secret"), "got {rendered}");
    }

    #[cfg(feature = "openai")]
    #[tokio::test]
    async fn openai_agent_builds_runtime_offline() {
        use crate::providers::openai::OpenAI;

        // Building the runtime registers the OpenAI driver and assembles the
        // in-process composition without any network call — the provider is only
        // contacted when a turn actually runs, which this test never does.
        let agent = Agent::builder()
            .instructions("You are concise.")
            .provider(OpenAI::new("sk-test"))
            .model("gpt-5-mini")
            .build()
            .expect("valid agent");

        let runtime = agent
            .build_runtime_with_backends(SessionId::new(), None, None)
            .await
            .expect("openai runtime builds offline");
        let _ = runtime;
    }

    #[tokio::test]
    async fn build_runtime_seeds_the_requested_session_id() {
        let agent = Agent::builder()
            .instructions("You are concise.")
            .model(Model::simulated("Sure."))
            .build()
            .expect("valid agent");

        let session_id = SessionId::new();
        let runtime = agent
            .build_runtime_with_backends(session_id, None, None)
            .await
            .expect("runtime builds");
        // The seeded session id is usable directly: a caller can run a turn
        // against it without going through `default_session_id`.
        let result = runtime
            .run_turn(session_id, everruns_core::InputMessage::user("hi"))
            .await
            .expect("turn runs");
        assert!(result.success);
        assert_eq!(result.response, "Sure.");
    }
}
