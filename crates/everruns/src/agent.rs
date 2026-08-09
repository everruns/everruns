//! Value-first agent description (EVE-832).
//!
//! [`Agent::builder`] lets a library user describe an agent — instructions, a
//! model, optional tools and files — without constructing stored `Harness`,
//! `Agent`, `Session`, IDs, timestamps, statuses, registries, or a
//! `PlatformDefinition`. The builder validates the value-first configuration and
//! adapts it, inside [`AgentBuilder::build`], to the existing runtime builders.
//!
//! Running turns and multi-turn sessions are intentionally out of scope here;
//! this type only describes an agent and can materialize independent in-process
//! runtimes from that description.

use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use everruns_core::llmsim_driver::{LlmSimConfig, LlmSimDriver};
use everruns_core::{AgentCapabilityConfig, InitialFile, ModelSpec, Provider, SessionId};
use everruns_runtime::{
    AgentBuilder as RuntimeAgentBuilder, EventBus, HarnessBuilder, InProcessRuntime,
    InProcessRuntimeBuilder, RuntimeBackends, RuntimeMessageStore, SessionBuilder,
};

use crate::tool::{FunctionTool, IntoTool, Tool, validate_tool_name, validate_tool_schema};

/// How an [`Agent`] talks to a model.
///
/// A `Model` carries a credential-free specification and may bundle a ready-made
/// provider assembly. [`Model::simulated`] uses the same provider path as every
/// network-backed model.
#[derive(Clone)]
pub struct Model {
    spec: ModelSpec,
    bundled_provider: Option<Provider>,
}

impl Model {
    /// A deterministic in-process model that always replies with `response`.
    ///
    /// Backed by the `llmsim` driver, so an agent using it runs entirely
    /// offline — no credentials, no network.
    pub fn simulated(response: impl Into<String>) -> Self {
        Self::with_provider(
            ModelSpec::on("llmsim", "llmsim-model"),
            Provider::new("llmsim", LlmSimDriver::new(LlmSimConfig::fixed(response))),
        )
    }

    /// Build a `Model` that targets OpenAI's Responses API.
    ///
    /// The convenience produces the same public model/provider pair callers can
    /// assemble directly with [`ModelSpec`] and [`Provider`].
    #[cfg(feature = "openai")]
    pub(crate) fn openai(config: crate::providers::openai::OpenAI) -> Self {
        let (model, api_key, base_url) = config.into_parts();
        let mut provider = everruns_openai::provider("openai", api_key);
        if let Some(base_url) = base_url {
            provider = provider.base_url(base_url);
        }
        Self::with_provider(ModelSpec::on("openai", model), provider)
    }

    /// Pair a model specification with a ready-to-use provider assembly.
    pub fn with_provider(spec: ModelSpec, provider: Provider) -> Self {
        Self {
            spec,
            bundled_provider: Some(provider),
        }
    }
}

impl From<ModelSpec> for Model {
    fn from(spec: ModelSpec) -> Self {
        Self {
            spec,
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
        Self::with_provider(
            ModelSpec::on("llmsim", "llmsim-model"),
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
        Self::with_provider(
            ModelSpec::on("llmsim", "llmsim-model"),
            Provider::new("llmsim", LlmSimDriver::new(sim)),
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
        Self::with_provider(
            ModelSpec::on("llmsim", "llmsim-model"),
            Provider::new("llmsim", LlmSimDriver::new(sim)),
        )
    }
}

impl fmt::Debug for Model {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Model")
            .field("spec", &self.spec)
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
    /// An advanced capability's public descriptor is invalid.
    InvalidCapability {
        /// The rejected capability id.
        id: String,
        /// Why the descriptor was rejected.
        reason: String,
    },
    /// Two advanced capability registrations used the same stable id.
    DuplicateCapability {
        /// The colliding capability id.
        id: String,
    },
    /// Two providers used the same normalized identity.
    DuplicateProvider { id: String },
    /// The selected model names a provider that was not registered.
    UnknownProvider {
        requested: String,
        registered: Vec<String>,
    },
    /// MCP server configuration was invalid or duplicated.
    InvalidMcpServer { reason: String },
    /// Context compaction configuration was invalid.
    InvalidCompaction { reason: String },
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
                write!(f, "invalid advanced capability {id:?}: {reason}")
            }
            BuildError::DuplicateCapability { id } => {
                write!(f, "duplicate advanced capability id {id:?}")
            }
            BuildError::DuplicateProvider { id } => write!(f, "duplicate provider {id:?}"),
            BuildError::UnknownProvider {
                requested,
                registered,
            } => write!(
                f,
                "provider {requested:?} is not registered; registered providers: [{}]",
                registered.join(", ")
            ),
            BuildError::InvalidMcpServer { reason } => {
                write!(f, "invalid MCP server configuration: {reason}")
            }
            BuildError::InvalidCompaction { reason } => {
                write!(f, "invalid compaction configuration: {reason}")
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
#[derive(Clone, Debug)]
pub struct Agent {
    name: String,
    instructions: String,
    model: Model,
    providers: Vec<Provider>,
    capabilities: Vec<AgentCapabilityConfig>,
    function_tools: Vec<FunctionTool>,
    #[cfg(feature = "capabilities")]
    advanced_capabilities: Vec<crate::capability::Definition>,
    initial_files: Vec<InitialFile>,
    parallel_tool_calls: Option<bool>,
    workspace_root: Option<PathBuf>,
    mcp_servers: everruns_core::ScopedMcpServers,
    plugin_warnings: Vec<String>,
    #[cfg(feature = "local")]
    local: Option<crate::LocalConfig>,
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
    ///     .tool("test_math")
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
    /// [`Session::run`](crate::Session::run). Each session gets a fresh id and
    /// its own history, so two sessions from the same agent never share
    /// conversation state, and cloning an `Agent` never shares history.
    pub fn session(&self) -> crate::Session {
        crate::Session::new(self.clone(), SessionId::new())
    }

    /// Open a new persisted session backed by `store` (EVE-836).
    ///
    /// Identical to [`session`](Self::session) but every turn's messages are
    /// written to the store's JSONL file, so the conversation survives the
    /// process. Resume it later with [`resume_session`](Self::resume_session)
    /// using the id returned by [`Session::id`](crate::Session::id). Requires the
    /// `jsonl` feature.
    #[cfg(feature = "jsonl")]
    pub fn session_with_store(
        &self,
        store: Arc<crate::persistence::JsonlSessionStore>,
    ) -> crate::Session {
        crate::Session::with_message_store(self.clone(), SessionId::new(), store)
    }

    /// Resume a persisted session by id, using history already loaded into
    /// `store` (EVE-836).
    ///
    /// Pass a store opened over the same file and the `session_id` string from a
    /// previous run (as returned by [`Session::id`](crate::Session::id)). The
    /// next turn includes the reloaded history. Requires the `jsonl` feature.
    #[cfg(feature = "jsonl")]
    pub fn resume_session(
        &self,
        store: Arc<crate::persistence::JsonlSessionStore>,
        session_id: &str,
    ) -> Result<crate::Session, crate::persistence::JsonlError> {
        let id: SessionId = session_id.parse().map_err(|_| {
            crate::persistence::JsonlError::InvalidSessionId(session_id.to_string())
        })?;
        Ok(crate::Session::with_message_store(self.clone(), id, store))
    }

    /// Materialize a fresh in-process runtime for this agent, seeded with the
    /// given session id, routing the runtime's raw event bus through the supplied
    /// facade sink so a [`Session`](crate::Session) can stream its events.
    ///
    /// Each call assembles a new `Harness`/`Agent`/`Session` composition, so
    /// distinct session ids yield independent sessions. This is the private seam
    /// [`Session`](crate::Session) builds on. The bus replaces the default
    /// in-memory emitter; message persistence is unaffected because the bus
    /// assigns event ids/sequences the same way.
    pub(crate) async fn build_runtime_with_event_bus(
        &self,
        session_id: SessionId,
        event_bus: Arc<dyn EventBus>,
        message_store: Option<Arc<dyn RuntimeMessageStore>>,
    ) -> Result<InProcessRuntime, everruns_core::AgentLoopError> {
        self.build_runtime_with_backends(session_id, Some(event_bus), message_store)
            .await
    }

    pub(crate) fn plugin_warnings(&self) -> Vec<String> {
        self.plugin_warnings.clone()
    }

    async fn build_runtime_with_backends(
        &self,
        session_id: SessionId,
        event_bus: Option<Arc<dyn EventBus>>,
        message_store: Option<Arc<dyn RuntimeMessageStore>>,
    ) -> Result<InProcessRuntime, everruns_core::AgentLoopError> {
        let mut harness = HarnessBuilder::new(&self.name, &self.instructions)
            .capabilities(self.capabilities.clone());
        if let Some(parallel) = self.parallel_tool_calls {
            harness = harness.parallel_tool_calls(parallel);
        }
        for file in &self.initial_files {
            harness = harness.initial_file(file.clone());
        }
        let harness_id = harness.harness_id();
        let harness = harness.build();

        let mut agent = RuntimeAgentBuilder::new(&self.name, &self.instructions)
            .harness_id(harness_id)
            .capabilities(self.capabilities.clone());
        if let Some(parallel) = self.parallel_tool_calls {
            agent = agent.parallel_tool_calls(parallel);
        }
        let agent_id = agent.agent_id();
        let agent = agent.build();

        let mut session = SessionBuilder::new(harness_id)
            .id(session_id)
            .agent(agent_id)
            .capabilities(self.capabilities.clone())
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
            .model_spec(self.model.spec.clone());

        let mut backends = RuntimeBackends::in_memory();
        if let Some(event_bus) = event_bus {
            backends = backends.with_event_bus(event_bus);
        }
        #[cfg(feature = "local")]
        if let Some(config) = &self.local {
            config
                .profile()
                .ensure_dirs()
                .map_err(|error| everruns_core::AgentLoopError::config(error.to_string()))?;
        }
        if let Some(message_store) = message_store {
            backends = backends.with_message_store(message_store);
        }

        #[cfg(feature = "local")]
        if let Some(config) = &self.local {
            let local = everruns_local::LocalBackends::new(config.profile(), backends)?;
            backends = local.runtime_backends;
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
                    everruns_core::CapabilityRegistry::with_builtins()
                } else {
                    everruns_core::CapabilityRegistry::runtime_builtins()
                }
                #[cfg(not(feature = "local"))]
                {
                    everruns_core::CapabilityRegistry::runtime_builtins()
                }
            };
            let platform = everruns_core::PlatformDefinition::builder()
                .capability_registry(registry)
                .driver_registry(everruns_core::DriverRegistry::new())
                .session_file_system_factory(Arc::new(
                    everruns_runtime::RealDiskSessionFileSystemFactory::new(root),
                ))
                .build();
            builder = builder.platform_definition(platform);
        }
        // Register each function tool as a closure-backed, single-tool
        // capability so the runtime can execute the model's calls; the matching
        // capability ref was attached to the harness/agent/session above.
        for tool in &self.function_tools {
            builder = builder.capability(tool.clone().into_capability());
        }
        #[cfg(feature = "capabilities")]
        for capability in &self.advanced_capabilities {
            builder = builder.capability(capability.runtime_adapter());
        }
        for provider in &self.providers {
            builder = builder.provider(provider.clone());
        }
        builder.build().await
    }
}

/// Fluent builder behind [`Agent::builder`].
///
/// Instructions and a model are required; everything else is optional. Blank
/// instructions or a missing model fail [`build`](Self::build) with a typed
/// [`BuildError`].
#[derive(Clone, Debug, Default)]
pub struct AgentBuilder {
    name: Option<String>,
    instructions: Option<String>,
    model: Option<Model>,
    providers: Vec<Provider>,
    capabilities: Vec<AgentCapabilityConfig>,
    tools: Vec<Tool>,
    #[cfg(feature = "capabilities")]
    advanced_capabilities: Vec<crate::capability::Definition>,
    initial_files: Vec<InitialFile>,
    parallel_tool_calls: Option<bool>,
    workspace_root: Option<PathBuf>,
    mcp_servers: Vec<crate::McpServer>,
    plugin_warnings: Vec<String>,
    compaction: Option<crate::CompactionConfig>,
    #[cfg(feature = "local")]
    local: Option<crate::LocalConfig>,
}

impl AgentBuilder {
    /// Set the agent's system instructions. Required.
    pub fn instructions(mut self, instructions: impl Into<String>) -> Self {
        self.instructions = Some(instructions.into());
        self
    }

    /// Select the model the agent talks to. Required.
    ///
    /// Accepts a [`Model`] directly or anything convertible into one — for
    /// example an [`OpenAI`](crate::providers::openai::OpenAI) configuration
    /// under the `openai` feature.
    pub fn model(mut self, model: impl Into<Model>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Register a service provider selected by the model specification.
    pub fn provider(mut self, provider: Provider) -> Self {
        self.providers.push(provider);
        self
    }

    /// Set a human-readable name. Optional; defaults to `"agent"`.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Add a tool the agent can call.
    ///
    /// Accepts anything that is [`IntoTool`](crate::IntoTool): a
    /// [`FunctionTool`](crate::FunctionTool) backed by an async function or
    /// closure, or a `&str`/`String` capability id for a capability-referenced
    /// tool. Tool names and JSON schemas are validated, and duplicate names
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
    ///     .tool("test_math")
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

    /// Add a capability the agent can use.
    pub fn capability(mut self, capability: impl Into<AgentCapabilityConfig>) -> Self {
        self.capabilities.push(capability.into());
        self
    }

    /// Register a code-defined capability through the curated advanced SPI.
    ///
    /// This both installs the implementation on the private in-process runtime
    /// and activates its stable id on the agent. Use [`#[everruns::tool]`](crate::tool)
    /// for ordinary single-function tools; use this method when a reusable
    /// capability needs typed protocol descriptors, context, progress, or
    /// call-scoped cancellation.
    #[cfg(feature = "capabilities")]
    pub fn advanced_capability(mut self, capability: crate::capability::Definition) -> Self {
        self.advanced_capabilities.push(capability);
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
            .any(|capability| capability.capability_id() == "session_file_system")
        {
            self.capabilities
                .push(AgentCapabilityConfig::new("session_file_system"));
        }
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

    /// Configure automatic context compaction for long-running sessions.
    pub fn compaction(mut self, config: crate::CompactionConfig) -> Self {
        self.compaction = Some(config);
        self
    }

    /// Enable local task/schedule state and a real workspace.
    ///
    /// Task and schedule state is stored in SQLite. Conversation persistence is
    /// event-derived and is not selected by this profile. Requires the `local`
    /// feature.
    #[cfg(feature = "local")]
    pub fn local(mut self, config: crate::LocalConfig) -> Self {
        self.local = Some(config);
        if !self
            .capabilities
            .iter()
            .any(|capability| capability.capability_id() == "session_file_system")
        {
            self.capabilities
                .push(AgentCapabilityConfig::new("session_file_system"));
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
    /// - [`BuildError::InvalidToolName`] if a function tool's name is not a
    ///   valid model-facing identifier.
    /// - [`BuildError::InvalidToolSchema`] if a function tool's JSON schema is
    ///   not a valid arguments schema.
    /// - [`BuildError::DuplicateTool`] if two `.tool(..)` calls share a name.
    /// - [`BuildError::InvalidMcpServer`] if an MCP server is unnamed,
    ///   incomplete, or duplicates another server name.
    /// - [`BuildError::InvalidCompaction`] if the proactive context budget is
    ///   outside the supported range.
    /// - [`BuildError::InvalidCapability`] if an advanced capability descriptor
    ///   is invalid.
    /// - [`BuildError::DuplicateCapability`] if advanced capabilities share an id.
    pub fn build(self) -> Result<Agent, BuildError> {
        let instructions = self.instructions.unwrap_or_default();
        if instructions.trim().is_empty() {
            return Err(BuildError::BlankInstructions);
        }
        let model = self.model.ok_or(BuildError::MissingModel)?;
        let mut providers = self.providers;
        if let Some(provider) = model.bundled_provider.clone() {
            providers.push(provider);
        }
        let mut provider_ids = HashSet::new();
        for provider in &providers {
            if !provider_ids.insert(provider.id().clone()) {
                return Err(BuildError::DuplicateProvider {
                    id: provider.id().to_string(),
                });
            }
        }
        if !provider_ids.contains(&model.spec.provider) {
            let mut registered = provider_ids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            registered.sort();
            return Err(BuildError::UnknownProvider {
                requested: model.spec.provider.to_string(),
                registered,
            });
        }
        let name = self.name.unwrap_or_else(|| "agent".to_string());

        // Validate tools and split them into capability refs (attached to the
        // agent) and function tools (also registered on the runtime). Names
        // must be unique across all `.tool(..)` calls.
        let mut capabilities = self.capabilities;
        let mut function_tools = Vec::new();
        let mut seen_tool_names: HashSet<String> = HashSet::new();
        for tool in self.tools {
            let tool_name = tool.name().to_string();
            if !seen_tool_names.insert(tool_name.clone()) {
                return Err(BuildError::DuplicateTool { name: tool_name });
            }
            match tool {
                Tool::Capability(config) => capabilities.push(config),
                Tool::Function(function_tool) => {
                    validate_tool_name(function_tool.name()).map_err(|reason| {
                        BuildError::InvalidToolName {
                            name: tool_name.clone(),
                            reason,
                        }
                    })?;
                    validate_tool_schema(function_tool.schema()).map_err(|reason| {
                        BuildError::InvalidToolSchema {
                            name: tool_name.clone(),
                            reason,
                        }
                    })?;
                    capabilities.push(AgentCapabilityConfig::new(function_tool.name()));
                    function_tools.push(function_tool);
                }
            }
        }

        let mcp_servers = crate::mcp::into_scoped(self.mcp_servers)
            .map_err(|reason| BuildError::InvalidMcpServer { reason })?;
        if let Some(compaction) = self.compaction {
            if !(compaction.budget_percent.is_finite()
                && 0.1 <= compaction.budget_percent
                && compaction.budget_percent <= 1.0)
            {
                return Err(BuildError::InvalidCompaction {
                    reason: "budget_percent must be at least 0.1 and at most 1".to_string(),
                });
            }
            capabilities.push(compaction.capability_config());
        }

        #[cfg(feature = "capabilities")]
        let advanced_capabilities = {
            let mut seen_capability_ids: HashSet<String> = capabilities
                .iter()
                .map(|config| config.capability_id().to_string())
                .collect();
            for capability in &self.advanced_capabilities {
                capability
                    .validate()
                    .map_err(|reason| BuildError::InvalidCapability {
                        id: capability.id().to_string(),
                        reason,
                    })?;
                if !seen_capability_ids.insert(capability.id().to_string()) {
                    return Err(BuildError::DuplicateCapability {
                        id: capability.id().to_string(),
                    });
                }
                for tool in capability.tools() {
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
                capabilities.push(AgentCapabilityConfig::new(capability.id()));
            }
            self.advanced_capabilities
        };

        Ok(Agent {
            name,
            instructions,
            model,
            providers,
            capabilities,
            function_tools,
            #[cfg(feature = "capabilities")]
            advanced_capabilities,
            initial_files: self.initial_files,
            parallel_tool_calls: self.parallel_tool_calls,
            workspace_root: self.workspace_root,
            mcp_servers,
            plugin_warnings: self.plugin_warnings,
            #[cfg(feature = "local")]
            local: self.local,
        })
    }
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

        let mut session = agent.session();
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

        let mut session = agent.session();
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

    #[cfg(feature = "openai")]
    #[test]
    fn openai_model_reports_provider_without_leaking_key() {
        use crate::providers::openai::OpenAI;

        let model = Model::openai(OpenAI::new("gpt-5-mini", "sk-super-secret"));
        assert_eq!(model.spec, ModelSpec::on("openai", "gpt-5-mini"));
        assert_eq!(
            model.bundled_provider.as_ref().unwrap().id().as_str(),
            "openai"
        );
        // The value-first `Debug` reports shape only, never the key.
        let rendered = format!("{model:?}");
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
            .model(OpenAI::new("gpt-5-mini", "sk-test"))
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
