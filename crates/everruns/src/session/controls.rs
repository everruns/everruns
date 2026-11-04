//! Session commands, live capability controls and host queries (EVE-1297).
//!
//! A host that drives a [`Session`] must keep its slash commands, hot-installed
//! extensions and live model, reasoning and MCP settings consistent with the
//! session's next model call, without reaching for a runtime that could drive
//! a second turn loop. Every operation here works on the session's own runtime
//! (the one its turns run on) or, for coherent hosting, on the read-only query
//! runtime its inspection uses. None of them starts a turn, appends to
//! canonical history or takes execution ownership.
//!
//! # Ordering and concurrency
//!
//! The legacy session actor answers these operations even while a turn runs,
//! so a tool of the running turn (a model-facing `run_command`, say) can call
//! them without deadlocking on the actor mailbox. A change made between two
//! model calls applies from the next one, including later calls of the turn
//! that is running. An operation awaited before a `send` applies to the turn
//! that send starts or steers.
//!
//! # Where state lives
//!
//! Capability code is trusted host code: the Agent, its host services or
//! [`Engine::register_capability`](crate::Engine::register_capability)
//! register it, and it is never deserialized. MCP server definitions can
//! carry credentials, so live MCP edits are held by the owning Engine and
//! never persisted with the session.
//!
//! - **Coherent hosting** (the default Engine and
//!   [`EngineBuilder::hosting`](crate::EngineBuilder::hosting)): queries and
//!   commands run on the query runtime inspection uses. The capability
//!   overlay, model and reasoning effort commit through the profile's
//!   `configure` authority operation, so they are durable and reach every
//!   activation, in this process or another. A running activation in this
//!   process sees model and effort changes from its next model call and
//!   overlay changes from its next reason boundary; Engine-registered code
//!   and live MCP edits reach it from its next turn and next activation.
//!   Session-record writes a command makes, other than to the capability
//!   overlay, are not retained.
//! - **Explicit legacy lane** (custom backends and local profiles): the
//!   session's own runtime runs every operation. The capability overlay
//!   persists in the session record (`ExecutionSession::capabilities`), the
//!   field Core's `activate_capability` writes. Model and effort selections
//!   are held by the owning Engine like MCP edits: they survive Session handle
//!   drops and runtime rebuilds within that Engine and are not persisted, so
//!   after a process restart the host applies them again, as it re-attaches
//!   the Agent and re-registers capability code. This lane's semantics are
//!   deliberately weaker than authority-owned settings.
//!
//! Stability: experimental.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::driver_registry::ProviderConfig;
use everruns_contracts::error::{AgentLoopError, Result as LoopResult};
use everruns_contracts::model_spec::ModelSpec;
use everruns_contracts::runtime_provider::ProviderKey;
use everruns_contracts::typed_id::{ModelId, SessionId};
use everruns_core::host::{InProcessRuntime, RuntimeProviderStore, RuntimeSessionStore};
use everruns_core::provider_resolution::ProviderStore;
use everruns_core::{ReasoningEffortHandle, ScopedMcpServer, ScopedMcpServers};

use super::{Command, RunError, Session, oneshot};
use crate::{CapabilityRef, Model, ReasoningEffort, SessionContext};

/// Why a session control or query was refused.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum SessionControlError {
    /// The request names something this session cannot use: an unknown
    /// command or capability, an invalid configuration, an inherited
    /// capability, or an unavailable model, provider or effort. Nothing was
    /// changed, and nothing fell back to another selection.
    Rejected(String),
    /// The session's hosting profile does not support this operation.
    Unsupported(String),
    /// The session could not run the operation.
    Run(RunError),
    /// The operation did not finish within the deadline the request set.
    /// It was abandoned, with nothing appended to history.
    TimedOut(Duration),
}

impl fmt::Display for SessionControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected(reason) => write!(f, "session control rejected: {reason}"),
            Self::Unsupported(reason) => write!(f, "session control unsupported: {reason}"),
            Self::Run(error) => write!(f, "session control failed: {error}"),
            Self::TimedOut(after) => write!(f, "session control timed out after {after:?}"),
        }
    }
}

impl std::error::Error for SessionControlError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Run(error) => Some(error),
            _ => None,
        }
    }
}

impl From<RunError> for SessionControlError {
    fn from(error: RunError) -> Self {
        Self::Run(error)
    }
}

impl From<AgentLoopError> for SessionControlError {
    fn from(error: AgentLoopError) -> Self {
        match error {
            AgentLoopError::Configuration(reason) => Self::Rejected(reason),
            other => Self::Run(other.into()),
        }
    }
}

/// Why [`Engine::register_capability`](crate::Engine::register_capability)
/// refused a capability.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CapabilityRegistrationError {
    /// The capability id is blank.
    InvalidId,
    /// The id or one of its aliases is already served by the Framework
    /// catalog or by a different registered capability.
    Conflict {
        /// The colliding id or alias.
        id: String,
    },
}

impl fmt::Display for CapabilityRegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId => f.write_str("capability id must not be blank"),
            Self::Conflict { id } => write!(f, "capability id {id:?} is already registered"),
        }
    }
}

impl std::error::Error for CapabilityRegistrationError {}

/// The result of a capability command run through [`Session::run_command`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CommandOutput {
    /// Whether the command reported success.
    pub success: bool,
    /// Human-readable result.
    pub message: String,
    /// Stable error code when the command failed in a classified way.
    pub error_code: Option<String>,
}

/// The effect of a session capability activation or deactivation.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CapabilityChange {
    /// Canonical capability id.
    pub capability_id: String,
    /// Whether the session overlay changed.
    pub changed: bool,
    /// Whether the capability is effective for the next model call.
    pub active: bool,
}

/// Provider metadata key [`CompletionRequest::purpose`] sets.
pub const COMPLETION_PURPOSE_KEY: &str = "purpose";
/// Most messages a [`CompletionRequest`] may carry.
pub const MAX_COMPLETION_MESSAGES: usize = 64;
/// Most system prompts a [`CompletionRequest`] may carry.
pub const MAX_COMPLETION_SYSTEM_PROMPTS: usize = 8;

/// A host-curated, tool-less completion request for
/// [`Session::complete`].
///
/// The host chooses the whole envelope: its system prompts, then its
/// messages, nothing else. The session contributes only what it owns: its
/// effective provider and model (live selections included), the Agent's call
/// settings, file and image resolution, provider errors and the
/// authority-owned `session_id` metadata. Neither the session's instructions
/// nor its conversation, capability message filters or model views are
/// added. A host that wants filtered session context reads
/// [`Session::command_context`] and bounds it into its own messages.
///
/// ```
/// use everruns::CompletionRequest;
///
/// let request = CompletionRequest::new()
///     .system_prompt("Decide whether the task is complete.")
///     .user("Evidence: tests pass, PR merged.")
///     .purpose("completion_review");
/// # let _ = request;
/// ```
///
/// Stability: experimental.
#[derive(Clone, Debug, Default)]
#[must_use]
pub struct CompletionRequest {
    system_prompts: Vec<String>,
    messages: Vec<everruns_core::RuntimeMessage>,
    metadata: BTreeMap<String, String>,
    reasoning_effort: Option<ReasoningEffort>,
    timeout: Option<Duration>,
}

impl CompletionRequest {
    /// An empty request; add at least one message.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a system prompt. Prompts are sent in order, before every
    /// message. A blank prompt is rejected.
    pub fn system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompts.push(prompt.into());
        self
    }

    /// Append a user message.
    pub fn user(self, text: impl Into<String>) -> Self {
        self.message(everruns_core::RuntimeMessage::user(text))
    }

    /// Append an assistant message, for example an earlier answer the host
    /// asks the model to check.
    pub fn assistant(self, text: impl Into<String>) -> Self {
        self.message(everruns_core::RuntimeMessage::assistant(text))
    }

    /// Append a prepared user or assistant message. Image and file
    /// references resolve through the session's resolvers. System, tool-call
    /// and tool-result messages are rejected: the request is tool-less and
    /// system text belongs in [`system_prompt`](Self::system_prompt).
    pub fn message(mut self, message: everruns_core::RuntimeMessage) -> Self {
        self.messages.push(message);
        self
    }

    /// Mark what the completion is for, sent as provider metadata
    /// `purpose` (for example `completion_review`).
    pub fn purpose(self, purpose: impl Into<String>) -> Self {
        self.metadata(COMPLETION_PURPOSE_KEY, purpose)
    }

    /// Add provider metadata. Keys are 1-64 lowercase ASCII letters, digits,
    /// `_`, `-` or `.`; values are at most 256 bytes; at most eight entries.
    /// Keys the host stamps from its own authority (`session_id`, `turn_id`,
    /// `model_id` and the like) are rejected, never overwritten. Request
    /// metadata wins over the host's own request metadata.
    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Reasoning effort for this request only. Without it the request uses
    /// the session's current selection
    /// ([`Session::set_reasoning_effort`]). An effort the effective model's
    /// curated profile cannot honor is rejected.
    pub fn reasoning_effort(mut self, effort: ReasoningEffort) -> Self {
        self.reasoning_effort = Some(effort);
        self
    }

    /// Abandon the completion after `timeout` with
    /// [`SessionControlError::TimedOut`].
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    fn validate(&self) -> Result<(), SessionControlError> {
        let rejected = |reason: String| Err(SessionControlError::Rejected(reason));
        if self.system_prompts.len() > MAX_COMPLETION_SYSTEM_PROMPTS {
            return rejected(format!(
                "a completion request carries at most {MAX_COMPLETION_SYSTEM_PROMPTS} system \
                 prompts"
            ));
        }
        if self
            .system_prompts
            .iter()
            .any(|prompt| prompt.trim().is_empty())
        {
            return rejected("completion system prompts must not be blank".into());
        }
        if self.messages.is_empty() || self.messages.len() > MAX_COMPLETION_MESSAGES {
            return rejected(format!(
                "a completion request carries 1-{MAX_COMPLETION_MESSAGES} messages"
            ));
        }
        for message in &self.messages {
            use everruns_core::RuntimeMessageRole;
            if !matches!(
                message.role,
                RuntimeMessageRole::User | RuntimeMessageRole::Agent
            ) || message.has_tool_calls()
                || message.tool_result_content().is_some()
            {
                return rejected(
                    "completion messages are user or assistant text and attachments; system \
                     text goes in system prompts, and tool calls or results are not allowed"
                        .into(),
                );
            }
        }
        if self.timeout.is_some_and(|timeout| timeout.is_zero()) {
            return rejected("completion timeout must be positive".into());
        }
        Ok(())
    }
}

/// Trusted, process-local live settings of one session (see the module docs).
#[derive(Default)]
pub(crate) struct LiveControls {
    model: Arc<RwLock<Option<ModelSpec>>>,
    /// The session's effort selection, which every turn starts from.
    effort: ReasoningEffortHandle,
    /// The running turn's live effort (Core resets it at each turn start).
    turn_effort: ReasoningEffortHandle,
    /// Live MCP edits by server name: `Some` adds or replaces a server,
    /// `None` removes one the Agent declared.
    mcp: Mutex<BTreeMap<String, Option<ScopedMcpServer>>>,
    /// Serializes changes so validation and application see one state.
    gate: tokio::sync::Mutex<()>,
}

impl fmt::Debug for LiveControls {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // THREAT[TM-TOOL-007]: MCP definitions may carry credentials.
        f.debug_struct("LiveControls")
            .field("model", &self.selected_model())
            .field("effort", &self.effort.get())
            .finish_non_exhaustive()
    }
}

impl LiveControls {
    fn selected_model(&self) -> Option<ModelSpec> {
        self.model
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Wrap a runtime's provider store so the live model selection is the
    /// session default. Per-message model controls still win, as before.
    ///
    /// `agent_provider` is the only provider a selection may name when no
    /// host provider services resolve providers (they refuse unknown keys
    /// themselves).
    pub(crate) fn provider_store(
        &self,
        inner: Arc<dyn RuntimeProviderStore>,
        agent_provider: Option<ProviderKey>,
    ) -> Arc<dyn RuntimeProviderStore> {
        Arc::new(LiveModelStore {
            inner,
            selection: self.model.clone(),
            agent_provider,
        })
    }

    /// The live model and effort selections, as a cutover carries them into
    /// authority configuration (EVE-1298).
    pub(crate) fn selections(&self) -> (Option<ModelSpec>, Option<ReasoningEffort>) {
        (self.selected_model(), self.effort.get())
    }

    /// Adopt the model and effort a coherent hosting authority retains.
    pub(crate) fn sync_authority(&self, configuration: &crate::hosting::SessionConfiguration) {
        *self
            .model
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            configuration.model.clone().flatten();
        self.effort.set(configuration.reasoning_effort.flatten());
    }

    /// The running turn's live handle and the session selection it starts from.
    pub(crate) fn effort_handles(&self) -> (ReasoningEffortHandle, ReasoningEffortHandle) {
        (self.turn_effort.clone(), self.effort.clone())
    }

    fn set_effort(&self, effort: Option<ReasoningEffort>) {
        self.effort.set(effort);
        // The running turn's next model call, too.
        self.turn_effort.set(effort);
    }

    /// Apply live MCP edits over the Agent-derived servers of a fresh record.
    pub(crate) fn apply_mcp(&self, servers: &mut ScopedMcpServers) {
        let edits = self
            .mcp
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for (name, server) in edits.iter() {
            match server {
                Some(server) => {
                    servers.insert(name.clone(), server.clone());
                }
                None => {
                    servers.remove(name);
                }
            }
        }
    }
}

/// Provider store whose default model is the session's live selection.
struct LiveModelStore {
    inner: Arc<dyn RuntimeProviderStore>,
    selection: Arc<RwLock<Option<ModelSpec>>>,
    agent_provider: Option<ProviderKey>,
}

#[async_trait]
impl ProviderStore for LiveModelStore {
    async fn get_model_spec(&self, model_id: ModelId) -> LoopResult<Option<ModelSpec>> {
        self.inner.get_model_spec(model_id).await
    }

    async fn get_default_model_spec(&self) -> LoopResult<Option<ModelSpec>> {
        let selected = self
            .selection
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        match selected {
            // THREAT[TM-LLM-052]: a retained selection whose provider the
            // Agent no longer serves fails instead of letting resolution guess
            // an external driver under that name.
            Some(model)
                if self
                    .agent_provider
                    .as_ref()
                    .is_some_and(|provider| *provider != model.provider) =>
            {
                Err(AgentLoopError::config(format!(
                    "selected provider {} is not configured for this agent; refusing to fall \
                     back to another provider",
                    model.provider
                )))
            }
            Some(model) => Ok(Some(model)),
            None => self.inner.get_default_model_spec().await,
        }
    }

    async fn get_provider_config_for_session(
        &self,
        provider: &ProviderKey,
        session: SessionId,
    ) -> LoopResult<Option<ProviderConfig>> {
        self.inner
            .get_provider_config_for_session(provider, session)
            .await
    }

    async fn get_provider_config(
        &self,
        provider: &ProviderKey,
    ) -> LoopResult<Option<ProviderConfig>> {
        self.inner.get_provider_config(provider).await
    }
}

#[async_trait]
impl RuntimeProviderStore for LiveModelStore {
    async fn set_default_model_spec(&self, model: ModelSpec) -> LoopResult<()> {
        self.inner.set_default_model_spec(model).await
    }
}

/// A coherent session's read-only query runtime with its throwaway record
/// store and the trusted Agent it was built from.
pub(crate) struct HostedQuery {
    pub(crate) runtime: InProcessRuntime,
    pub(crate) store: Arc<dyn RuntimeSessionStore>,
    pub(crate) agent: crate::Agent,
}

/// Which runtime an operation reached. Short-lived, one per operation.
#[allow(clippy::large_enum_variant)]
enum ControlRuntime {
    /// The legacy session's own runtime, shared with its turns.
    Live(InProcessRuntime),
    /// A coherent session's query runtime; changes commit through the
    /// hosting profile's `configure` authority operation.
    Hosted(HostedQuery),
}

impl ControlRuntime {
    fn runtime(&self) -> &InProcessRuntime {
        match self {
            Self::Live(runtime) => runtime,
            Self::Hosted(query) => &query.runtime,
        }
    }
}

fn capability_ids(record: &everruns_core::session::ExecutionSession) -> Vec<CapabilityRef> {
    record.capabilities.clone()
}

impl Session {
    async fn control_runtime(&self) -> Result<ControlRuntime, SessionControlError> {
        if self.inner.execution.hosting().is_some() {
            if self.inner.execution.frontend_environment() {
                self.start().await.map_err(RunError::Environment)?;
            }
            self.inner
                .execution
                .ensure_cataloged()
                .await
                .map_err(|error| RunError::from(AgentLoopError::store(error.to_string())))?;
            return Ok(ControlRuntime::Hosted(
                self.inner
                    .execution
                    .query_runtime()
                    .await
                    .map_err(RunError::from)?,
            ));
        }
        let (response, result) = oneshot::channel();
        self.command_sender()
            .await?
            .send(Command::Runtime { response })
            .await
            .map_err(|_| RunError::SessionClosed)?;
        Ok(ControlRuntime::Live(
            result.await.map_err(|_| RunError::SessionClosed)??,
        ))
    }

    /// Commit a configuration patch through the coherent hosting profile.
    async fn configure_hosted(
        &self,
        patch: crate::hosting::SessionConfiguration,
    ) -> Result<(), SessionControlError> {
        let profile = self.inner.execution.hosting().ok_or_else(|| {
            SessionControlError::Unsupported("session has no hosting profile".into())
        })?;
        patch.validate()?;
        profile
            .configure(self.session_id(), uuid::Uuid::new_v4(), patch)
            .await
            .map_err(RunError::from)?;
        Ok(())
    }

    /// Persist a query runtime's session capability overlay as authority
    /// configuration.
    async fn commit_hosted_overlay(&self, query: &HostedQuery) -> Result<(), SessionControlError> {
        let record = query
            .store
            .get_session(self.session_id())
            .await?
            .ok_or_else(|| AgentLoopError::session_not_found(self.session_id()))?;
        self.configure_hosted(crate::hosting::SessionConfiguration {
            capabilities: Some(capability_ids(&record)),
            ..Default::default()
        })
        .await
    }

    /// Slash commands the session's active capabilities contribute now, in
    /// capability resolution order and deduplicated by name.
    ///
    /// This is the list [`run_command`](Self::run_command) dispatches against
    /// and the one [`inspect`](Self::inspect) reports, so every frontend of
    /// one session discovers the same commands. Answered during an active
    /// turn as well.
    pub async fn commands(&self) -> Result<Vec<crate::CommandInfo>, SessionControlError> {
        let runtime = self.control_runtime().await?;
        let commands = runtime.runtime().list_commands(self.session_id()).await?;
        Ok(commands
            .into_iter()
            .map(crate::context::CommandInfo::from_descriptor)
            .collect())
    }

    /// Execute a capability command against the session's effective context.
    ///
    /// `name` may carry its leading `/`. Dispatch resolves the session's
    /// capability chain exactly as the next model call will, and the command
    /// receives the same provider, filters and filesystem as a turn. It runs
    /// out of band: no turn starts and no message is appended to history.
    /// Calling it from a tool of the running turn is supported. An unknown
    /// command is [`SessionControlError::Rejected`].
    ///
    /// On coherent hosting the command runs on the query runtime; a change it
    /// makes to the session capability overlay is committed through the
    /// hosting profile, and other session-record writes are not retained.
    pub async fn run_command(
        &self,
        name: &str,
        arguments: Option<&str>,
    ) -> Result<CommandOutput, SessionControlError> {
        let name = name.trim().trim_start_matches('/').to_string();
        if name.is_empty() {
            return Err(SessionControlError::Rejected(
                "command name must not be blank".into(),
            ));
        }
        let request = everruns_core::command::ExecuteCommandRequest {
            name,
            arguments: arguments
                .map(str::trim)
                .filter(|arguments| !arguments.is_empty())
                .map(str::to_string),
            controls: None,
        };
        let result = match self.control_runtime().await? {
            ControlRuntime::Live(runtime) => {
                runtime.execute_command(self.session_id(), request).await?
            }
            ControlRuntime::Hosted(query) => {
                let live = self.inner.execution.live_controls();
                let _gate = live.gate.lock().await;
                let overlay = |record: Option<everruns_core::session::ExecutionSession>| {
                    record.map(|record| serde_json::to_value(capability_ids(&record)).ok())
                };
                let before = overlay(query.store.get_session(self.session_id()).await?);
                let result = query
                    .runtime
                    .execute_command(self.session_id(), request)
                    .await?;
                if overlay(query.store.get_session(self.session_id()).await?) != before {
                    self.commit_hosted_overlay(&query).await?;
                }
                result
            }
        };
        Ok(CommandOutput {
            success: result.success,
            message: result.message,
            error_code: result.error_code,
        })
    }

    /// Activate a registered capability on this session.
    ///
    /// The capability must be registered with the Agent, its host services or
    /// [`Engine::register_capability`](crate::Engine::register_capability).
    /// Its configuration is validated and its dependencies resolved before
    /// the session overlay changes; on failure nothing changes. Activating an
    /// already effective capability is a no-op. The activation is persisted
    /// in the session's capability overlay and applies from the next model
    /// call.
    pub async fn activate_capability(
        &self,
        capability: impl Into<CapabilityRef>,
    ) -> Result<CapabilityChange, SessionControlError> {
        let capability = capability.into();
        let live = self.inner.execution.live_controls();
        let _gate = live.gate.lock().await;
        let runtime = self.control_runtime().await?;
        self.ensure_engine_capability(runtime.runtime(), capability.capability_id())?;
        let delta = runtime
            .runtime()
            .activate_capability(self.session_id(), capability)
            .await?;
        if let ControlRuntime::Hosted(query) = &runtime
            && delta.changed
        {
            self.commit_hosted_overlay(query).await?;
        }
        Ok(CapabilityChange {
            capability_id: delta.capability_id,
            changed: delta.changed,
            active: delta.active,
        })
    }

    /// Deactivate a capability this session activated.
    ///
    /// The capability leaves the next model call's tools, instructions,
    /// hooks and commands. Capabilities the Agent or Harness declares cannot
    /// be removed at the session layer and are
    /// [`SessionControlError::Rejected`]. Deactivating an inactive capability
    /// is a no-op.
    pub async fn deactivate_capability(
        &self,
        capability_id: &str,
    ) -> Result<CapabilityChange, SessionControlError> {
        let live = self.inner.execution.live_controls();
        let _gate = live.gate.lock().await;
        let runtime = self.control_runtime().await?;
        let agent = match &runtime {
            ControlRuntime::Live(_) => self.inner.execution.agent_snapshot(),
            ControlRuntime::Hosted(query) => query.agent.clone(),
        };
        let canonical = runtime
            .runtime()
            .registered_capability(capability_id)
            .map_or_else(
                || capability_id.to_string(),
                |capability| capability.id().to_string(),
            );
        // Agent capabilities belong to the agent layer even where an older
        // record also copied them into the session layer.
        if agent.capability_ids().contains(&canonical) {
            return Err(SessionControlError::Rejected(format!(
                "capability {canonical} is inherited from the Agent and cannot be deactivated at \
                 the session layer"
            )));
        }
        let delta = runtime
            .runtime()
            .deactivate_capability(self.session_id(), capability_id)
            .await?;
        if let ControlRuntime::Hosted(query) = &runtime
            && delta.changed
        {
            self.commit_hosted_overlay(query).await?;
        }
        Ok(CapabilityChange {
            capability_id: delta.capability_id,
            changed: delta.changed,
            active: delta.active,
        })
    }

    /// Make an Engine-registered capability resolvable on a runtime built
    /// before it was registered.
    fn ensure_engine_capability(
        &self,
        runtime: &InProcessRuntime,
        capability_id: &str,
    ) -> Result<(), SessionControlError> {
        let registered = self.inner.execution.registered_capabilities();
        let Some(capability) = registered.iter().find(|capability| {
            capability.id() == capability_id || capability.aliases().contains(&capability_id)
        }) else {
            return Ok(());
        };
        register_engine_capability(runtime, capability).map_err(SessionControlError::from)
    }

    /// Select the model for this session's next model calls.
    ///
    /// A plain model id selects a model of the Agent's provider;
    /// [`Model::on`] selects one by provider key. The provider must be one
    /// the Agent or its host provider services already serve: an unknown
    /// provider is [`SessionControlError::Rejected`] and leaves the current
    /// selection and saved state as they were. A model value that bundles its
    /// own provider is rejected too, because a live selection never changes
    /// credentials or provider code. Returns the effective selection.
    pub async fn set_model(
        &self,
        model: impl Into<Model>,
    ) -> Result<ModelSpec, SessionControlError> {
        let live = self.inner.execution.live_controls();
        let _gate = live.gate.lock().await;
        let runtime = self.control_runtime().await?;
        let agent = match &runtime {
            ControlRuntime::Live(_) => self.inner.execution.agent_snapshot(),
            ControlRuntime::Hosted(query) => query.agent.clone(),
        };
        let spec = agent
            .live_model_spec(model.into())
            .await
            .map_err(SessionControlError::Rejected)?;
        if let Some(effort) = live.effort.get() {
            let context = runtime.runtime().load_context(self.session_id()).await?;
            check_effort(&context.model.provider_type, &spec.model, effort)?;
        }
        if let ControlRuntime::Hosted(_) = runtime {
            self.configure_hosted(crate::hosting::SessionConfiguration {
                model: Some(Some(spec.clone())),
                ..Default::default()
            })
            .await?;
        }
        *live
            .model
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(spec.clone());
        Ok(spec)
    }

    /// Return to the Agent's model for the next model calls.
    pub async fn reset_model(&self) -> Result<(), SessionControlError> {
        let live = self.inner.execution.live_controls();
        let _gate = live.gate.lock().await;
        if self.inner.execution.hosting().is_some() {
            self.configure_hosted(crate::hosting::SessionConfiguration {
                model: Some(None),
                ..Default::default()
            })
            .await?;
        }
        *live
            .model
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        Ok(())
    }

    /// Set or clear the reasoning effort of the next model calls.
    ///
    /// The selection overrides effort carried by message controls and is
    /// re-read before every model call, so a change during a turn applies to
    /// its next call. An effort the effective model's curated profile says it
    /// cannot honor is [`SessionControlError::Rejected`] and leaves the
    /// current setting as it was. `None` returns to message-controlled effort.
    pub async fn set_reasoning_effort(
        &self,
        effort: Option<ReasoningEffort>,
    ) -> Result<(), SessionControlError> {
        let live = self.inner.execution.live_controls();
        let _gate = live.gate.lock().await;
        if let Some(effort) = effort {
            let runtime = self.control_runtime().await?;
            let context = runtime.runtime().load_context(self.session_id()).await?;
            check_effort(&context.model.provider_type, &context.model.model, effort)?;
        }
        if self.inner.execution.hosting().is_some() {
            self.configure_hosted(crate::hosting::SessionConfiguration {
                reasoning_effort: Some(effort),
                ..Default::default()
            })
            .await?;
        }
        live.set_effort(effort);
        Ok(())
    }

    /// Add or replace a scoped MCP server for this session.
    ///
    /// The server is trusted host configuration, validated like an Agent
    /// server. Its tools are discovered for the next model call. The
    /// definition is held by the owning Engine and never persisted with the
    /// session, so credentials never come back from serialized state. On
    /// coherent hosting it applies from the session's next activation.
    pub async fn add_mcp_server(
        &self,
        server: crate::McpServer,
    ) -> Result<(), SessionControlError> {
        let name = server.name().trim().to_string();
        let scoped = crate::mcp::into_scoped(vec![server])
            .map_err(SessionControlError::Rejected)?
            .remove(&name)
            .ok_or_else(|| SessionControlError::Rejected("invalid MCP server".into()))?;
        self.edit_mcp(name, Some(scoped)).await
    }

    /// Remove a scoped MCP server, live-added or Agent-declared, from this
    /// session. Removing a server the session does not have is
    /// [`SessionControlError::Rejected`].
    pub async fn remove_mcp_server(&self, name: &str) -> Result<(), SessionControlError> {
        self.edit_mcp(name.trim().to_string(), None).await
    }

    /// Rediscover this session's MCP tools, requesting credentials from the
    /// host's MCP auth provider again, for example after an auth refresh.
    /// On coherent hosting every activation and query discovers afresh.
    pub async fn refresh_mcp(&self) -> Result<(), SessionControlError> {
        if let ControlRuntime::Live(runtime) = self.control_runtime().await? {
            #[cfg(feature = "mcp")]
            runtime.invalidate_mcp_discovery(self.session_id());
            #[cfg(not(feature = "mcp"))]
            let _ = runtime;
        }
        Ok(())
    }

    async fn edit_mcp(
        &self,
        name: String,
        server: Option<ScopedMcpServer>,
    ) -> Result<(), SessionControlError> {
        let live = self.inner.execution.live_controls();
        let _gate = live.gate.lock().await;
        let runtime = self.control_runtime().await?;
        let store = match &runtime {
            ControlRuntime::Live(_) => self
                .inner
                .execution
                .backends()
                .await
                .map_err(|error| RunError::from(error.into_agent_loop()))?
                .host
                .session_store
                .clone(),
            ControlRuntime::Hosted(query) => query.store.clone(),
        };
        let mut record = store
            .get_session(self.session_id())
            .await?
            .ok_or_else(|| AgentLoopError::session_not_found(self.session_id()))?;
        match &server {
            Some(server) => {
                record.mcp_servers.insert(name.clone(), server.clone());
            }
            None => {
                if record.mcp_servers.remove(&name).is_none() {
                    return Err(SessionControlError::Rejected(format!(
                        "session has no MCP server {name:?}"
                    )));
                }
            }
        }
        // THREAT[TM-TOOL-065]: definitions may carry credentials, so they are
        // never committed to coherent authority configuration.
        if let ControlRuntime::Live(runtime) = &runtime {
            // The live record first, then the Engine-held edit that every
            // rebuild re-applies.
            store.add_session(record).await?;
            #[cfg(feature = "mcp")]
            runtime.invalidate_mcp_discovery(self.session_id());
            #[cfg(not(feature = "mcp"))]
            let _ = runtime;
        }
        live.mcp
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(name, server);
        Ok(())
    }

    /// The effective context a command or host-side review sees now.
    ///
    /// Same projection as [`inspect`](Self::inspect), assembled by the same
    /// path as the next model call (provider, message filters, capability
    /// instructions, tools and commands), but answered during an active turn
    /// too instead of waiting for its terminal boundary. During a turn it
    /// reflects what the turn has committed so far. It never mutates history,
    /// takes execution ownership or resumes work.
    pub async fn command_context(&self) -> Result<SessionContext, SessionControlError> {
        let runtime = self.control_runtime().await?;
        let warnings = match &runtime {
            ControlRuntime::Live(_) => self.inner.execution.agent_snapshot().plugin_warnings(),
            ControlRuntime::Hosted(query) => query.agent.plugin_warnings(),
        };
        let runtime = runtime.runtime();
        let context = runtime.load_context(self.session_id()).await?;
        let commands = runtime.list_commands(self.session_id()).await?;
        Ok(SessionContext::from_runtime(context, warnings, commands))
    }

    /// Run a tool-less, out-of-band completion over this session's effective
    /// context, for host-side review.
    ///
    /// The request is the session's filtered conversation and effective
    /// instructions, then `instructions` (if any) as a further system prompt,
    /// then `prompt` as a user message. It uses the session's resolved model,
    /// provider and live selections, exactly as the next model call would.
    /// Nothing is appended to history, no tool runs and no turn starts.
    ///
    /// This is the convenience form of [`complete`](Self::complete), whose
    /// host chooses the whole envelope instead.
    pub async fn complete_out_of_band(
        &self,
        instructions: Option<&str>,
        prompt: &str,
    ) -> Result<String, SessionControlError> {
        let runtime = self.control_runtime().await?;
        let host = runtime.runtime().command_host(self.session_id()).await?;
        let context = host.turn_context().await?;
        let mut messages = context.messages;
        messages.push(everruns_core::RuntimeMessage::user(prompt));
        let mut system_prompts = vec![context.system_prompt];
        system_prompts.extend(instructions.map(str::to_string));
        let request = everruns_core::SessionCompletionRequest {
            system_prompts,
            messages,
            ..Default::default()
        };
        completion_text(host.completion(request).await)
    }

    /// Run a host-curated, tool-less completion against this session's
    /// effective model (EVE-1303).
    ///
    /// The provider receives exactly the request's system prompts and
    /// messages (see [`CompletionRequest`]), with no tools, through the
    /// provider and model the session's next model call would use, live
    /// selections included. A selection the session can no longer serve
    /// fails instead of falling back to another provider or model. Provider
    /// metadata is the host's own request metadata, then the request's
    /// metadata, then the authority-owned `session_id`.
    ///
    /// Like [`complete_out_of_band`](Self::complete_out_of_band) it is
    /// answered during an active turn, including from a tool of that turn,
    /// and never starts a turn, appends to history or takes execution
    /// ownership. Dropping the future abandons the request.
    pub async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<String, SessionControlError> {
        request.validate()?;
        match request.timeout {
            Some(timeout) => tokio::time::timeout(timeout, self.complete_curated(request))
                .await
                .map_err(|_| SessionControlError::TimedOut(timeout))?,
            None => self.complete_curated(request).await,
        }
    }

    async fn complete_curated(
        &self,
        request: CompletionRequest,
    ) -> Result<String, SessionControlError> {
        let runtime = self.control_runtime().await?;
        let host = runtime.runtime().command_host(self.session_id()).await?;
        let effort = request
            .reasoning_effort
            .or_else(|| self.inner.execution.live_controls().effort.get());
        if let Some(effort) = effort {
            // The effective model is the one the session resolves now.
            let context = runtime.runtime().load_context(self.session_id()).await?;
            check_effort(&context.model.provider_type, &context.model.model, effort)?;
        }
        let completion = everruns_core::SessionCompletionRequest {
            system_prompts: request.system_prompts,
            messages: request.messages,
            controls: effort.map(|effort| everruns_core::Controls {
                reasoning: Some(everruns_core::ReasoningConfig {
                    effort: Some(effort),
                }),
                ..Default::default()
            }),
            metadata: request.metadata.into_iter().collect(),
        };
        completion_text(host.completion(completion).await)
    }
}

fn completion_text(
    result: Result<everruns_core::SessionCompletion, everruns_core::SessionCompletionError>,
) -> Result<String, SessionControlError> {
    match result {
        Ok(completion) => Ok(completion.text),
        Err(everruns_core::SessionCompletionError::InvalidRequest(error)) => Err(error.into()),
        Err(everruns_core::SessionCompletionError::StreamingUnsupported) => Err(
            SessionControlError::Unsupported("streaming completion".into()),
        ),
        Err(everruns_core::SessionCompletionError::Completion { error, .. }) => {
            Err(SessionControlError::Run(AgentLoopError::llm(error).into()))
        }
    }
}

/// Register an Engine capability on a runtime, or confirm it is already
/// there. A different implementation under the same id is a conflict, never
/// an override.
pub(crate) fn register_engine_capability(
    runtime: &InProcessRuntime,
    capability: &Arc<dyn everruns_core::capabilities::Capability>,
) -> LoopResult<()> {
    match runtime.registered_capability(capability.id()) {
        Some(existing) if crate::engine::same_capability(&existing, capability) => Ok(()),
        Some(_) => Err(AgentLoopError::config(format!(
            "capability {} is already registered by the Agent or Framework",
            capability.id()
        ))),
        None => runtime.register_capability(capability.clone()),
    }
}

fn check_effort(
    provider_type: &everruns_contracts::DriverId,
    model: &str,
    effort: ReasoningEffort,
) -> Result<(), SessionControlError> {
    let Some(profile) = crate::models::profile(provider_type, model) else {
        // No curated profile: the provider validates the request itself.
        return Ok(());
    };
    if effort.requests_reasoning() && !profile.reasoning {
        return Err(SessionControlError::Rejected(format!(
            "model {model} does not support reasoning effort {}",
            effort.as_str()
        )));
    }
    Ok(())
}
