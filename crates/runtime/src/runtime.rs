// In-process runtime builder and runner.
// Decision: the public runtime is in-memory today, but uses the same core atoms
// and capability resolution path as the durable worker so behavior stays close.

use crate::backends::{
    EventBus, RuntimeAgentStore, RuntimeBackends, RuntimeHarnessStore, RuntimeMessageStore,
    RuntimeProviderStore, RuntimeSessionStore,
};
use crate::builders::SingleSessionBuilder;
use crate::host::{
    RuntimeHostAdapter, RuntimeHostTurnContext, execute_act_activity, execute_input_activity,
    execute_reason_activity_with_prompt_messages,
};
use crate::in_memory::{InMemorySessionFileStore, InMemorySessionFileSystemFactory};
use crate::turn_strategy::{RuntimeTurnPlan, RuntimeTurnState};
use async_trait::async_trait;
use chrono::Utc;
use everruns_core::agent::Agent;
use everruns_core::atoms::{AtomContext, InputAtomInput, ReasonInput};
use everruns_core::capabilities::{
    Capability, CapabilityRegistry, CapabilityStatus, collect_capability_mcp_servers,
    resolve_capability_configs,
};
use everruns_core::config_layer::AgentConfigOverlay;
use everruns_core::driver_registry::{DriverId, DriverRegistry};
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::events::{
    Event, EventContext, EventData, EventRequest, InputMessageData, OutputMessageCompletedData,
    ToolCompletedData,
};
use everruns_core::harness::Harness;
use everruns_core::llmsim_driver::{LlmSimConfig, LlmSimDriver};
use everruns_core::message::{ContentPart, Message};
use everruns_core::platform_definition::PlatformDefinition;
use everruns_core::plugins::{PluginFileSet, compile_plugin};
use everruns_core::runtime_context::{AssembledTurnContext, inspect_turn_context};
use everruns_core::session::{Session, SessionStatus};
use everruns_core::session_file::{InitialFile, SessionFile};
use everruns_core::tools::ToolResultImage;
use everruns_core::traits::{
    AgentStore, EventEmitter, HarnessStore, ProviderStore, ResolvedModel, SessionMutator,
    SessionStorageStore, SessionStore, UserConnectionResolver,
};
use everruns_core::turn::TurnStopReason;
use everruns_core::typed_id::{AgentId, HarnessId, MessageId, OrgId, SessionId, TurnId};
use everruns_core::{
    AgentCapabilityConfig, CapabilityId, InputMessage, MessageRetriever, SessionFileSystem,
    SessionFileSystemFactoryContext, plugin_capability_id, resolve_runtime_capabilities,
};
use everruns_engine::{ActOutcome, plan_after_act, plan_after_process_input, plan_after_reason};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Arc;

/// Cap on the input length hashed by [`hash_public_org_id`].
///
/// Legitimate org public ids are `org_<32hex>` (36 bytes). Bounding the
/// hashed prefix keeps worst-case cost predictable when an attacker-controlled
/// session carries an oversize string.
const HASH_INPUT_CAP_BYTES: usize = 128;

/// Derive an internal `i64` org id from the public `org_<32hex>` form on a
/// [`Session`].
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
    pub turn_id: everruns_core::typed_id::TurnId,
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
    turn_id: everruns_core::typed_id::TurnId,
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
/// - `PlatformDefinition` for capabilities and drivers
/// - in-memory stores for sessions, files, storage, memory, and messages
/// - seeded harness/agent/session entities
///
/// `build()` returns an [`InProcessRuntime`] that can execute turns in-process
/// without the durable engine or the control-plane server.
pub struct InProcessRuntimeBuilder {
    platform_definition: PlatformDefinition,
    llm_sim_config: Option<LlmSimConfig>,
    providers: Vec<everruns_core::Provider>,
    model_spec: Option<everruns_core::ModelSpec>,
    legacy_provider_config: Option<everruns_core::ProviderConfig>,
    backends: Option<RuntimeBackends>,
    session_file_system_factory_context: SessionFileSystemFactoryContext,
    harnesses: Vec<Harness>,
    agents: Vec<Agent>,
    sessions: Vec<Session>,
    default_session_id: Option<SessionId>,
    seeded_files: Vec<(SessionId, InitialFile)>,
    mcp_auth_provider: Option<Arc<dyn everruns_mcp::McpAuthProvider>>,
    provider_retry_config: Option<everruns_core::llm_retry::LlmRetryConfig>,
    provider_stall_timeout: Option<std::time::Duration>,
    /// Hydrated capability configs for plugins loaded via [`Self::with_plugin_dir`].
    ///
    /// Keyed by `plugin:{name}`. Agents and harnesses reference these by the
    /// same `plugin:{name}` capability ref; the hydrated config carries the
    /// compiled `DeclarativeCapabilityDefinition` so no registry entry is needed.
    plugin_capability_configs: Vec<AgentCapabilityConfig>,
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
    /// - call [`Self::llm_sim`] for deterministic local examples/tests, or
    /// - register their own driver(s) on the platform definition and set a
    ///   default model via [`Self::default_model`].
    pub fn new() -> Self {
        Self {
            platform_definition: PlatformDefinition::builder()
                .capability_registry(CapabilityRegistry::runtime_builtins())
                .driver_registry(DriverRegistry::new())
                .session_file_system_factory(Arc::new(InMemorySessionFileSystemFactory))
                .build(),
            llm_sim_config: None,
            providers: Vec::new(),
            model_spec: None,
            legacy_provider_config: None,
            backends: None,
            session_file_system_factory_context: SessionFileSystemFactoryContext::new(),
            harnesses: Vec::new(),
            agents: Vec::new(),
            sessions: Vec::new(),
            default_session_id: None,
            seeded_files: Vec::new(),
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
    pub fn mcp_auth_provider(mut self, provider: Arc<dyn everruns_mcp::McpAuthProvider>) -> Self {
        self.mcp_auth_provider = Some(provider);
        self
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

    /// Register a canonical runtime provider selected by [`ModelSpec`](everruns_core::ModelSpec).
    pub fn provider(mut self, provider: everruns_core::Provider) -> Self {
        self.providers.push(provider);
        self
    }

    /// Register the built-in `llmsim` driver for deterministic local execution.
    pub fn llm_sim(mut self, config: LlmSimConfig) -> Self {
        self.llm_sim_config = Some(config);
        self
    }

    /// Set the runtime default model used when sessions/agents do not override it.
    pub fn default_model(mut self, model: ResolvedModel) -> Self {
        let (spec, provider_config) = model.canonical_parts();
        self.model_spec = Some(spec);
        self.legacy_provider_config = Some(provider_config);
        self
    }

    /// Select a credential-free model served by a provider registered on this builder.
    pub fn model_spec(mut self, model: everruns_core::ModelSpec) -> Self {
        self.model_spec = Some(model);
        self.legacy_provider_config = None;
        self
    }

    /// Supply a custom backend bundle instead of the built-in in-memory stores.
    pub fn backends(mut self, backends: RuntimeBackends) -> Self {
        self.backends = Some(backends);
        self
    }

    /// Inject a session-task registry. Convenience over `backends(...)` that
    /// initializes an in-memory backend bundle on first use, so embedders can
    /// add the registry without assembling a full `RuntimeBackends`.
    pub fn with_session_task_registry(
        mut self,
        registry: Arc<dyn everruns_core::session_task::SessionTaskRegistry>,
    ) -> Self {
        let backends = self
            .backends
            .take()
            .unwrap_or_else(RuntimeBackends::in_memory);
        self.backends = Some(backends.with_session_task_registry(registry));
        self
    }

    /// Inject a per-org schedule store factory (see [`RuntimeBackends`]).
    pub fn with_schedule_store_factory(
        mut self,
        factory: crate::backends::ScheduleStoreFactory,
    ) -> Self {
        let backends = self
            .backends
            .take()
            .unwrap_or_else(RuntimeBackends::in_memory);
        self.backends = Some(backends.with_schedule_store_factory(factory));
        self
    }

    /// Inject a per-(org, session) platform store factory (see [`RuntimeBackends`]).
    pub fn with_platform_store_factory(
        mut self,
        factory: crate::backends::PlatformStoreFactory,
    ) -> Self {
        let backends = self
            .backends
            .take()
            .unwrap_or_else(RuntimeBackends::in_memory);
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
        config: everruns_core::llm_retry::LlmRetryConfig,
    ) -> Self {
        self.provider_retry_config = Some(config);
        self
    }

    /// Override the no-output provider stream stall timeout.
    pub fn provider_stall_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.provider_stall_timeout = Some(timeout);
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
            .push(AgentCapabilityConfig::with_config(cap_id, hydrated_config));

        Ok(self)
    }

    /// Return a hydrated `AgentCapabilityConfig` for a previously loaded plugin.
    ///
    /// Returns `None` when no plugin with that name was loaded via
    /// [`Self::with_plugin_dir`]. Primarily used by callers that need the
    /// hydrated config to seed it onto a harness or agent before building.
    pub fn plugin_capability(&self, name: &str) -> Option<AgentCapabilityConfig> {
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
            None => RuntimeBackends::in_memory(),
        };
        let file_store = resolve_session_file_system(
            &self.platform_definition,
            self.session_file_system_factory_context.clone(),
        )
        .await?;

        if let Some(config) = self.llm_sim_config.take() {
            let driver = LlmSimDriver::new(config);
            // This legacy convenience adapts into the canonical provider path.
            self.platform_definition
                .driver_registry_mut()
                .replace_provider(everruns_core::Provider::new("llmsim", driver));

            if self.model_spec.is_none() {
                self.model_spec = Some(everruns_core::ModelSpec::on("llmsim", "llmsim-model"));
            }
        }

        for provider in self.providers {
            self.platform_definition
                .driver_registry_mut()
                .register_provider(provider)?;
        }

        let model_spec = self.model_spec.ok_or_else(|| {
            AgentLoopError::config(
                "in-process runtime requires a default model; call \
                 InProcessRuntimeBuilder::default_model(...) or \
                 InProcessRuntimeBuilder::llm_sim(...)",
            )
        })?;

        // `ResolvedModel` is a 0.17 compatibility input. Adapt it once into a
        // canonical registered provider; no credential-bearing model state is
        // retained by the runtime execution path.
        let legacy_config = self.legacy_provider_config.take();
        let provider_type = legacy_config
            .as_ref()
            .map(|config| config.provider_type.clone())
            .unwrap_or_else(|| DriverId::external(model_spec.provider.as_str()));
        if let Some(config) = legacy_config
            && self
                .platform_definition
                .driver_registry()
                .provider(&model_spec.provider)
                .is_none()
        {
            let driver = self
                .platform_definition
                .driver_registry()
                .create_chat_driver(&config)?;
            self.platform_definition
                .driver_registry_mut()
                .register_provider(everruns_core::Provider::new(
                    model_spec.provider.clone(),
                    driver,
                ))?;
        }

        let provider_metadata = if provider_type.as_str() == model_spec.provider.as_str() {
            None
        } else {
            Some(everruns_core::ProviderMetadata {
                extra: Some(serde_json::json!({
                    "provider_id": model_spec.provider.as_str(),
                })),
                ..Default::default()
            })
        };
        let default_model = ResolvedModel {
            model: model_spec.model,
            provider_type,
            api_key: None,
            base_url: None,
            provider_metadata,
        };

        backends
            .provider_store
            .set_default_model(default_model)
            .await?;

        // Hydrate bare plugin: refs in harnesses/agents/sessions with the
        // compiled definition config so the capability resolution path can
        // deserialise them without a registry entry (same path as declarative:).
        for harness in &mut self.harnesses {
            hydrate_plugin_refs(&mut harness.capabilities, &self.plugin_capability_configs);
        }
        for agent in &mut self.agents {
            hydrate_plugin_refs(&mut agent.capabilities, &self.plugin_capability_configs);
        }
        for session in &mut self.sessions {
            hydrate_plugin_refs(&mut session.capabilities, &self.plugin_capability_configs);
        }

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
                file_store.as_ref(),
                session,
            )
            .await?;
        }

        for (session_id, file) in &self.seeded_files {
            file_store.seed_initial_file(*session_id, file).await?;
        }

        let persisting_emitter =
            PersistingEventEmitter::new(backends.event_bus.clone(), backends.message_store.clone());

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

        Ok(InProcessRuntime {
            platform_definition: Arc::new(self.platform_definition),
            harness_store: backends.harness_store,
            agent_store: backends.agent_store,
            session_store: backends.session_store,
            default_session_id: self.default_session_id,
            message_store: backends.message_store,
            compaction_checkpoint_store: backends.compaction_checkpoint_store,
            provider_store: backends.provider_store,
            event_bus: backends.event_bus,
            persisting_emitter,
            file_store,
            storage_store: backends.storage_store,
            connection_resolver: backends.connection_resolver,
            session_task_registry,
            session_wake_queue,
            schedule_store_factory: backends.schedule_store_factory,
            platform_store_factory: backends.platform_store_factory,
            mcp_auth_provider: self
                .mcp_auth_provider
                .unwrap_or_else(|| Arc::new(everruns_mcp::NoAuthProvider)),
            provider_retry_config: self.provider_retry_config,
            provider_stall_timeout: self.provider_stall_timeout,
            mcp_discovery_cache: Arc::new(crate::mcp_cache::McpDiscoveryCache::new()),
            plugin_warnings: self.plugin_warnings,
        })
    }
}

async fn resolve_session_file_system(
    platform_definition: &PlatformDefinition,
    file_system_factory_context: SessionFileSystemFactoryContext,
) -> Result<Arc<dyn SessionFileSystem>> {
    let file_system_factory = platform_definition.session_file_system_factory();
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
    platform_definition: Arc<PlatformDefinition>,
    harness_store: Arc<dyn RuntimeHarnessStore>,
    agent_store: Arc<dyn RuntimeAgentStore>,
    session_store: Arc<dyn RuntimeSessionStore>,
    default_session_id: Option<SessionId>,
    message_store: Arc<dyn RuntimeMessageStore>,
    compaction_checkpoint_store: Arc<dyn everruns_core::CompactionCheckpointStore>,
    provider_store: Arc<dyn RuntimeProviderStore>,
    event_bus: Arc<dyn EventBus>,
    persisting_emitter: PersistingEventEmitter,
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
    mcp_auth_provider: Arc<dyn everruns_mcp::McpAuthProvider>,
    provider_retry_config: Option<everruns_core::llm_retry::LlmRetryConfig>,
    provider_stall_timeout: Option<std::time::Duration>,
    mcp_discovery_cache: Arc<crate::mcp_cache::McpDiscoveryCache>,
    /// Non-fatal warnings collected during plugin compilation (see
    /// [`InProcessRuntimeBuilder::with_plugin_dir`]).
    plugin_warnings: Vec<String>,
}

impl InProcessRuntime {
    /// Build the shared MCP client over the platform egress boundary and the
    /// configured auth provider.
    fn mcp_client(&self) -> Arc<everruns_mcp::McpClient> {
        Arc::new(everruns_mcp::McpClient::new(
            self.platform_definition.egress_service(),
            self.mcp_auth_provider.clone(),
        ))
    }

    /// Resolve the effective scoped MCP servers for a session.
    async fn session_mcp_servers(
        &self,
        session: &Session,
        agent: Option<&Agent>,
    ) -> everruns_core::ScopedMcpServers {
        let harness_chain = self
            .harness_store
            .get_harness_chain(session.harness_id)
            .await
            .unwrap_or_default();
        let resolved = resolve_runtime_capabilities(
            &harness_chain,
            agent,
            session,
            self.platform_definition.capability_registry(),
        );
        let contributed = collect_capability_mcp_servers(
            &resolved.resolved_capability_configs,
            self.platform_definition.capability_registry(),
        );
        let explicit = crate::mcp::merge_session_scoped_servers(&harness_chain, agent, session);
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
        capability: impl Into<AgentCapabilityConfig>,
    ) -> Result<CapabilityDelta> {
        let mut capability = capability.into();
        let registry = self.platform_definition.capability_registry();
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
            .validate_config(&capability.config)
            .map_err(|error| {
                AgentLoopError::config(format!("invalid capability config: {error}"))
            })?;

        let canonical_id = registered.id().to_string();
        capability.capability_ref = CapabilityId::new(canonical_id.clone());
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

        let mut candidate = context.effective_overlay.capabilities;
        candidate.push(capability.clone());
        resolve_capability_configs(&candidate, registry)
            .map_err(|error| AgentLoopError::config(error.to_string()))?;

        self.session_store
            .upsert_session_capability(session_id, capability)
            .await?;
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
        let registry = self.platform_definition.capability_registry();
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

        let session_capability_id = context
            .session
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
    /// The input message is stored in the runtime history, an `input.message`
    /// event is emitted, and the turn then runs `input -> reason -> act` as
    /// planned step-by-step by [`everruns_engine`] — the same planner the
    /// durable worker drives its turns with.
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

        // Input message is recorded directly (and emitted via the raw bus so
        // that PersistingEventEmitter does not double-store it). All
        // subsequent activity-emitted events flow through the persisting
        // emitter the adapter hands out.
        let input_message = self
            .message_store
            .add_input_message(session_id, input.into())
            .await?;
        self.event_bus
            .emit(EventRequest::new(
                session_id,
                EventContext::empty(),
                InputMessageData::new(input_message.clone()),
            ))
            .await?;

        let org_id = in_process_internal_org_id(&session.organization_id);
        let turn_id = TurnId::new();

        // Engine-planned turn loop (EVE-842). Every reason-vs-act-vs-complete
        // decision comes from `everruns-engine`; this loop only executes the
        // host operation each plan names and performs the lifecycle effects the
        // engine returns as data. There is no second copy of the planning brain
        // in the runtime.
        let mut state = RuntimeTurnState {
            org_id,
            session_id,
            harness_id: session.harness_id,
            agent_id: session.agent_id,
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
                .with_workspace_id(session.workspace_id);
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

        loop {
            match plan {
                RuntimeTurnPlan::ScheduleReason(next_state) => {
                    state = next_state;
                    // Iteration boundary: drain queued task wakes and inject
                    // them before the LLM call so this reason reacts to them
                    // (EVE-681, part A). Draining here also delivers wakes that
                    // arrived while the session was idle, on the next turn's
                    // first iteration (between-turn fallback).
                    let wake_message_ids = self.drain_and_inject_wakes(session_id).await?;
                    let mut prompt_message_ids = wake_message_ids;
                    if state.iteration == 1 {
                        prompt_message_ids.insert(0, input_message.id);
                    }
                    let reason_result = execute_reason_activity_with_prompt_messages(
                        self,
                        org_id,
                        ReasonInput {
                            context: base_context(true),
                            harness_id: session.harness_id,
                            agent_id: session.agent_id,
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

                    // If a wake landed during this reason (e.g. a background
                    // task settling on another task), continue a would-idle turn
                    // so it is delivered on the very next iteration rather than
                    // after the session idles. The engine reads this as the
                    // steering-message count the durable host reports.
                    let pending_user_message_count = usize::from(
                        self.session_wake_queue
                            .as_ref()
                            .is_some_and(|q| q.has_pending(session_id)),
                    );

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
            // Persist the wake as a user message (history reload picks it up)
            // and emit the input event on the raw bus so it appears in the turn
            // span without the persisting emitter double-storing it — mirroring
            // how `run_turn` records the initial input message.
            let message = self
                .message_store
                .add_input_message(session_id, InputMessage::user(wake.text))
                .await?;
            message_ids.push(message.id);
            self.event_bus
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

    /// Assemble the current runtime context for a session without executing a turn.
    pub async fn load_context(&self, session_id: SessionId) -> Result<AssembledTurnContext> {
        let session = self
            .session_store
            .get_session(session_id)
            .await?
            .ok_or_else(|| AgentLoopError::store(format!("session not found: {session_id}")))?;
        let agent = match session.agent_id {
            Some(agent_id) => self.agent_store.get_agent(agent_id).await?,
            None => None,
        };
        let scoped_servers = self.session_mcp_servers(&session, agent.as_ref()).await;
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
        self.inspect_context_with_ids(
            session_id,
            session.harness_id,
            session.agent_id,
            &mcp_tool_definitions,
        )
        .await
    }

    /// Return all collected events from the runtime event bus.
    ///
    /// Event buses that do not retain events return an empty `Vec` (see
    /// [`EventBus::collected_events`]).
    pub async fn events(&self) -> Result<Vec<Event>> {
        Ok(self.event_bus.collected_events().await)
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
        let registry = self.platform_definition.capability_registry();
        // Context-aware commands (e.g. /btw) get the same store-backed host
        // facilities the server provides; the already-assembled context seeds
        // the host so dispatch and execution assemble it once.
        let host = everruns_core::command_host::StoreCommandHost::new(
            session_id,
            self.harness_store.clone(),
            self.agent_store.clone(),
            self.session_store.clone(),
            self.message_store.clone(),
            self.provider_store.clone(),
            registry.clone(),
            self.platform_definition.driver_registry().clone(),
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
        let registry = self.platform_definition.capability_registry();
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

    async fn inspect_context_with_ids(
        &self,
        session_id: SessionId,
        harness_id: everruns_core::HarnessId,
        agent_id: Option<AgentId>,
        mcp_tool_definitions: &[everruns_core::ToolDefinition],
    ) -> Result<AssembledTurnContext> {
        inspect_turn_context(
            self.harness_store.as_ref(),
            self.agent_store.as_ref(),
            self.session_store.as_ref(),
            self.message_store.as_ref(),
            self.provider_store.as_ref(),
            self.platform_definition.capability_registry(),
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
    async fn get_agent(&self, _org_id: i64, agent_id: AgentId) -> Result<Option<Agent>> {
        self.agent_store.get_agent(agent_id).await
    }

    async fn get_harness(&self, _org_id: i64, harness_id: HarnessId) -> Result<Option<Harness>> {
        let chain = self.harness_store.get_harness_chain(harness_id).await?;
        Ok(chain.into_iter().last())
    }

    async fn set_session_status(
        &self,
        _org_id: i64,
        session_id: SessionId,
        _status: SessionStatus,
    ) -> Result<Session> {
        // The in-process runtime does not persist status. Lifecycle callers
        // still emit their events; downstream consumers in-process don't
        // observe session.status.
        self.session_store
            .get_session(session_id)
            .await?
            .ok_or_else(|| AgentLoopError::store(format!("session not found: {session_id}")))
    }

    async fn load_turn_context(
        &self,
        _org_id: i64,
        session_id: SessionId,
    ) -> Result<RuntimeHostTurnContext> {
        let mut session = self
            .session_store
            .get_session(session_id)
            .await?
            .ok_or_else(|| AgentLoopError::store(format!("session not found: {session_id}")))?;
        // Fold runtime ARD attachments into the session config layer before
        // scoped MCP servers / capabilities are resolved (resource_discovery).
        everruns_core::ard_attachment::apply_session_attachments(
            self.storage_store.as_ref(),
            &mut session,
        )
        .await;
        let agent = match session.agent_id {
            Some(agent_id) => self.agent_store.get_agent(agent_id).await?,
            None => None,
        };
        let messages = self.message_store.load(session_id).await?;
        let model = self.provider_store.get_default_model().await?;

        // Discover tools from the session's scoped MCP servers so they appear
        // to the LLM alongside built-in tools (knowledge/integrations/runtime-mcp.md D4).
        let scoped_servers = self.session_mcp_servers(&session, agent.as_ref()).await;
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

        Ok(RuntimeHostTurnContext {
            agent,
            session,
            messages,
            model,
            mcp_tool_definitions,
        })
    }

    async fn mcp_executor(
        &self,
        _org_id: i64,
        session_id: SessionId,
    ) -> Option<Arc<everruns_mcp::McpExecutor>> {
        let session = self.session_store.get_session(session_id).await.ok()??;
        let agent = match session.agent_id {
            Some(agent_id) => self.agent_store.get_agent(agent_id).await.ok().flatten(),
            None => None,
        };
        let scoped_servers = self.session_mcp_servers(&session, agent.as_ref()).await;
        crate::mcp::build_executor(self.mcp_client(), &scoped_servers)
    }

    fn capability_registry(&self) -> CapabilityRegistry {
        self.platform_definition.capability_registry().clone()
    }

    fn driver_registry(&self) -> DriverRegistry {
        self.platform_definition.driver_registry().clone()
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
        self.message_store.clone()
    }

    fn compaction_checkpoint_store(
        &self,
    ) -> Option<Arc<dyn everruns_core::CompactionCheckpointStore>> {
        Some(self.compaction_checkpoint_store.clone())
    }

    fn event_emitter(&self) -> Arc<dyn EventEmitter> {
        Arc::new(self.persisting_emitter.clone())
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
    ) -> Option<Arc<dyn everruns_core::traits::SessionScheduleStore>> {
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
        Some(self.platform_definition.utility_llm_service())
    }

    fn egress_service(&self) -> Option<Arc<dyn everruns_core::EgressService>> {
        Some(self.platform_definition.egress_service())
    }

    fn provider_retry_config(&self) -> Option<everruns_core::llm_retry::LlmRetryConfig> {
        self.provider_retry_config.clone()
    }

    fn provider_stall_timeout(&self) -> Option<std::time::Duration> {
        self.provider_stall_timeout
    }
}

#[derive(Clone)]
struct PersistingEventEmitter {
    inner: Arc<dyn EventBus>,
    message_store: Arc<dyn RuntimeMessageStore>,
}

impl PersistingEventEmitter {
    fn new(inner: Arc<dyn EventBus>, message_store: Arc<dyn RuntimeMessageStore>) -> Self {
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

/// Replace bare `plugin:{name}` refs (empty config) with the hydrated version
/// (config = serialised `DeclarativeCapabilityDefinition`) so that the core
/// capability resolution path can deserialise them without a registry entry.
///
/// Only replaces entries whose config is empty / `null`; entries that already
/// carry a non-empty config are left unchanged so explicit overrides are honoured.
fn hydrate_plugin_refs(
    capabilities: &mut [AgentCapabilityConfig],
    plugin_configs: &[AgentCapabilityConfig],
) {
    for cap in capabilities.iter_mut() {
        let cap_id = cap.capability_id();
        if !everruns_core::is_plugin_capability(cap_id) {
            continue;
        }
        // Only replace if the config is missing / empty so explicit overrides are honoured.
        let is_bare = cap.config.is_null()
            || cap
                .config
                .as_object()
                .map(|o| o.is_empty())
                .unwrap_or(false);
        if !is_bare {
            continue;
        }
        if let Some(hydrated) = plugin_configs.iter().find(|c| c.capability_id() == cap_id) {
            cap.config = hydrated.config.clone();
        }
    }
}

async fn seed_runtime_initial_files(
    harness_store: &dyn RuntimeHarnessStore,
    agent_store: &dyn RuntimeAgentStore,
    file_store: &dyn SessionFileSystem,
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
    // Seed into the session's workspace (a shared workspace differs from the
    // session id; the default 1:1 case is equal).
    let seed_key = SessionId::from_uuid(session.workspace_id.uuid());
    for file in &overlay.initial_files {
        file_store.seed_initial_file(seed_key, file).await?;
    }
    Ok(())
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
    let metadata = tool_result_metadata(&data);
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
            return parse_structured_tool_result_text(&text.text);
        }
        if !text_parts.is_empty() {
            serde_json::to_value(&text_parts).unwrap_or_default()
        } else {
            serde_json::Value::Null
        }
    });

    let mut message = if images.is_empty() {
        Message::tool_result(&data.tool_call_id, result, data.error)
    } else {
        Message::tool_result_with_images(&data.tool_call_id, result, images)
    };
    message.metadata = metadata;
    message
}

fn tool_result_metadata(
    data: &ToolCompletedData,
) -> Option<std::collections::HashMap<String, serde_json::Value>> {
    let mut metadata = std::collections::HashMap::new();
    metadata.insert("tool_name".to_string(), serde_json::json!(data.tool_name));
    if let Some(fingerprint) = &data.tool_call_fingerprint {
        metadata.insert(
            "tool_call_fingerprint".to_string(),
            serde_json::json!(fingerprint),
        );
    }
    if let Some(fingerprint) = &data.tool_result_fingerprint {
        metadata.insert(
            "tool_result_fingerprint".to_string(),
            serde_json::json!(fingerprint),
        );
    }
    (!metadata.is_empty()).then_some(metadata)
}

fn parse_structured_tool_result_text(text: &str) -> serde_json::Value {
    let trimmed = text.trim_start();
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        return serde_json::Value::String(text.to_string());
    }

    match serde_json::from_str(text) {
        Ok(value @ (serde_json::Value::Object(_) | serde_json::Value::Array(_))) => value,
        _ => serde_json::Value::String(text.to_string()),
    }
}

#[cfg(test)]
mod tool_completed_replay_tests {
    use super::*;

    #[test]
    fn tool_completed_replay_preserves_json_object_shape() {
        let data = ToolCompletedData::success(
            "call_read".to_string(),
            "read_file".to_string(),
            vec![ContentPart::text(
                serde_json::json!({
                    "path": "/workspace/src/lib.rs",
                    "content": "1|fn main() {}"
                })
                .to_string(),
            )],
            Some(1),
        );

        let message = tool_completed_to_message(data);
        let result = message
            .tool_result_content()
            .and_then(|content| content.result.as_ref())
            .expect("tool result should be present");

        assert_eq!(result["path"], "/workspace/src/lib.rs");
        assert_eq!(result["content"], "1|fn main() {}");
    }

    #[test]
    fn tool_completed_replay_keeps_scalar_json_as_text() {
        let data = ToolCompletedData::success(
            "call_scalar".to_string(),
            "custom_tool".to_string(),
            vec![ContentPart::text("123")],
            Some(1),
        );

        let message = tool_completed_to_message(data);
        let result = message
            .tool_result_content()
            .and_then(|content| content.result.as_ref())
            .expect("tool result should be present");

        assert_eq!(result, &serde_json::Value::String("123".to_string()));
    }

    #[test]
    fn tool_completed_replay_preserves_fingerprints_as_metadata() {
        let data = ToolCompletedData::success(
            "call_read".to_string(),
            "read_file".to_string(),
            vec![ContentPart::text("{}")],
            Some(1),
        )
        .with_fingerprints("sha256:call".to_string(), "sha256:result".to_string());

        let message = tool_completed_to_message(data);
        let metadata = message.metadata.expect("metadata should be present");

        assert_eq!(metadata["tool_name"], "read_file");
        assert_eq!(metadata["tool_call_fingerprint"], "sha256:call");
        assert_eq!(metadata["tool_result_fingerprint"], "sha256:result");
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
