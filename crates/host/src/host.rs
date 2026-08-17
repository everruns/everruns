// Shared host orchestration for embedded and durable execution hosts.
// Decision: everruns-host owns worker-facing turn phase execution so
// durable/server-backed hosts reuse the same input/reason/act wiring without
// depending on the application facade.

use async_trait::async_trait;
use everruns_core::capabilities::{
    Capability, SystemPromptContext, collect_capabilities_with_configs,
};
use everruns_core::events::{
    EventContext, EventRequest, OutputMessageCompletedData, SessionActivatedData, SessionIdledData,
    TurnCompletedData, TurnFailedData, TurnStartedData,
};
use everruns_core::message::{ContentPart, Message};
use everruns_core::message_retriever::MessageRetriever;
use everruns_core::session::SessionExecutionState;
use everruns_core::{
    CapabilityRegistry, CapabilityStatus, DependencyBlocker, EgressService,
    ResolvedExecutionSnapshot, TokenUsage, ToolRegistry, UtilityLlmService,
    org_public_id_from_internal, resolve_runtime_capabilities,
};
use everruns_core::{
    connection_services::ProviderCredentialStore, connection_services::UserConnectionResolver,
    delegation_services::SessionCreationAuthority, event_emitter::EventEmitter,
    execution_loading::AgentStore, execution_loading::HarnessStore,
    execution_loading::SessionStore, image_services::ImageArtifactStore,
    image_services::ImageResolver, provider_resolution::ProviderStore,
    session_files::SessionFileSystem, session_services::LeasedResourceStore,
    session_services::SessionResourceRegistry, session_services::SessionScheduleStore,
    session_services::SessionStorageStore, tool_context::ToolContextServices,
    tool_execution::BudgetChecker, tool_execution::PaymentAuthority,
};
use everruns_engine::{
    ActAtom, ActInput, ActResult, InputAtom, InputAtomInput, InputAtomResult, ReasonAtom,
    ReasonInput, ReasonResult,
};
use everruns_provider::driver_registry::DriverRegistry;
use everruns_provider::tool_types::ToolDefinition;
use everruns_provider::typed_id::{AgentId, HarnessId, MessageId, SessionId, TurnId};
use everruns_provider::user_facing_error::{ErrorDisclosure, UserFacingError};
use everruns_session_services::{SessionMutator, SessionMutatorExt};
use std::sync::Arc;
use tracing::warn;

/// Turn-local view that preserves a capability's message filtering while
/// suppressing every model-visible contribution.
struct MessageFilterOnlyCapability(Arc<dyn Capability>);

impl Capability for MessageFilterOnlyCapability {
    fn id(&self) -> &str {
        self.0.id()
    }

    fn aliases(&self) -> Vec<&'static str> {
        self.0.aliases()
    }

    fn name(&self) -> &str {
        self.0.name()
    }

    fn description(&self) -> &str {
        self.0.description()
    }

    fn status(&self) -> CapabilityStatus {
        self.0.status()
    }

    fn message_filter_provider(
        &self,
    ) -> Option<Arc<dyn everruns_core::message_filter::MessageFilterProvider>> {
        self.0.message_filter_provider()
    }

    fn message_filter_config(
        &self,
        config: &serde_json::Value,
        compaction_enabled: bool,
    ) -> serde_json::Value {
        self.0.message_filter_config(config, compaction_enabled)
    }
}

#[cfg(feature = "bashkit")]
fn bash_hook_dispatcher(
    file_store: Arc<dyn SessionFileSystem>,
) -> Arc<dyn everruns_core::hook_executor::BashHookDispatcher> {
    Arc::new(everruns_integrations_bashkit::BashkitShellHookDispatcher::new(file_store))
}

#[cfg(not(feature = "bashkit"))]
fn bash_hook_dispatcher(
    _file_store: Arc<dyn SessionFileSystem>,
) -> Arc<dyn everruns_core::hook_executor::BashHookDispatcher> {
    struct DisabledDispatcher;

    #[async_trait]
    impl everruns_core::hook_executor::BashHookDispatcher for DisabledDispatcher {
        async fn dispatch(
            &self,
            _payload: &everruns_core::hook_executor::HookPayload,
            _command: &str,
            _extra_env: &std::collections::BTreeMap<String, String>,
            _opts: &everruns_core::hook_executor::ExecutorOpts,
        ) -> std::result::Result<everruns_core::hook_executor::BashExecOutput, String> {
            Err("bash hooks require the everruns-host `bashkit` feature".to_string())
        }
    }

    Arc::new(DisabledDispatcher)
}

/// Resolved inputs loaded in one batched call for runtime host execution.
///
/// This is the narrow load/resolve contract (EVE-872): hosts return the
/// canonical [`ResolvedExecutionSnapshot`] plus the turn's message and
/// MCP tool inputs. Stored Agent/Harness/Session aggregates never cross this
/// boundary — platform projection happens inside the adapter.
#[derive(Debug, Clone)]
pub struct ResolvedTurnInputs {
    /// Canonical resolved execution value for the session.
    pub snapshot: ResolvedExecutionSnapshot,
    /// Conversation messages available to the turn.
    pub messages: Vec<Message>,
    /// MCP tool definitions discovered for the session's scoped servers.
    pub mcp_tool_definitions: Vec<ToolDefinition>,
}

/// Public adapter contract for server-backed or durable runtime hosts.
///
/// `everruns-host` owns shared orchestration for both embedded and durable
/// execution. That includes phase execution (`input -> reason -> act`),
/// lifecycle emission, and the generic turn-strategy decisions used by durable
/// or custom hosts.
///
/// Host crates implement this trait to provide persistence, session-lifecycle
/// plumbing, event delivery, and their own orchestration backend. The durable
/// engine itself remains outside this crate.
#[async_trait]
pub trait RuntimeHostAdapter: Send + Sync + Clone + 'static {
    /// Session status mutation is a host effect, separate from execution
    /// inputs: it exposes no stored Session record to the engine.
    async fn set_session_status(
        &self,
        org_id: i64,
        session_id: SessionId,
        status: SessionExecutionState,
    ) -> everruns_provider::error::Result<()>;

    /// Load and resolve the turn's execution inputs.
    ///
    /// Implementations project their stored records into the canonical
    /// [`ResolvedExecutionSnapshot`] (via
    /// [`ResolvedExecutionSnapshot::project`]) so missing, mismatched, or
    /// inactive records fail here — during platform projection — before host
    /// execution.
    async fn load_resolved_turn(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> everruns_provider::error::Result<ResolvedTurnInputs>;

    fn capability_registry(&self) -> CapabilityRegistry;

    fn driver_registry(&self) -> DriverRegistry;

    fn harness_store(&self, org_id: i64) -> Arc<dyn HarnessStore>;

    fn agent_store(&self, org_id: i64) -> Arc<dyn AgentStore>;

    fn session_store(&self, org_id: i64) -> Arc<dyn SessionStore>;

    fn session_mutator(&self, org_id: i64) -> Arc<dyn SessionMutator>;

    fn provider_store(&self, org_id: i64) -> Arc<dyn ProviderStore>;

    fn message_store(&self) -> Arc<dyn MessageRetriever>;

    fn compaction_checkpoint_store(
        &self,
    ) -> Option<Arc<dyn everruns_core::CompactionCheckpointStore>> {
        None
    }

    fn event_emitter(&self) -> Arc<dyn EventEmitter>;

    fn file_store(&self) -> Arc<dyn SessionFileSystem>;

    fn image_resolver(&self, _org_id: i64) -> Option<Arc<dyn ImageResolver>> {
        None
    }

    fn image_artifact_store(&self, _org_id: i64) -> Option<Arc<dyn ImageArtifactStore>> {
        None
    }

    fn provider_credential_store(&self, _org_id: i64) -> Option<Arc<dyn ProviderCredentialStore>> {
        None
    }

    fn utility_llm_service(&self) -> Option<Arc<dyn UtilityLlmService>> {
        None
    }

    fn egress_service(&self) -> Option<Arc<dyn EgressService>> {
        None
    }

    fn storage_store(&self) -> Option<Arc<dyn SessionStorageStore>> {
        None
    }

    fn connection_resolver(&self) -> Option<Arc<dyn UserConnectionResolver>> {
        None
    }

    /// Type-erased tool services supplied by layers above the host.
    fn tool_context_extensions(
        &self,
        _org_id: i64,
        _session_id: SessionId,
    ) -> everruns_core::tool_context::ToolContextExtensions {
        Default::default()
    }

    /// Neutral subagent delegation supplied by layers above the host.
    fn subagent_delegate(
        &self,
        _org_id: i64,
        _session_id: SessionId,
    ) -> Option<Arc<dyn everruns_core::subagent_delegation::SubagentSessionDelegate>> {
        None
    }

    /// Turn-dependent tools supplied by layers above the host.
    fn tool_augmentor(&self) -> Option<Arc<dyn crate::HostToolAugmentor>> {
        None
    }

    fn leased_resource_store(&self) -> Option<Arc<dyn LeasedResourceStore>> {
        None
    }

    fn session_resource_registry(&self) -> Option<Arc<dyn SessionResourceRegistry>> {
        None
    }

    fn session_task_registry(
        &self,
    ) -> Option<Arc<dyn everruns_core::session_task::SessionTaskRegistry>> {
        None
    }

    fn schedule_store(&self, _org_id: i64) -> Option<Arc<dyn SessionScheduleStore>> {
        None
    }

    fn budget_checker(
        &self,
        _org_id: i64,
        _agent_id: Option<AgentId>,
    ) -> Option<Arc<dyn BudgetChecker>> {
        None
    }

    fn payment_authority(
        &self,
        _org_id: i64,
        _agent_id: Option<AgentId>,
    ) -> Option<Arc<dyn PaymentAuthority>> {
        None
    }

    fn session_creation_authority(
        &self,
        _org_id: i64,
        _session_id: SessionId,
    ) -> Option<Arc<dyn SessionCreationAuthority>> {
        None
    }

    /// Per-org outbound tool-call rate limiter (TM-TOOL-009).
    /// Default: `None` (no rate limiting — suitable for in-process / test environments).
    fn outbound_tool_rate_limiter(
        &self,
        _org_id: i64,
    ) -> Option<Arc<dyn everruns_core::tool_execution::OutboundToolRateLimiter>> {
        None
    }

    /// Per-turn durable tool result store for act-activity idempotency (EVE-530).
    /// Default: `None` (no durable claim/settle — every execution runs tools fresh).
    fn durable_tool_result_store(
        &self,
    ) -> Option<Arc<dyn everruns_core::durability::DurableToolResultStore>> {
        None
    }

    /// Durable subagent spawn handle store for reattach on reclaim (EVE-535).
    /// Default: `None` (no spawn dedup — dev/test mode or hosts without durable execution).
    fn subagent_spawn_store(
        &self,
    ) -> Option<Arc<dyn everruns_core::delegation_services::SubagentSpawnStore>> {
        None
    }

    /// Stream-liveness heartbeater for the Reason activity (EVE-531).
    /// Default: `None` (no heartbeats sent — durable workers supply one).
    fn stream_heartbeater(&self) -> Option<Arc<dyn everruns_core::durability::StreamHeartbeater>> {
        None
    }

    /// Partial-stream store for ContinuePartial recovery (EVE-532).
    /// Default: `None` (no recovery; in-memory and dev hosts use this default).
    fn partial_stream_store(
        &self,
    ) -> Option<Arc<dyn everruns_core::durability::PartialStreamStore>> {
        None
    }

    /// Live, turn-scoped reasoning-effort handle for the given session (EVE-595).
    ///
    /// When a host returns a handle, the Reason activity re-reads it on every
    /// LLM step and the Act activity hands the same instance to each tool's
    /// `ToolContext`. A tool can then change effort mid-turn and have subsequent
    /// LLM steps in the same turn observe it. Hosts MUST return the *same*
    /// handle instance for a session across reason/act activities of one turn.
    /// Default: `None` (effort is resolved solely from message controls).
    fn reasoning_effort_handle(
        &self,
        _session_id: SessionId,
    ) -> Option<everruns_core::tool_context::ReasoningEffortHandle> {
        None
    }

    /// Provider stall timeout for the Reason activity (EVE-531).
    /// Default: `None` (use built-in 120s default).
    fn provider_stall_timeout(&self) -> Option<std::time::Duration> {
        None
    }

    /// Bounded automatic-recovery policy for provider failures.
    /// Default: `None` (use the provider policy defaults).
    fn provider_retry_config(&self) -> Option<everruns_provider::llm_retry::LlmRetryConfig> {
        None
    }

    /// MCP executor routing `mcp_*` tool calls for this session, if the host
    /// configures MCP (knowledge/integrations/runtime-mcp.md D4). Default: `None`, so hosts
    /// without scoped MCP servers keep the plain tool registry unchanged.
    async fn mcp_executor(
        &self,
        _org_id: i64,
        _session_id: SessionId,
    ) -> Option<Arc<dyn everruns_core::McpToolInvoker>> {
        None
    }
}

struct RuntimeExecutionCapabilities {
    tool_registry: ToolRegistry,
    post_tool_hooks: Vec<Arc<dyn everruns_core::tool_hooks::PostToolExecHook>>,
    pre_tool_hooks: Vec<Arc<dyn everruns_core::tool_hooks::PreToolUseHook>>,
    tool_call_hooks: Vec<Arc<dyn everruns_core::ToolCallHook>>,
    subagent_nesting_policy: everruns_core::delegation_services::SubagentNestingPolicy,
}

fn subagent_nesting_policy_from_configs(
    resolved_capability_configs: &[everruns_capability::CapabilityRef],
) -> everruns_core::delegation_services::SubagentNestingPolicy {
    let subagents_config = resolved_capability_configs
        .iter()
        .find(|config| config.capability_id() == "subagents");

    let configured_depth = subagents_config
        .and_then(|config| {
            config
                .config_value()
                .get("max_subagent_depth")
                .or_else(|| config.config_value().get("max_depth"))
        })
        .and_then(|value| value.as_u64())
        .and_then(|value| u32::try_from(value).ok());
    let configured_max_active = subagents_config
        .and_then(|config| {
            config
                .config_value()
                .get("max_active_descendant_tasks")
                .or_else(|| config.config_value().get("max_concurrent_descendant_tasks"))
        })
        .and_then(|value| value.as_u64())
        .and_then(|value| u32::try_from(value).ok());
    let configured_max_total = subagents_config
        .and_then(|config| config.config_value().get("max_total_descendant_tasks"))
        .and_then(|value| value.as_u64())
        .and_then(|value| u32::try_from(value).ok());
    let configured_max_active_detached = subagents_config
        .and_then(|config| config.config_value().get("max_active_detached_tasks"))
        .and_then(|value| value.as_u64())
        .and_then(|value| u32::try_from(value).ok());
    let configured_max_total_detached = subagents_config
        .and_then(|config| config.config_value().get("max_total_detached_tasks"))
        .and_then(|value| value.as_u64())
        .and_then(|value| u32::try_from(value).ok());

    everruns_core::delegation_services::SubagentNestingPolicy::default()
        .with_agent_override(configured_depth)
        .with_agent_task_caps_override(configured_max_active, configured_max_total)
        .with_agent_detached_task_caps_override(
            configured_max_active_detached,
            configured_max_total_detached,
        )
}

/// Collect and finalize user-hook specs for a session from its resolved
/// capability configs, plus the shared bash dispatcher used to run them.
///
/// This is the single place hook specs are gathered so every firing point —
/// the act path (`load_execution_capabilities`) and the lifecycle firing
/// points (`execute_reason_activity` for `user_prompt_submit`, turn completion
/// for `turn_end`, and the server session paths) — applies identical
/// `finalize_hook_specs` semantics: `{capability_id}:` namespace stamping,
/// stable default ids, and `disabled_contributions` muting (TM-HOOK-004).
fn finalize_specs_from_configs(
    resolved_capability_configs: &[everruns_capability::CapabilityRef],
    capability_registry: &CapabilityRegistry,
    tool_augmentor: Option<&dyn crate::HostToolAugmentor>,
) -> Vec<everruns_core::user_hook_types::UserHookSpec> {
    let mut hook_contributions: Vec<(String, Vec<everruns_core::user_hook_types::UserHookSpec>)> =
        Vec::new();
    let mut disabled_contributions: Vec<String> = Vec::new();
    for config in resolved_capability_configs {
        let Some(capability) = capability_registry.get(config.capability_id()) else {
            continue;
        };
        let specs = capability.user_hooks_with_config(config.config_value());
        if !specs.is_empty() {
            hook_contributions.push((config.capability_id().to_string(), specs));
        }
        if let Some(augmentor) = tool_augmentor {
            disabled_contributions.extend(
                augmentor
                    .disabled_hook_contributions(config.capability_id(), config.config_value()),
            );
        }
    }
    everruns_core::hook_adapter::finalize_hook_specs(hook_contributions, &disabled_contributions)
}

/// Resolve a session's capability configs and collect finalized hook specs.
/// Used by the lifecycle firing points, which need specs outside the act path.
/// Returns `(specs, dispatcher)`; `specs` is empty when the session has no
/// hook-contributing capabilities.
async fn collect_lifecycle_hook_specs<A: RuntimeHostAdapter>(
    adapter: &A,
    org_id: i64,
    session_id: SessionId,
    harness_id: HarnessId,
    agent_id: Option<AgentId>,
) -> everruns_provider::error::Result<(
    Vec<everruns_core::user_hook_types::UserHookSpec>,
    Arc<dyn everruns_core::hook_executor::BashHookDispatcher>,
)> {
    let capability_registry = adapter.capability_registry();
    let harness = adapter
        .harness_store(org_id)
        .get_harness(harness_id)
        .await?
        .ok_or_else(|| everruns_provider::error::AgentLoopError::harness_not_found(harness_id))?;
    let session = adapter
        .session_store(org_id)
        .get_session(session_id)
        .await?
        .ok_or_else(|| everruns_provider::error::AgentLoopError::session_not_found(session_id))?;
    let agent = match agent_id {
        Some(agent_id) => adapter.agent_store(org_id).get_agent(agent_id).await?,
        None => None,
    };
    let resolved =
        resolve_runtime_capabilities(&harness, agent.as_ref(), &session, &capability_registry);
    let tool_augmentor = adapter.tool_augmentor();
    let specs = finalize_specs_from_configs(
        &resolved.resolved_capability_configs,
        &capability_registry,
        tool_augmentor.as_deref(),
    );
    let dispatcher = bash_hook_dispatcher(adapter.file_store());
    Ok((specs, dispatcher))
}

async fn load_execution_capabilities<A: RuntimeHostAdapter>(
    adapter: &A,
    org_id: i64,
    session_id: SessionId,
    harness_id: HarnessId,
    agent_id: Option<AgentId>,
    locale: Option<String>,
    blueprint_id: Option<&str>,
) -> everruns_provider::error::Result<RuntimeExecutionCapabilities> {
    let capability_registry = adapter.capability_registry();
    if let Some(blueprint_id) = blueprint_id {
        let mut registry = ToolRegistry::with_defaults();
        #[cfg(feature = "builtins")]
        everruns_builtins::register_default_tools(&mut registry);
        let blueprint = capability_registry.blueprint(blueprint_id).ok_or_else(|| {
            everruns_provider::error::AgentLoopError::config(format!(
                "Blueprint \"{blueprint_id}\" not found in registry"
            ))
        })?;
        for tool in blueprint.tools {
            registry.register_boxed(tool);
        }
        return Ok(RuntimeExecutionCapabilities {
            tool_registry: registry,
            post_tool_hooks: Vec::new(),
            pre_tool_hooks: Vec::new(),
            tool_call_hooks: Vec::new(),
            subagent_nesting_policy:
                everruns_core::delegation_services::SubagentNestingPolicy::default(),
        });
    }

    let harness = adapter
        .harness_store(org_id)
        .get_harness(harness_id)
        .await?
        .ok_or_else(|| everruns_provider::error::AgentLoopError::harness_not_found(harness_id))?;

    let session = adapter
        .session_store(org_id)
        .get_session(session_id)
        .await?
        .ok_or_else(|| everruns_provider::error::AgentLoopError::session_not_found(session_id))?;

    let agent_store = adapter.agent_store(org_id);
    let agent =
        match agent_id {
            Some(agent_id) => Some(agent_store.get_agent(agent_id).await?.ok_or_else(|| {
                everruns_provider::error::AgentLoopError::agent_not_found(agent_id)
            })?),
            None => None,
        };

    let resolved =
        resolve_runtime_capabilities(&harness, agent.as_ref(), &session, &capability_registry);
    // Executor (act) path: this builds the worker-side tool registry, not the
    // model-visible tool list. The model is left unset, so a model-adaptive
    // capability like `auto_tool_search` resolves to its provider-agnostic
    // client-side mechanism here. That registers the `tool_search` tool in the
    // executor, which is a harmless superset: on native models the reason path
    // never shows that tool to the model, so it is simply never called.
    let prompt_ctx = SystemPromptContext {
        session_id,
        locale: locale.or(session.locale.clone()),
        // Pin system-prompt file reads to the session's workspace (the default
        // 1:1 case is a transparent pass-through), then resolve through the
        // mount resolver (EVE-660): `/workspace` is a mount + cwd.
        // `scoped_prompt_file_store` wraps with `wrap_if_needed` so a local
        // embedder's backend-native display policy survives here too (it must
        // match the reason path — see its doc); server stores stay on `/workspace`.
        file_store: Some(everruns_core::scoped_prompt_file_store(
            adapter.file_store(),
            session.workspace_id,
        )),
        model: None,
    };
    let collected = collect_capabilities_with_configs(
        &resolved.resolved_capability_configs,
        &capability_registry,
        &prompt_ctx,
    )
    .await;

    let mut registry = ToolRegistry::with_defaults();
    #[cfg(feature = "builtins")]
    everruns_builtins::register_default_tools(&mut registry);
    for tool in collected.tools {
        registry.register_boxed(tool);
    }

    // Only `Available` capabilities contribute hooks, matching
    // `collect_capabilities_with_configs` (which skips non-available
    // capabilities). This keeps a `ComingSoon`/unavailable capability from
    // affecting execution via any of its hook seams.
    let mut post_tool_hooks: Vec<Arc<dyn everruns_core::tool_hooks::PostToolExecHook>> = resolved
        .resolved_capability_configs
        .iter()
        .flat_map(|config| {
            capability_registry
                .get(config.capability_id())
                .filter(|capability| capability.status() == CapabilityStatus::Available)
                .map(|capability| {
                    capability.post_tool_exec_hooks_with_config(config.config_value())
                })
                .unwrap_or_default()
        })
        .collect();
    // Tool-output guardrails must inspect the original result before other
    // capability hooks can persist or compact it into secondary surfaces.
    post_tool_hooks.sort_by_key(|hook| hook.priority());

    // User-hook contributions (see `knowledge/runtime-resources/user-hooks.md`). `finalize_specs_from_configs`
    // gathers specs across every resolved capability — both the user-facing
    // `user_hooks` capability and any capability that bundles hooks — and applies
    // `finalize_hook_specs` (namespace stamping, stable ids, `disabled_contributions`
    // muting; TM-HOOK-004). The same helper backs the lifecycle firing points so
    // every event finalizes specs identically.
    let tool_augmentor = adapter.tool_augmentor();
    let user_hook_specs = finalize_specs_from_configs(
        &resolved.resolved_capability_configs,
        &capability_registry,
        tool_augmentor.as_deref(),
    );
    // Persisted messages remain the immutable audit record, so they can contain
    // text removed by a provider-bound user_prompt_submit hook. Until there is a
    // durable provider-visible history view, fail closed rather than let
    // query_history bypass that enforcement boundary.
    if user_hook_specs
        .iter()
        .any(|spec| spec.event == everruns_core::user_hook_types::HookEvent::UserPromptSubmit)
    {
        registry.unregister("query_history");
    }
    // Capability-contributed pre-tool hooks run first (e.g. approval gating),
    // then user-hook (`PreToolUse`) specs. The first hook to block wins.
    let mut pre_tool_hooks: Vec<Arc<dyn everruns_core::tool_hooks::PreToolUseHook>> = resolved
        .resolved_capability_configs
        .iter()
        .flat_map(|config| {
            capability_registry
                .get(config.capability_id())
                .filter(|capability| capability.status() == CapabilityStatus::Available)
                .map(|capability| capability.pre_tool_use_hooks_with_config(config.config_value()))
                .unwrap_or_default()
        })
        .collect();
    if !user_hook_specs.is_empty() {
        let dispatcher = bash_hook_dispatcher(adapter.file_store());
        post_tool_hooks.extend(everruns_core::hook_adapter::build_post_tool_use_hooks(
            &user_hook_specs,
            dispatcher.clone(),
        ));
        pre_tool_hooks.extend(everruns_core::hook_adapter::build_pre_tool_use_hooks(
            &user_hook_specs,
            dispatcher,
        ));
    }

    // Use the hook list assembled by `collect_capabilities_with_configs` as the
    // single source of truth. It already contains every explicit capability
    // `tool_call_hooks()` followed by the generated `CapabilityNarrationHook`
    // adapters — one per collected capability plus any auto-activated
    // cross-cutting capability such as `background_execution`. Re-deriving only
    // the explicit subset here dropped capability-owned narration, so tools fell
    // back to generic `Ran {display_name}` lines (EVE-601). Explicit hooks stay
    // first in this list, so model-authored narration (`human_intent`) keeps its
    // precedence over default `Tool::narrate()`, and only available capabilities
    // contributed because collection skips non-available ones.
    let tool_call_hooks = collected.tool_call_hooks;

    Ok(RuntimeExecutionCapabilities {
        tool_registry: registry,
        post_tool_hooks,
        pre_tool_hooks,
        tool_call_hooks,
        subagent_nesting_policy: subagent_nesting_policy_from_configs(
            &resolved.resolved_capability_configs,
        ),
    })
}

fn runtime_tool_context_services<A: RuntimeHostAdapter>(
    adapter: &A,
    org_id: i64,
    session_id: SessionId,
    agent_id: Option<AgentId>,
    tool_registry: Option<Arc<ToolRegistry>>,
    mcp_invoker: Option<Arc<dyn everruns_core::McpToolInvoker>>,
    subagent_nesting_policy: everruns_core::delegation_services::SubagentNestingPolicy,
) -> ToolContextServices {
    let extensions = {
        let mut extensions = adapter.tool_context_extensions(org_id, session_id);
        extensions.insert(Arc::new(SessionMutatorExt(adapter.session_mutator(org_id))));
        extensions
    };
    ToolContextServices {
        file_store: Some(adapter.file_store()),
        storage_store: adapter.storage_store(),
        image_store: adapter.image_artifact_store(org_id),
        provider_credential_store: adapter.provider_credential_store(org_id),
        utility_llm_service: adapter.utility_llm_service(),
        mcp_invoker,
        egress_service: adapter.egress_service(),
        message_retriever: Some(adapter.message_store()),
        session_store: Some(adapter.session_store(org_id)),
        agent_store: Some(adapter.agent_store(org_id)),
        connection_resolver: adapter.connection_resolver(),
        schedule_store: adapter.schedule_store(org_id),
        subagent_delegate: adapter.subagent_delegate(org_id, session_id),
        extensions,
        leased_resource_store: adapter.leased_resource_store(),
        session_resource_registry: adapter.session_resource_registry(),
        session_task_registry: adapter.session_task_registry(),
        event_emitter: Some(adapter.event_emitter()),
        capability_registry: Some(adapter.capability_registry()),
        tool_registry,
        org_id: Some(
            org_public_id_from_internal(org_id)
                .parse()
                .expect("internal org id converts to valid public org id"),
        ),
        network_access: None,
        budget_checker: adapter.budget_checker(org_id, agent_id),
        payment_authority: adapter.payment_authority(org_id, agent_id),
        session_creation_authority: adapter.session_creation_authority(org_id, session_id),
        subagent_spawn_store: adapter.subagent_spawn_store(),
        subagent_nesting_policy,
        reasoning_effort_handle: adapter.reasoning_effort_handle(session_id),
    }
}

/// Shared lifecycle helper for runtime-backed hosts.
pub struct RuntimeSessionLifecycle<A: RuntimeHostAdapter> {
    adapter: A,
    org_id: i64,
    session_id: SessionId,
}

impl<A: RuntimeHostAdapter> RuntimeSessionLifecycle<A> {
    pub fn new(adapter: A, org_id: i64, session_id: SessionId) -> Self {
        Self {
            adapter,
            org_id,
            session_id,
        }
    }

    async fn set_session_status(
        &self,
        status: SessionExecutionState,
        _action: &'static str,
    ) -> everruns_provider::error::Result<()> {
        self.adapter
            .set_session_status(self.org_id, self.session_id, status)
            .await
    }

    async fn emit_event(&self, request: EventRequest) -> everruns_provider::error::Result<()> {
        self.adapter.event_emitter().emit(request).await.map(|_| ())
    }

    pub async fn turn_started(
        &self,
        turn_id: TurnId,
        input_message_id: MessageId,
    ) -> everruns_provider::error::Result<()> {
        let input_content = self
            .adapter
            .message_store()
            .get(self.session_id, input_message_id)
            .await
            .ok()
            .flatten()
            .map(|message| message.content_to_llm_string());

        self.set_session_status(SessionExecutionState::Active, "turn_started")
            .await?;

        self.emit_event(EventRequest::new(
            self.session_id,
            EventContext::turn(turn_id, input_message_id),
            SessionActivatedData {
                turn_id,
                input_message_id,
            },
        ))
        .await?;

        self.emit_event(EventRequest::new(
            self.session_id,
            EventContext::turn(turn_id, input_message_id),
            TurnStartedData {
                turn_id,
                input_message_id,
                input_content,
            },
        ))
        .await?;
        Ok(())
    }

    pub async fn emit_turn_completed(
        &self,
        input_message_id: MessageId,
        data: TurnCompletedData,
    ) -> everruns_provider::error::Result<()> {
        let turn_id = data.turn_id;
        self.emit_event(EventRequest::new(
            self.session_id,
            EventContext::turn(turn_id, input_message_id),
            data,
        ))
        .await
    }

    pub async fn emit_session_idled(
        &self,
        turn_id: TurnId,
        input_message_id: MessageId,
        iterations: Option<u32>,
        usage: Option<TokenUsage>,
    ) -> everruns_provider::error::Result<()> {
        self.set_session_status(SessionExecutionState::Idle, "emit_session_idled")
            .await?;

        self.emit_event(EventRequest::new(
            self.session_id,
            EventContext::turn(turn_id, input_message_id),
            SessionIdledData {
                turn_id,
                iterations,
                usage,
            },
        ))
        .await
    }

    pub async fn turn_completed(
        &self,
        turn_id: TurnId,
        input_message_id: MessageId,
        iterations: u32,
        usage: Option<TokenUsage>,
        input_content: Option<String>,
    ) -> everruns_provider::error::Result<()> {
        self.emit_turn_completed(
            input_message_id,
            TurnCompletedData {
                turn_id,
                iterations,
                duration_ms: None,
                usage: usage.clone(),
                input_content,
                final_message_id: None,
                final_answer_preview: None,
                time_to_first_token_ms: None,
                tool_call_count: None,
                llm_call_count: None,
                status: Some("completed".to_string()),
            },
        )
        .await?;
        self.emit_session_idled(turn_id, input_message_id, Some(iterations), usage)
            .await
    }

    /// Turn was deliberately sealed (EVE-534): emit `turn.sealed` + a
    /// user-facing message + `session.idled`, and idle the session.
    ///
    /// Distinct from `turn_completed` (success) and `turn_failed` (error). The
    /// session returns to `idle` so the UI unblocks; the Sealed state is
    /// observable via the `turn.sealed` event and its `reason`.
    pub async fn turn_sealed(
        &self,
        turn_id: TurnId,
        input_message_id: MessageId,
        reason: &str,
        iterations: u32,
        usage: Option<TokenUsage>,
    ) -> everruns_provider::error::Result<()> {
        let context = EventContext::turn(turn_id, input_message_id);

        self.emit_event(EventRequest::new(
            self.session_id,
            context.clone(),
            everruns_core::events::TurnSealedData {
                turn_id,
                reason: reason.to_string(),
                detail: None,
                iterations: Some(iterations),
                usage: usage.clone(),
            },
        ))
        .await?;

        self.emit_session_idled(turn_id, input_message_id, Some(iterations), usage)
            .await
    }

    /// Fire `turn_end` lifecycle hooks (advisory). Collects the session's hook
    /// specs and runs every `turn_end` hook; failures are logged, never fatal.
    /// `harness_id`/`agent_id` are required to resolve the capability chain.
    pub async fn fire_turn_end_hooks(
        &self,
        harness_id: HarnessId,
        agent_id: Option<AgentId>,
        turn_id: TurnId,
        success: bool,
    ) {
        let (specs, dispatcher) = match collect_lifecycle_hook_specs(
            &self.adapter,
            self.org_id,
            self.session_id,
            harness_id,
            agent_id,
        )
        .await
        {
            Ok(pair) => pair,
            Err(error) => {
                warn!(
                    session_id = %self.session_id,
                    %error,
                    "failed to collect turn_end hook specs; skipping"
                );
                return;
            }
        };
        let hooks = everruns_core::lifecycle_hooks::build_turn_lifecycle_hooks(
            &specs,
            everruns_core::user_hook_types::HookEvent::TurnEnd,
            dispatcher,
        );
        if hooks.is_empty() {
            return;
        }
        let ctx = everruns_core::lifecycle_hooks::TurnHookContext {
            session_id: self.session_id,
            turn_id: Some(turn_id),
            org_id: org_public_id_from_internal(self.org_id).parse().ok(),
            agent_id: agent_id.map(|a| a.to_string()),
        };
        everruns_core::lifecycle_hooks::run_turn_end_hooks(
            &hooks,
            &ctx,
            serde_json::json!({ "success": success }),
        )
        .await;
    }

    /// Abort a turn because a `user_prompt_submit` hook returned `Block`.
    /// Reuses the dependency-blocked failure shape: emit a user-facing message
    /// carrying the hook's `user_message` (or `reason`), then mark the turn
    /// failed and idle the session.
    pub async fn user_prompt_blocked(
        &self,
        turn_id: TurnId,
        input_message_id: MessageId,
        reason: &str,
        user_message: Option<&str>,
    ) -> everruns_provider::error::Result<()> {
        let user_error =
            UserFacingError::new(everruns_provider::user_facing_error::codes::BLOCKED_BY_HOOK);
        let shown = user_message.unwrap_or(reason);
        let mut error_message = Message::assistant(shown);
        let mut metadata = std::collections::HashMap::new();
        user_error.apply_to_message_metadata(&mut metadata);
        error_message.metadata = Some(metadata);

        self.emit_event(EventRequest::new(
            self.session_id,
            EventContext::turn(turn_id, input_message_id),
            OutputMessageCompletedData::new(error_message).with_user_facing_error(&user_error),
        ))
        .await?;

        self.turn_failed(turn_id, input_message_id, reason, Some(&user_error))
            .await
    }

    pub async fn turn_failed(
        &self,
        turn_id: TurnId,
        input_message_id: MessageId,
        error: &str,
        user_error: Option<&UserFacingError>,
    ) -> everruns_provider::error::Result<()> {
        self.turn_failed_with_disclosure(turn_id, input_message_id, error, user_error, None)
            .await
    }

    /// `turn_failed` with the applied error-disclosure mode recorded on the
    /// event. `user_error` (and the `error` text shown alongside it) must
    /// already be disclosure-filtered by the caller.
    pub async fn turn_failed_with_disclosure(
        &self,
        turn_id: TurnId,
        input_message_id: MessageId,
        error: &str,
        user_error: Option<&UserFacingError>,
        disclosure: Option<ErrorDisclosure>,
    ) -> everruns_provider::error::Result<()> {
        self.set_session_status(SessionExecutionState::Idle, "turn_failed")
            .await?;

        self.emit_event(EventRequest::new(
            self.session_id,
            EventContext::turn(turn_id, input_message_id),
            {
                let mut data = TurnFailedData {
                    turn_id,
                    error: error.to_string(),
                    error_code: None,
                    error_fields: None,
                    error_disclosure: disclosure.map(|mode| mode.as_str().to_string()),
                };
                if let Some(user_error) = user_error {
                    user_error.apply_to_event_fields(&mut data.error_code, &mut data.error_fields);
                }
                data
            },
        ))
        .await?;

        self.emit_event(EventRequest::new(
            self.session_id,
            EventContext::turn(turn_id, input_message_id),
            SessionIdledData {
                turn_id,
                iterations: None,
                usage: None,
            },
        ))
        .await
    }

    pub async fn waiting_for_tool_results(&self) -> everruns_provider::error::Result<()> {
        self.set_session_status(
            SessionExecutionState::WaitingForToolResults,
            "waiting_for_tool_results",
        )
        .await
    }

    pub async fn dependency_blocked(
        &self,
        turn_id: TurnId,
        input_message_id: MessageId,
        blocker: DependencyBlocker,
    ) -> everruns_provider::error::Result<()> {
        let user_error = UserFacingError::new(blocker.error_code())
            .with_field(
                "dependency",
                match blocker {
                    DependencyBlocker::HarnessArchived | DependencyBlocker::HarnessDeleted => {
                        "harness"
                    }
                    DependencyBlocker::AgentArchived | DependencyBlocker::AgentDeleted => "agent",
                },
            )
            .with_field(
                "state",
                match blocker {
                    DependencyBlocker::HarnessArchived | DependencyBlocker::AgentArchived => {
                        "archived"
                    }
                    DependencyBlocker::HarnessDeleted | DependencyBlocker::AgentDeleted => {
                        "deleted"
                    }
                },
            );
        let mut error_message = Message::assistant(blocker.message());
        let mut metadata = std::collections::HashMap::new();
        user_error.apply_to_message_metadata(&mut metadata);
        error_message.metadata = Some(metadata);

        self.emit_event(EventRequest::new(
            self.session_id,
            EventContext::turn(turn_id, input_message_id),
            OutputMessageCompletedData::new(error_message).with_user_facing_error(&user_error),
        ))
        .await?;

        self.turn_failed(
            turn_id,
            input_message_id,
            blocker.message(),
            Some(&user_error),
        )
        .await
    }
}

pub async fn detect_dependency_blocker<A: RuntimeHostAdapter>(
    adapter: &A,
    org_id: i64,
    harness_id: HarnessId,
    agent_id: Option<AgentId>,
) -> everruns_provider::error::Result<Option<DependencyBlocker>> {
    let harness_store = adapter.harness_store(org_id);
    let agent_store = adapter.agent_store(org_id);
    if let Some(blocker) = harness_store.get_harness_blocker(harness_id).await? {
        return Ok(Some(blocker));
    }
    if let Some(agent_id) = agent_id
        && let Some(blocker) = agent_store.get_agent_blocker(agent_id).await?
    {
        return Ok(Some(blocker));
    }
    Ok(None)
}

pub async fn execute_input_activity<A: RuntimeHostAdapter>(
    adapter: &A,
    org_id: i64,
    input: InputAtomInput,
) -> everruns_provider::error::Result<InputAtomResult> {
    // The live effort override is turn-scoped. Clear any value left by the
    // previous turn before ReasonAtom can prefer it over this turn's message
    // controls.
    if let Some(handle) = adapter.reasoning_effort_handle(input.context.session_id) {
        handle.set(None);
    }

    RuntimeSessionLifecycle::new(adapter.clone(), org_id, input.context.session_id)
        .turn_started(input.context.turn_id, input.context.input_message_id)
        .await?;

    let atom = InputAtom::new(adapter.message_store());
    atom.execute(input).await
}

/// Collect `user_prompt_submit` hooks for this turn and run them against the
/// inbound user message text. Returns `None` when the session has no such
/// hooks (the common case — no overhead beyond the spec collection, which is
/// skipped early). Errors loading specs are logged and treated as "no hooks"
/// so a hook-collection failure never blocks a turn that wasn't asking to be
/// hooked.
struct UserPromptHookResult {
    decision: everruns_core::lifecycle_hooks::UserPromptDecision,
    original_message: String,
}

async fn run_user_prompt_submit_for_turn<A: RuntimeHostAdapter>(
    adapter: &A,
    org_id: i64,
    input: &ReasonInput,
) -> everruns_provider::error::Result<Option<UserPromptHookResult>> {
    let (specs, dispatcher) = match collect_lifecycle_hook_specs(
        adapter,
        org_id,
        input.context.session_id,
        input.harness_id,
        input.agent_id,
    )
    .await
    {
        Ok(pair) => pair,
        Err(error) => {
            warn!(
                session_id = %input.context.session_id,
                %error,
                "failed to collect user_prompt_submit hook specs; continuing without them"
            );
            return Ok(None);
        }
    };
    let hooks = everruns_core::lifecycle_hooks::build_turn_lifecycle_hooks(
        &specs,
        everruns_core::user_hook_types::HookEvent::UserPromptSubmit,
        dispatcher,
    );
    if hooks.is_empty() {
        return Ok(None);
    }

    let message_text = adapter
        .message_store()
        .get(input.context.session_id, input.context.input_message_id)
        .await
        .ok()
        .flatten()
        .map(|m| m.content_to_llm_string())
        .unwrap_or_default();

    let ctx = everruns_core::lifecycle_hooks::TurnHookContext {
        session_id: input.context.session_id,
        turn_id: Some(input.context.turn_id),
        org_id: org_public_id_from_internal(org_id).parse().ok(),
        agent_id: input.agent_id.map(|a| a.to_string()),
    };
    let original_message = message_text.clone();
    let decision =
        everruns_core::lifecycle_hooks::run_user_prompt_submit_hooks(&hooks, &ctx, message_text)
            .await;
    Ok(Some(UserPromptHookResult {
        decision,
        original_message,
    }))
}

pub async fn execute_reason_activity<A: RuntimeHostAdapter>(
    adapter: &A,
    org_id: i64,
    input: ReasonInput,
) -> everruns_provider::error::Result<ReasonResult> {
    let prompt_message_ids = (input.iteration <= 1)
        .then_some(input.context.input_message_id)
        .into_iter()
        .collect();
    execute_reason_activity_with_prompt_messages(adapter, org_id, input, prompt_message_ids).await
}

/// Execute a reason activity while applying `user_prompt_submit` hooks to the
/// supplied messages. Hosts that inject synthetic user messages between reason
/// iterations must include their ids here so they cross the same policy
/// boundary as the turn's original input.
pub async fn execute_reason_activity_with_prompt_messages<A: RuntimeHostAdapter>(
    adapter: &A,
    org_id: i64,
    input: ReasonInput,
    prompt_message_ids: Vec<MessageId>,
) -> everruns_provider::error::Result<ReasonResult> {
    if let Some(blocker) =
        detect_dependency_blocker(adapter, org_id, input.harness_id, input.agent_id).await?
    {
        RuntimeSessionLifecycle::new(adapter.clone(), org_id, input.context.session_id)
            .dependency_blocked(
                input.context.turn_id,
                input.context.input_message_id,
                blocker,
            )
            .await?;
        return Ok(ReasonResult {
            success: false,
            text: blocker.message().to_string(),
            tool_calls: vec![],
            has_tool_calls: false,
            tool_definitions: vec![],
            max_iterations: everruns_core::runtime_agent::default_max_iterations(),
            error: Some("dependency_unavailable".to_string()),
            user_facing_error: None,
            error_disclosure: None,
            usage: None,
            output_message_id: None,
            time_to_first_token_ms: None,
            response_id: None,
            finish_reason: None,
            locale: None,
            network_access: None,
            parallel_tool_calls: None,
        });
    }

    // A `Block` aborts the turn by reusing the same failure path as
    // `dependency_blocked`. Hosts pass the original input on iteration one and
    // any synthetic user messages injected later, ensuring every provider-bound
    // user message crosses this policy boundary.
    let mut user_prompt_message_overrides = Vec::new();
    for message_id in prompt_message_ids {
        let mut hook_input = input.clone();
        hook_input.context.input_message_id = message_id;
        let Some(hook_result) =
            run_user_prompt_submit_for_turn(adapter, org_id, &hook_input).await?
        else {
            continue;
        };
        match hook_result.decision {
            everruns_core::lifecycle_hooks::UserPromptDecision::Block {
                reason,
                user_message,
            } => {
                RuntimeSessionLifecycle::new(adapter.clone(), org_id, input.context.session_id)
                    .user_prompt_blocked(
                        input.context.turn_id,
                        input.context.input_message_id,
                        &reason,
                        user_message.as_deref(),
                    )
                    .await?;
                return Ok(ReasonResult {
                    success: false,
                    text: user_message.unwrap_or_else(|| reason.clone()),
                    tool_calls: vec![],
                    has_tool_calls: false,
                    tool_definitions: vec![],
                    max_iterations: everruns_core::runtime_agent::default_max_iterations(),
                    error: Some("blocked_by_user_prompt_hook".to_string()),
                    user_facing_error: None,
                    error_disclosure: None,
                    usage: None,
                    output_message_id: None,
                    time_to_first_token_ms: None,
                    response_id: None,
                    finish_reason: None,
                    locale: None,
                    network_access: None,
                    parallel_tool_calls: None,
                });
            }
            everruns_core::lifecycle_hooks::UserPromptDecision::Continue { message } => {
                if message != hook_result.original_message {
                    user_prompt_message_overrides.push((message_id, message));
                }
            }
        }
    }

    // Validate the executor-side registry before ReasonAtom exposes its tool
    // definitions to the model. This catches host wiring errors as
    // configuration failures instead of late tool-call failures.
    let validation_session = adapter
        .session_store(org_id)
        .get_session(input.context.session_id)
        .await?
        .ok_or_else(|| {
            everruns_provider::error::AgentLoopError::session_not_found(input.context.session_id)
        })?;
    let validation_capabilities = load_execution_capabilities(
        adapter,
        org_id,
        input.context.session_id,
        input.harness_id,
        input.agent_id,
        validation_session.locale.clone(),
        validation_session.blueprint_id.as_deref(),
    )
    .await?;
    let query_history_allowed = validation_capabilities
        .tool_registry
        .get("query_history")
        .is_some();
    let validation_services = runtime_tool_context_services(
        adapter,
        org_id,
        input.context.session_id,
        input.agent_id,
        Some(Arc::new(validation_capabilities.tool_registry.clone())),
        None,
        validation_capabilities.subagent_nesting_policy,
    );
    validation_capabilities
        .tool_registry
        .validate_context_services(&validation_services)?;

    let mut turn_inputs = adapter
        .load_resolved_turn(org_id, input.context.session_id)
        .await?;
    if let Some(augmentor) = adapter.tool_augmentor() {
        augmentor
            .augment_reason_tools(
                input.context.session_id,
                adapter.session_store(org_id),
                adapter.session_task_registry(),
                &mut turn_inputs.mcp_tool_definitions,
            )
            .await?;
    }

    let reason_capability_registry = {
        let mut registry = adapter.capability_registry();
        if !query_history_allowed {
            // Persisted history may contain raw text removed by earlier prompt
            // hooks. Preserve the owning capability's message filter while
            // suppressing its prompt/tool contributions for this turn. Tool
            // ownership is discovered through the neutral capability contract;
            // host does not depend on a concrete implementation or capability ID.
            let query_history_owner = registry
                .list()
                .into_iter()
                .find(|capability| {
                    capability
                        .tool_definitions()
                        .iter()
                        .any(|tool| tool.name() == "query_history")
                })
                .map(Arc::clone);
            if let Some(capability) = query_history_owner {
                registry.register(MessageFilterOnlyCapability(capability));
            }
        }
        registry
    };
    let context_resolver = crate::runtime_context::StoreTurnContextResolver::new(
        adapter.harness_store(org_id),
        adapter.agent_store(org_id),
        adapter.session_store(org_id),
        adapter.message_store(),
        adapter.provider_store(org_id),
        reason_capability_registry.clone(),
        adapter.driver_registry(),
    )
    .with_file_store(adapter.file_store());
    let mut atom = ReasonAtom::new(
        context_resolver,
        adapter.message_store(),
        reason_capability_registry.clone(),
        adapter.event_emitter(),
    );
    if let Some(image_resolver) = adapter.image_resolver(org_id) {
        atom = atom.with_image_resolver(image_resolver);
    }
    if let Some(hb) = adapter.stream_heartbeater() {
        atom = atom.with_stream_heartbeater(hb);
    }
    if let Some(timeout) = adapter.provider_stall_timeout() {
        atom = atom.with_provider_stall_timeout(timeout);
    }
    if let Some(config) = adapter.provider_retry_config() {
        atom = atom.with_provider_retry_config(config);
    }
    if let Some(store) = adapter.partial_stream_store() {
        atom = atom.with_partial_stream_store(store);
    }
    if let Some(store) = adapter.durable_tool_result_store() {
        atom = atom.with_durable_tool_result_store(store);
    }
    if let Some(store) = adapter.compaction_checkpoint_store() {
        atom = atom.with_compaction_checkpoint_store(store);
    }
    if let Some(handle) = adapter.reasoning_effort_handle(input.context.session_id) {
        atom = atom.with_reasoning_effort_handle(handle);
    }
    if let Some(utility_llm_service) = adapter.utility_llm_service() {
        atom = atom.with_utility_llm_service(utility_llm_service);
    }
    // Schedule store powers the `usage_limit_auto_continue` capability, which
    // schedules a continuation after a provider usage limit resets.
    if let Some(schedule_store) = adapter.schedule_store(org_id) {
        atom = atom.with_schedule_store(schedule_store);
    }

    let mut assembled = crate::runtime_context::assemble_turn_context_from_snapshot(
        turn_inputs.snapshot,
        adapter.message_store().as_ref(),
        adapter.provider_store(org_id).as_ref(),
        &reason_capability_registry,
        &adapter.driver_registry(),
        &turn_inputs.mcp_tool_definitions,
        Some(adapter.file_store()),
    )
    .await?;
    let input = ReasonInput {
        mcp_tool_definitions: turn_inputs.mcp_tool_definitions,
        ..input
    };

    if !user_prompt_message_overrides.is_empty() {
        for (message_id, message_override) in user_prompt_message_overrides {
            let message = assembled
                .messages
                .iter_mut()
                .find(|message| message.id == message_id)
                .ok_or_else(|| {
                    everruns_provider::error::AgentLoopError::config(
                        "user_prompt_submit mutation: input message not found in assembled context",
                    )
                })?;

            // Apply enforcement mutations to provider context only, retaining
            // persisted history as an audit record of the original content.
            message
                .content
                .retain(|part| !matches!(part, ContentPart::Text(_)));
            message
                .content
                .insert(0, ContentPart::text(message_override));
        }
    }
    atom.execute_with_assembled_context(input, assembled).await
}

pub async fn execute_act_activity<A: RuntimeHostAdapter>(
    adapter: &A,
    input: ActInput,
) -> everruns_provider::error::Result<ActResult> {
    let org_id = input.org_id.ok_or_else(|| {
        everruns_provider::error::AgentLoopError::config(
            "ActInput.org_id must be set for runtime host execution",
        )
    })?;

    if let Some(blocker) =
        detect_dependency_blocker(adapter, org_id, input.harness_id, input.agent_id).await?
    {
        RuntimeSessionLifecycle::new(adapter.clone(), org_id, input.context.session_id)
            .dependency_blocked(
                input.context.turn_id,
                input.context.input_message_id,
                blocker,
            )
            .await?;
        return Ok(ActResult {
            results: vec![],
            completed: true,
            success_count: 0,
            error_count: 1,
            waiting_for_tool_results: false,
            blocked: true,
            client_tool_calls: vec![],
            client_tool_definitions: vec![],
        });
    }

    let execution_capabilities = load_execution_capabilities(
        adapter,
        org_id,
        input.context.session_id,
        input.harness_id,
        input.agent_id,
        input.locale.clone(),
        input.blueprint_id.as_deref(),
    )
    .await?;
    let mut tool_registry = execution_capabilities.tool_registry;

    if let Some(augmentor) = adapter.tool_augmentor() {
        augmentor
            .augment_act_tools(
                input.context.session_id,
                adapter.session_store(org_id),
                adapter.session_task_registry(),
                adapter.file_store(),
                &input.tool_definitions,
                &mut tool_registry,
            )
            .await?;
    }

    // Register the session's MCP tools as first-class registry tools, so they
    // execute through the regular `ToolExecutor` path and are visible to
    // everything that introspects the registry (spawn_background, tool_search,
    // openai_tool_search namespaces, ...). The turn's tool definitions already
    // include the discovered MCP tools, so no re-discovery is needed; the host's
    // MCP executor supplies execution (knowledge/integrations/runtime-mcp.md D5).
    // The MCP invoker is reused below for the guardrails `mcp` check, which
    // delegates a guardrail decision to an external endpoint over the same
    // scoped-MCP client/auth (knowledge/execution/guardrails.md).
    let mut mcp_invoker: Option<Arc<dyn everruns_core::McpToolInvoker>> = None;
    if let Some(mcp) = adapter.mcp_executor(org_id, input.context.session_id).await {
        let invoker: Arc<dyn everruns_core::McpToolInvoker> = mcp;
        for tool in everruns_core::build_mcp_proxy_tools(&input.tool_definitions, invoker.clone()) {
            tool_registry.register_boxed(tool);
        }
        mcp_invoker = Some(Arc::new(everruns_core::ScopedMcpToolInvoker::new(
            &input.tool_definitions,
            invoker,
        )));
    }

    let builtin_tool_registry = Arc::new(tool_registry.clone());
    let context_services = runtime_tool_context_services(
        adapter,
        org_id,
        input.context.session_id,
        input.agent_id,
        Some(builtin_tool_registry),
        mcp_invoker,
        execution_capabilities.subagent_nesting_policy,
    );
    tool_registry.validate_context_services(&context_services)?;
    let executor: Arc<dyn everruns_core::tool_execution::ToolExecutor> = Arc::new(tool_registry);

    let mut atom = ActAtom::new(executor, adapter.event_emitter())
        .with_context_services(context_services)
        .with_post_tool_hooks(execution_capabilities.post_tool_hooks)
        .with_pre_tool_hooks(execution_capabilities.pre_tool_hooks)
        .with_tool_call_hooks(execution_capabilities.tool_call_hooks);

    #[cfg(feature = "builtins")]
    {
        atom = atom.with_final_post_tool_hook(Arc::new(everruns_builtins::PersistOutputHook));
    }

    if let Some(limiter) = adapter.outbound_tool_rate_limiter(org_id) {
        atom = atom.with_outbound_tool_rate_limiter(limiter);
    }
    if let Some(store) = adapter.durable_tool_result_store() {
        atom = atom.with_durable_tool_result_store(store);
    }

    atom.execute(input).await
}
