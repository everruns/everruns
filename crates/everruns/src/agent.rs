//! Value-first agent description (EVE-832).
//!
//! [`Agent::builder`] lets a library user describe an agent — instructions, a
//! model, optional tools and files — without constructing stored platform
//! records, IDs, timestamps, statuses, registries, or a
//! `HostComposition`. The builder validates the value-first configuration and
//! adapts it, inside [`AgentBuilder::build`], to the existing runtime builders.
//!
//! A built Agent is an immutable behavior description. An application-owned
//! [`Engine`](crate::Engine) owns session creation, identity, and resumption.

mod builder;
mod communication;
mod package;

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use everruns_contracts::model_spec::ModelSpec;
use everruns_contracts::runtime_provider::Provider;
use everruns_contracts::typed_id::SessionId;
use everruns_core::InitialFile;
use everruns_core::host::{
    AgentBuilder as RuntimeAgentBuilder, Environment, EnvironmentBindingError,
    EnvironmentBindingStore, EventLogError, EventSink, HarnessBuilder, HostBackends,
    InProcessRuntime, InProcessRuntimeBuilder, SessionBuilder, WorkspaceBackend,
};
#[cfg(not(target_arch = "wasm32"))]
use everruns_llmsim::{LlmSimConfig, LlmSimDriver};

use crate::agent_state::AgentState;
use crate::capability_config::{
    framework_capability_registry, validate_registered_capability_config,
};
use crate::tool::{FunctionTool, IntoTool, Tool, validate_tool_name, validate_tool_schema};
#[cfg(feature = "builtins")]
mod approval;
#[cfg(feature = "builtins")]
mod ask_user;
mod capability_implementation;
use capability_implementation::CapabilityImplementation;

/// A model selected for an [`Agent`].
///
/// Live models can carry their provider with [`Model::new`] or use a plain
/// provider-visible id with a separate [`AgentBuilder::provider`] call.
/// [`Model::simulated`] bundles its private offline provider so deterministic
/// agents need no extra setup.
#[derive(Clone)]
pub struct Model {
    id: String,
    bundled_provider: Option<Provider>,
    /// Provider named by [`Model::on`]; resolved by key rather than carried.
    provider_key: Option<everruns_contracts::runtime_provider::ProviderKey>,
}

impl Model {
    /// Select a live model and the provider used to reach it.
    ///
    /// Bundling these values is convenient when model selection is application
    /// configuration. Advanced builders can still configure a plain model id
    /// and provider separately.
    pub fn new(id: impl Into<String>, provider: impl Into<Provider>) -> Self {
        Self::bundled(id, provider.into())
    }

    /// A deterministic in-process model that always replies with `response`.
    ///
    /// Backed by the `llmsim` driver, so an agent using it runs entirely
    /// offline — no credentials, no network.
    #[cfg(not(target_arch = "wasm32"))]
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
    #[cfg(not(target_arch = "wasm32"))]
    pub fn simulated_with_config(config: LlmSimConfig) -> Self {
        Self::bundled(
            "llmsim-model",
            Provider::new("llmsim", LlmSimDriver::new(config)),
        )
    }

    /// Select a model served by the provider registered under `provider`.
    ///
    /// The key is credential-free. Host provider services
    /// ([`HostProviders`](crate::hosting::services::HostProviders)) resolve it,
    /// or a separately configured [`AgentBuilder::provider`] with the same key
    /// serves it. A provider under any other key is a
    /// [`BuildError::ConflictingProviders`]; nothing falls back to another one.
    pub fn on(provider: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            bundled_provider: None,
            provider_key: Some(everruns_contracts::runtime_provider::ProviderKey::from(
                provider.into(),
            )),
        }
    }

    fn bundled(id: impl Into<String>, provider: Provider) -> Self {
        Self {
            id: id.into(),
            bundled_provider: Some(provider),
            provider_key: None,
        }
    }

    /// The provider-visible model id.
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    /// The provider bundled with this model, if any. A model built from a bare
    /// id carries none: the agent builder takes one from `.provider(...)`, and
    /// a direct completion asks for one explicitly.
    pub(crate) fn bundled_provider(&self) -> Option<&Provider> {
        self.bundled_provider.as_ref()
    }
}

impl From<&str> for Model {
    fn from(id: &str) -> Self {
        Self {
            id: id.to_string(),
            bundled_provider: None,
            provider_key: None,
        }
    }
}

impl From<String> for Model {
    fn from(id: String) -> Self {
        Self {
            id,
            bundled_provider: None,
            provider_key: None,
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
        capture: std::sync::Arc<
            std::sync::Mutex<Vec<Vec<everruns_contracts::driver_registry::Message>>>,
        >,
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
        tool_call_sequence: Vec<Vec<everruns_contracts::tool_types::ToolCall>>,
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
            .field("provider_key", &self.provider_key)
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
    /// This target cannot supply a requested native host service.
    UnsupportedHostFeature(&'static str),
    /// Both [`AgentBuilder::backends`] and [`AgentBuilder::local`] were set.
    ConflictingBackends,
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
    /// The model, Agent and host services name different providers, or a
    /// host-resolved model also carries an Agent-owned provider.
    ConflictingProviders {
        /// Provider named by the model.
        model: String,
        /// Provider supplied by the Agent or `"host"` for host services.
        provider: String,
    },
    /// More than one provider was attached to the agent.
    MultipleProviders {
        /// Stable keys of the providers attached to the agent.
        registered: Vec<String>,
    },
    /// MCP server configuration was invalid or duplicated.
    InvalidMcpServer {
        /// Why the MCP server configuration was rejected.
        reason: String,
    },
    /// Canonical duplicate workspace-backend error name.
    ///
    /// [`AgentBuilder::build`] continues to emit
    /// [`DuplicateWorkspaceProvider`](Self::DuplicateWorkspaceProvider) during
    /// its deprecation window.
    DuplicateWorkspaceBackend {
        /// The colliding workspace backend id.
        id: String,
    },
    /// Compatibility variant emitted during its deprecation window.
    #[deprecated(note = "use DuplicateWorkspaceBackend")]
    DuplicateWorkspaceProvider {
        /// The colliding workspace backend id.
        id: String,
    },
    /// A tool requires approval but no approver was configured.
    ///
    /// Set one with `AgentBuilder::approver` (requires the default `builtins`
    /// feature). The build fails rather than letting gated calls run
    /// unapproved. Stability: alpha.
    MissingApprover {
        /// The first tool that needs approval.
        tool: String,
    },
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedHostFeature(feature) => write!(f, "host does not support {feature}"),
            BuildError::BlankInstructions => {
                write!(f, "agent instructions must not be blank")
            }
            BuildError::MissingModel => write!(f, "agent requires a model"),
            BuildError::ConflictingBackends => {
                write!(f, "agent cannot combine `backends(..)` with `local(..)`")
            }
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
            BuildError::ConflictingProviders { model, provider } => write!(
                f,
                "model provider {model:?} conflicts with provider {provider:?}"
            ),
            BuildError::MultipleProviders { registered } => write!(
                f,
                "agent accepts one provider; registered providers: [{}]",
                registered.join(", ")
            ),
            BuildError::InvalidMcpServer { reason } => {
                write!(f, "invalid MCP server configuration: {reason}")
            }
            BuildError::DuplicateWorkspaceBackend { id } => {
                write!(f, "duplicate workspace backend id {id:?}")
            }
            #[allow(deprecated)]
            BuildError::DuplicateWorkspaceProvider { id } => {
                write!(f, "duplicate workspace backend id {id:?}")
            }
            BuildError::MissingApprover { tool } => write!(
                f,
                "tool {tool:?} needs approval but the agent has no approver"
            ),
        }
    }
}

impl std::error::Error for BuildError {}

/// The immutable, validated description of an agent.
///
/// Produced by [`AgentBuilder::build`]. It holds the value-first configuration
/// and materializes independent in-process runtimes when an execution engine
/// creates sessions from it. The underlying runtime composition remains
/// private.
#[derive(Clone)]
pub struct Agent {
    name: String,
    instructions: String,
    model: ModelSpec,
    /// `None` exactly when host provider services resolve the model.
    provider: Option<Provider>,
    capabilities: Vec<everruns_contracts::CapabilityRef>,
    capability_implementations: Vec<CapabilityImplementation>,
    initial_files: Vec<InitialFile>,
    network_access: Option<everruns_core::network_access::NetworkAccessList>,
    package_manifest: Option<everruns_core::agent_package::Manifest>,
    max_iterations: Option<usize>,
    parallel_tool_calls: Option<bool>,
    communication: crate::conversation::Communication,
    workspace_root: Option<PathBuf>,
    default_workspace: crate::default_workspace::DefaultWorkspace,
    workspace_policy: everruns_core::WorkspacePolicy,
    mcp_servers: everruns_core::ScopedMcpServers,
    plugin_warnings: Vec<String>,
    #[cfg(feature = "local")]
    local: Option<crate::LocalConfig>,
    lifecycle_hooks: crate::hooks::LifecycleHooks,
    client_tools: Vec<everruns_contracts::tool_types::ClientSideTool>,
    client_questions: bool,
    /// See [`AgentBuilder::host_services`].
    host_services: Option<crate::hosting::services::HostServices>,
    /// See [`AgentBuilder::backends`]; mutually exclusive with `local`.
    pub(crate) backends: Option<HostBackends>,
    pub(crate) state: Arc<AgentState>,
}

impl fmt::Debug for Agent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let capability_ids = self
            .capabilities
            .iter()
            .map(everruns_contracts::CapabilityRef::capability_id)
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
            .field("host_services", &self.host_services)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

#[allow(dead_code)] // Only constructed by the optional local persistence edge.
pub(crate) enum BackendInitError {
    Event(EventLogError),
    Host(everruns_contracts::error::AgentLoopError),
}

impl BackendInitError {
    pub(crate) fn into_agent_loop(self) -> everruns_contracts::error::AgentLoopError {
        match self {
            Self::Event(error) => {
                everruns_contracts::error::AgentLoopError::store(error.to_string())
            }
            Self::Host(error) => error,
        }
    }

    pub(crate) fn history_error(&self) -> crate::HistoryError {
        match self {
            Self::Event(EventLogError::Corruption { .. }) => crate::HistoryError::Corrupt,
            Self::Event(_) | Self::Host(_) => crate::HistoryError::Unavailable,
        }
    }

    pub(crate) fn resume_error(&self) -> crate::ResumeError {
        match self {
            Self::Event(EventLogError::Corruption { .. }) => crate::ResumeError::Corrupt,
            Self::Event(_) | Self::Host(_) => crate::ResumeError::Unavailable,
        }
    }
}

impl Agent {
    pub(crate) fn requires_hosted_workspace(&self) -> bool {
        self.workspace_root.is_some()
            || !self.initial_files.is_empty()
            || self.capabilities.iter().any(|reference| {
                matches!(
                    reference.capability_id(),
                    "filesystem"
                        | "file_system"
                        | "session_file_system"
                        | "skills"
                        | "host_shell"
                        | "bash"
                        | "bashkit"
                        | "bashkit_shell"
                        | "virtual_bash"
                )
            })
    }

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

    pub(crate) async fn bind_session_environment(
        &self,
        binding_store: &dyn EnvironmentBindingStore,
        session_id: SessionId,
        environment: &Environment,
    ) -> Result<(), crate::SessionEnvironmentError> {
        self.validate_session_environment(environment)?;
        let head = environment.workspace_head();
        binding_store
            .bind(session_id, head.binding())
            .await
            .map_err(crate::SessionEnvironmentError::from)?;
        Ok(())
    }

    pub(crate) fn validate_session_environment(
        &self,
        environment: &Environment,
    ) -> Result<(), crate::SessionEnvironmentError> {
        if !self
            .state
            .remember_backend(environment.workspace_head().backend())
        {
            return Err(legacy_workspace_provider_conflict());
        }
        Ok(())
    }

    /// Trusted host factory for this Agent's configured workspace. Register it
    /// with an immutable Environment reference on the Engine. Persistent hosts
    /// must provision the workspace storage independently of session authority.
    pub fn environment_factory(&self) -> impl crate::hosting::EnvironmentFactory {
        let configured = self.clone();
        move |session| {
            let configured = configured.clone();
            async move {
                configured
                    .default_session_environment(session)
                    .await
                    .map_err(|error| {
                        everruns_contracts::error::AgentLoopError::config(error.to_string())
                    })
            }
        }
    }

    /// Reopen a separate durable directory workspace for each session. The
    /// host must preserve `root` independently of the session journal.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn directory_environment_factory(
        &self,
        root: impl Into<PathBuf>,
    ) -> impl crate::hosting::EnvironmentFactory {
        crate::hosting::DirectoryEnvironmentFactory::new(root)
    }

    pub(crate) async fn default_session_environment(
        &self,
        session_id: SessionId,
    ) -> Result<Environment, crate::SessionEnvironmentError> {
        self.default_workspace
            .environment(session_id)
            .await
            .map_err(crate::SessionEnvironmentError::Workspace)
    }

    pub(crate) async fn reopen_session_environment(
        &self,
        binding_store: &dyn EnvironmentBindingStore,
        session_id: SessionId,
    ) -> Result<Option<Environment>, crate::ResumeError> {
        let Some(binding) = binding_store
            .load(session_id)
            .await
            .map_err(|error| match error {
                EnvironmentBindingError::Corrupt => crate::ResumeError::WorkspaceBindingCorrupt,
                EnvironmentBindingError::Conflict | EnvironmentBindingError::Unavailable => {
                    crate::ResumeError::Unavailable
                }
                _ => crate::ResumeError::Unavailable,
            })?
        else {
            return Ok(None);
        };
        let backend = self.state.backend(&binding.provider_id).ok_or_else(|| {
            legacy_workspace_provider_unavailable(binding.provider_id.to_string())
        })?;
        let descriptor = backend
            .open_workspace_from_binding(&binding)
            .await
            .map_err(map_workspace_resume_error)?;
        let workspace = everruns_core::host::Workspace::from_descriptor(backend, descriptor);
        let head = workspace
            .reopen(&binding)
            .await
            .map_err(map_workspace_resume_error)?;
        Ok(Some(
            Environment::builder()
                .workspace(head)
                .build()
                // A reopened head with no compute request cannot fail
                // assembly today; treat any future failure as an
                // unavailable workspace rather than resuming half-bound.
                .map_err(|_| crate::ResumeError::WorkspaceUnavailable)?,
        ))
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn lifecycle_hooks(&self) -> crate::hooks::LifecycleHooks {
        self.lifecycle_hooks.clone()
    }

    #[cfg(feature = "local")]
    pub(crate) fn local_config(&self) -> Option<&crate::LocalConfig> {
        self.local.as_ref()
    }

    pub(crate) async fn catalog_session(
        &self,
        backends: &HostBackends,
        session_id: SessionId,
    ) -> Result<(), crate::HistoryError> {
        if backends
            .session_store
            .get_session(session_id)
            .await
            .map_err(|_| crate::HistoryError::Unavailable)?
            .is_some()
        {
            return Ok(());
        }
        // Catalog identity is intentionally independent from event presence.
        // A zero-message session becomes resumable without inventing a history
        // event or treating orphan events as a session record.
        let session = SessionBuilder::new(everruns_contracts::typed_id::HarnessId::new())
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
        backends: HostBackends,
        session_id: SessionId,
        environment: Option<Environment>,
        harness: Option<&crate::Harness>,
        event_sink: Arc<dyn EventSink>,
        hook_state: Arc<crate::hooks::HookRunState>,
    ) -> Result<InProcessRuntime, everruns_contracts::error::AgentLoopError> {
        self.build_runtime_with_backends(
            backends,
            session_id,
            environment,
            harness,
            Some(event_sink),
            Some(hook_state),
        )
        .await
    }

    pub(crate) fn plugin_warnings(&self) -> Vec<String> {
        self.plugin_warnings.clone()
    }

    pub(crate) async fn build_runtime_with_backends(
        &self,
        backends: HostBackends,
        session_id: SessionId,
        environment: Option<Environment>,
        bound_harness: Option<&crate::Harness>,
        event_sink: Option<Arc<dyn EventSink>>,
        hook_state: Option<Arc<crate::hooks::HookRunState>>,
    ) -> Result<InProcessRuntime, everruns_contracts::error::AgentLoopError> {
        self.runtime_builder(
            backends,
            session_id,
            environment,
            bound_harness,
            event_sink,
            hook_state,
        )
        .await?
        .build()
        .await
    }

    pub(crate) async fn build_context_runtime(
        &self,
        backends: HostBackends,
        session: SessionId,
        environment: Environment,
        harness: Option<&crate::Harness>,
    ) -> Result<InProcessRuntime, everruns_contracts::error::AgentLoopError> {
        self.runtime_builder(backends, session, Some(environment), harness, None, None)
            .await?
            .for_context_query()
            .build()
            .await
    }

    async fn runtime_builder(
        &self,
        mut backends: HostBackends,
        session_id: SessionId,
        environment: Option<Environment>,
        bound_harness: Option<&crate::Harness>,
        event_sink: Option<Arc<dyn EventSink>>,
        hook_state: Option<Arc<crate::hooks::HookRunState>>,
    ) -> Result<
        everruns_core::host::InProcessRuntimeBuilder,
        everruns_contracts::error::AgentLoopError,
    > {
        backends = crate::batteries::runtime_backends(backends);
        let host_services = self.host_services.as_ref();
        if let Some(providers) = host_services.and_then(|services| services.providers.as_ref()) {
            // Resolve before execution: an unknown host provider is a
            // configuration failure, never a fallback to another provider.
            providers.ensure_serves(&self.model.provider).await?;
            backends.provider_store = Arc::new(crate::hosting::services::HostProviderStore::new(
                providers.clone(),
            ));
        }
        let hook_capability = hook_state.and_then(|state| state.capability());
        let mut capabilities = self.capabilities.clone();
        if hook_capability.is_some() {
            capabilities.retain(|capability| {
                capability.capability_id() != crate::hooks::LIFECYCLE_HOOK_CAPABILITY_ID
            });
            capabilities.push(everruns_contracts::CapabilityRef::new(
                crate::hooks::LIFECYCLE_HOOK_CAPABILITY_ID,
            ));
        }
        let harness = match bound_harness {
            Some(harness) => harness.seeded(),
            None => {
                // Decision: the synthetic harness carries no prompt. The
                // agent definition below owns the instructions, and the
                // runtime folds harness + agent prompts additively, so
                // setting both rendered them twice.
                let mut harness =
                    HarnessBuilder::new(&self.name, "").capabilities(capabilities.clone());
                if let Some(parallel) = self.parallel_tool_calls {
                    harness = harness.parallel_tool_calls(parallel);
                }
                for file in &self.initial_files {
                    harness = harness.initial_file(file.clone());
                }
                harness.build()
            }
        };
        let harness_id = harness.id;

        // EVE-877: the builder produces a portable AgentDefinition; the
        // harness link lives on the session, not the definition.
        let mut agent = RuntimeAgentBuilder::new(&self.name, &self.instructions)
            .capabilities(capabilities.clone())
            .communication(self.communication);
        if let Some(parallel) = self.parallel_tool_calls {
            agent = agent.parallel_tool_calls(parallel);
        }
        if let Some(max_iterations) = self.max_iterations {
            agent = agent.max_iterations(max_iterations);
        }
        if let Some(access) = &self.network_access {
            agent = agent.network_access(access.clone());
        }
        let agent_id = agent.agent_id();
        let agent = agent.build();

        let mut session = SessionBuilder::new(harness_id)
            .id(session_id)
            .agent(agent_id)
            .capabilities(capabilities)
            .mcp_servers(self.mcp_servers.clone());
        for tool in &self.client_tools {
            session = session.tool(everruns_contracts::tool_types::ToolDefinition::ClientSide(
                tool.clone(),
            ));
        }
        if let Some(environment) = &environment {
            session = session.workspace(environment.workspace_head().workspace_id());
        }
        if let Some(parallel) = self.parallel_tool_calls {
            session = session.parallel_tool_calls(parallel);
        }
        if let Some(access) = &self.network_access {
            session = session.network_access(access.clone());
        }
        for file in &self.initial_files {
            session = session.initial_file(file.clone());
        }
        let mut session =
            crate::engine::keep_catalog_fields(backends.session_store.as_ref(), session.build())
                .await;
        if !self.client_tools.is_empty() || self.client_questions {
            let hints = session.hints.get_or_insert_with(Default::default);
            if !self.client_tools.is_empty() {
                hints.insert("setup_connection".into(), true.into());
            }
            if self.client_questions {
                hints.insert("ask_user".into(), true.into());
            }
        }
        let mut builder = InProcessRuntimeBuilder::new()
            .host_composition(
                everruns_core::host::HostComposition::builder()
                    .capability_registry(framework_capability_registry(false))
                    .egress_service(crate::batteries::runtime_egress_service())
                    .session_file_system_factory(Arc::new(
                        everruns_core::host::InMemorySessionFileSystemFactory,
                    ))
                    .build(),
            )
            .harness(harness)
            .agent(agent)
            .session(session)
            .model_spec(self.model.clone())
            .workspace_policy(self.workspace_policy.clone());

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
        if let Some(environment) = environment {
            let registry = {
                #[cfg(feature = "local")]
                if self.local.is_some() {
                    crate::local::local_capability_registry()
                } else {
                    framework_capability_registry(false)
                }
                #[cfg(not(feature = "local"))]
                {
                    framework_capability_registry(false)
                }
            };
            let platform = everruns_core::host::HostComposition::builder()
                .capability_registry(registry)
                .driver_registry(everruns_contracts::driver_registry::DriverRegistry::new())
                .egress_service(crate::batteries::runtime_egress_service())
                .session_file_system_factory(Arc::new(
                    everruns_core::host::FixedSessionFileSystemFactory::new(
                        environment.workspace_head().file_system(),
                    ),
                ))
                .build();
            builder = builder.host_composition(platform);
        } else if let Some(root) = workspace_root {
            #[cfg(target_arch = "wasm32")]
            return Err(everruns_contracts::error::AgentLoopError::config(format!(
                "native workspace unavailable: {}",
                root.display()
            )));
            #[cfg(not(target_arch = "wasm32"))]
            {
                // THREAT[TM-BASH-001] / THREAT[TM-FS-013]: reuse the canonical
                // real-disk factory so every operation retains containment and
                // symlink rejection; the facade does not implement a second path
                // mapper.
                let registry = {
                    #[cfg(feature = "local")]
                    if self.local.is_some() {
                        // The local profile serves the hosted catalog from SQLite
                        // (EVE-885), so validation and execution see one set.
                        crate::local::local_capability_registry()
                    } else {
                        framework_capability_registry(false)
                    }
                    #[cfg(not(feature = "local"))]
                    {
                        framework_capability_registry(false)
                    }
                };
                let platform = everruns_core::host::HostComposition::builder()
                    .capability_registry(registry)
                    .driver_registry(everruns_contracts::DriverRegistry::new())
                    .egress_service(crate::batteries::runtime_egress_service())
                    .session_file_system_factory(Arc::new(
                        everruns_core::host::RealDiskSessionFileSystemFactory::new(root),
                    ))
                    .build();
                builder = builder.host_composition(platform);
            }
        }
        // References and code-defined implementations were normalized and
        // collision-checked together at Agent build time. Register each
        // implementation once on this session's private runtime; the matching
        // activation ref is already attached at every configuration layer.
        for implementation in &self.capability_implementations {
            builder = implementation.register(builder);
        }
        #[cfg(feature = "builtins")]
        if let Some(waits) = approval::waits_on_person(&self.capability_implementations) {
            builder = builder.waits_on_person(waits);
        }
        if let Some(capability) = hook_capability {
            builder = builder.capability(capability);
        }
        if let Some(services) = host_services {
            builder = apply_host_services(builder, services);
        }
        Ok(match &self.provider {
            Some(provider) => builder.provider(provider.clone()),
            None => builder,
        })
    }
}

/// Apply host services after the Framework selected the Environment-backed
/// composition. Nothing here replaces the session filesystem, egress, stores or
/// the turn runner; capability ids were collision-checked at Agent build time.
fn apply_host_services(
    mut builder: InProcessRuntimeBuilder,
    services: &crate::hosting::services::HostServices,
) -> InProcessRuntimeBuilder {
    for capability in &services.capabilities {
        builder = builder.shared_capability(capability.implementation.clone());
    }
    if let Some(providers) = &services.providers {
        builder = builder.driver_registry(providers.drivers.clone());
    }
    if let Some(retry) = &services.retry {
        builder = builder.provider_retry_config(retry.clone());
    }
    if let Some(timeout) = services.stall_timeout {
        builder = builder.provider_stall_timeout(timeout);
    }
    #[cfg(feature = "mcp")]
    if let Some(auth) = &services.mcp_auth {
        builder = builder.mcp_auth_provider(auth.clone());
    }
    builder
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
#[derive(Clone, Default)]
pub struct AgentBuilder {
    name: Option<String>,
    instructions: Option<String>,
    model: Option<Model>,
    providers: Vec<Provider>,
    capabilities: Vec<crate::CapabilitySpec>,
    tools: Vec<Tool>,
    client_tools: Vec<everruns_contracts::tool_types::ClientSideTool>,
    client_questions: bool,
    initial_files: Vec<InitialFile>,
    network_access: Option<everruns_core::network_access::NetworkAccessList>,
    package_manifest: Option<everruns_core::agent_package::Manifest>,
    max_iterations: Option<usize>,
    parallel_tool_calls: Option<bool>,
    communication: crate::conversation::Communication,
    workspace_root: Option<PathBuf>,
    workspace_policy: everruns_core::WorkspacePolicy,
    workspace_backends: Vec<Arc<dyn WorkspaceBackend>>,
    mcp_servers: Vec<crate::McpServer>,
    plugin_warnings: Vec<String>,
    #[cfg(feature = "builtins")]
    ask_user: Option<everruns_core::builtins::AskUserCapability>,
    #[cfg(feature = "builtins")]
    approver: Option<Arc<dyn everruns_core::builtins::ToolApprover>>,
    #[cfg(feature = "local")]
    local: Option<crate::LocalConfig>,
    lifecycle_hooks: crate::hooks::LifecycleHooks,
    host_services: Option<crate::hosting::services::HostServices>,
    /// See [`AgentBuilder::backends`]; mutually exclusive with `local`.
    pub(crate) backends: Option<HostBackends>,
}

impl fmt::Debug for AgentBuilder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentBuilder")
            .field("name", &self.name)
            .field("instructions", &self.instructions)
            .field("workspace_root", &self.workspace_root)
            .field(
                "workspace_backends",
                &self
                    .workspace_backends
                    .iter()
                    .map(|backend| backend.id())
                    .collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

#[allow(deprecated)]
fn map_workspace_resume_error(error: everruns_core::host::WorkspaceError) -> crate::ResumeError {
    match error {
        everruns_core::host::WorkspaceError::BindingMismatch => {
            crate::ResumeError::WorkspaceMismatch
        }
        everruns_core::host::WorkspaceError::NotFound
        | everruns_core::host::WorkspaceError::Archived
        | everruns_core::host::WorkspaceError::BackendUnavailable(_)
        | everruns_core::host::WorkspaceError::ProviderUnavailable(_)
        | everruns_core::host::WorkspaceError::Backend(_)
        | everruns_core::host::WorkspaceError::Provider(_)
        | everruns_core::host::WorkspaceError::Conflict
        | everruns_core::host::WorkspaceError::InvalidRequest(_) => {
            crate::ResumeError::WorkspaceUnavailable
        }
        _ => crate::ResumeError::WorkspaceUnavailable,
    }
}

#[allow(deprecated)]
fn legacy_duplicate_workspace_provider(id: String) -> BuildError {
    BuildError::DuplicateWorkspaceProvider { id }
}

#[allow(deprecated)]
fn legacy_workspace_provider_conflict() -> crate::SessionEnvironmentError {
    crate::SessionEnvironmentError::ProviderConflict
}

#[allow(deprecated)]
fn legacy_workspace_provider_unavailable(provider_id: String) -> crate::ResumeError {
    crate::ResumeError::WorkspaceProviderUnavailable { provider_id }
}

#[cfg(feature = "builtins")]
pub(crate) fn validate_tool_search_config(config: &serde_json::Value) -> Result<(), String> {
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
#[path = "agent_tests.rs"]
mod tests;
