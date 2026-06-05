// Shared host orchestration for embedded and durable execution hosts.
// Decision: everruns-runtime owns worker-facing turn phase execution so
// durable/server-backed hosts reuse the same input/reason/act wiring.

use async_trait::async_trait;
use everruns_core::atoms::{
    ActAtom, ActInput, ActResult, Atom, InputAtom, InputAtomInput, InputAtomResult, ReasonAtom,
    ReasonInput, ReasonResult,
};
use everruns_core::capabilities::{SystemPromptContext, collect_capabilities_with_configs};
use everruns_core::events::{
    EventContext, EventRequest, OutputMessageCompletedData, SessionActivatedData, SessionIdledData,
    TurnCompletedData, TurnFailedData, TurnStartedData,
};
use everruns_core::message::{ContentPart, Message};
use everruns_core::message_retriever::MessageRetriever;
use everruns_core::platform_store::PlatformStore;
use everruns_core::session::SessionStatus;
use everruns_core::traits::{
    AgentStore, BudgetChecker, EventEmitter, HarnessStore, ImageArtifactStore, ImageResolver,
    LeasedResourceStore, LlmProviderStore, ModelWithProvider, PaymentAuthority,
    ProviderCredentialStore, SessionFileSystem, SessionMutator, SessionResourceRegistry,
    SessionScheduleStore, SessionSqlDbStoreRef, SessionStorageStore, SessionStore,
    UserConnectionResolver,
};
use everruns_core::typed_id::{AgentId, HarnessId, MessageId, SessionId, TurnId};
use everruns_core::{
    Agent, CapabilityRegistry, CapabilityStatus, DependencyBlocker, DriverRegistry, EgressService,
    Harness, Session, TokenUsage, ToolDefinition, ToolRegistry, UserFacingError, UtilityLlmService,
    assemble_turn_context, org_public_id_from_internal, resolve_runtime_capabilities,
};
use std::sync::Arc;
use tracing::warn;

/// Turn context loaded in one batched call for runtime host execution.
#[derive(Debug, Clone)]
pub struct RuntimeHostTurnContext {
    pub agent: Option<Agent>,
    pub session: Session,
    pub messages: Vec<Message>,
    pub model: Option<ModelWithProvider>,
    pub mcp_tool_definitions: Vec<ToolDefinition>,
}

/// Public adapter contract for server-backed or durable runtime hosts.
///
/// `everruns-runtime` owns shared host orchestration for both embedded and
/// durable execution. That includes phase execution (`input -> reason -> act`),
/// lifecycle emission, and the generic turn-strategy decisions used by durable
/// or custom hosts.
///
/// Host crates implement this trait to provide persistence, session-lifecycle
/// plumbing, event delivery, and their own orchestration backend. The durable
/// engine itself remains outside this crate.
#[async_trait]
pub trait RuntimeHostAdapter: Send + Sync + Clone + 'static {
    async fn get_agent(
        &self,
        org_id: i64,
        agent_id: AgentId,
    ) -> everruns_core::error::Result<Option<Agent>>;

    async fn get_harness(
        &self,
        org_id: i64,
        harness_id: HarnessId,
    ) -> everruns_core::error::Result<Option<Harness>>;

    async fn set_session_status(
        &self,
        org_id: i64,
        session_id: SessionId,
        status: SessionStatus,
    ) -> everruns_core::error::Result<Session>;

    async fn load_turn_context(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> everruns_core::error::Result<RuntimeHostTurnContext>;

    fn capability_registry(&self) -> CapabilityRegistry;

    fn driver_registry(&self) -> DriverRegistry;

    fn harness_store(&self, org_id: i64) -> Arc<dyn HarnessStore>;

    fn agent_store(&self, org_id: i64) -> Arc<dyn AgentStore>;

    fn session_store(&self, org_id: i64) -> Arc<dyn SessionStore>;

    fn session_mutator(&self, org_id: i64) -> Arc<dyn SessionMutator>;

    fn provider_store(&self, org_id: i64) -> Arc<dyn LlmProviderStore>;

    fn message_store(&self) -> Arc<dyn MessageRetriever>;

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

    fn memory_store(&self, _org_id: i64) -> Option<Arc<dyn everruns_core::MemoryStoreBackend>> {
        None
    }

    fn connection_resolver(&self) -> Option<Arc<dyn UserConnectionResolver>> {
        None
    }

    fn sqldb_store(&self) -> Option<SessionSqlDbStoreRef> {
        None
    }

    fn leased_resource_store(&self) -> Option<Arc<dyn LeasedResourceStore>> {
        None
    }

    fn session_resource_registry(&self) -> Option<Arc<dyn SessionResourceRegistry>> {
        None
    }

    fn schedule_store(&self, _org_id: i64) -> Option<Arc<dyn SessionScheduleStore>> {
        None
    }

    fn platform_store(
        &self,
        _org_id: i64,
        _session_id: SessionId,
    ) -> Option<Arc<dyn PlatformStore>> {
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

    /// Per-org outbound tool-call rate limiter (TM-TOOL-009).
    /// Default: `None` (no rate limiting — suitable for in-process / test environments).
    fn outbound_tool_rate_limiter(
        &self,
        _org_id: i64,
    ) -> Option<Arc<dyn everruns_core::OutboundToolRateLimiter>> {
        None
    }

    /// MCP executor routing `mcp_*` tool calls for this session, if the host
    /// configures MCP (specs/runtime-mcp.md D4). Default: `None`, so hosts
    /// without scoped MCP servers keep the plain tool registry unchanged.
    async fn mcp_executor(
        &self,
        _org_id: i64,
        _session_id: SessionId,
    ) -> Option<Arc<everruns_mcp::McpExecutor>> {
        None
    }
}

struct RuntimeExecutionCapabilities {
    tool_registry: ToolRegistry,
    post_tool_hooks: Vec<Arc<dyn everruns_core::PostToolExecHook>>,
    pre_tool_hooks: Vec<Arc<dyn everruns_core::atoms::PreToolUseHook>>,
    tool_call_hooks: Vec<Arc<dyn everruns_core::ToolCallHook>>,
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
    resolved_capability_configs: &[everruns_core::capability_types::AgentCapabilityConfig],
    capability_registry: &CapabilityRegistry,
) -> Vec<everruns_core::user_hook_types::UserHookSpec> {
    let mut hook_contributions: Vec<(String, Vec<everruns_core::user_hook_types::UserHookSpec>)> =
        Vec::new();
    let mut disabled_contributions: Vec<String> = Vec::new();
    for config in resolved_capability_configs {
        let Some(capability) = capability_registry.get(config.capability_id()) else {
            continue;
        };
        let specs = capability.user_hooks_with_config(&config.config);
        if !specs.is_empty() {
            hook_contributions.push((config.capability_id().to_string(), specs));
        }
        if config.capability_id() == "user_hooks" {
            disabled_contributions.extend(
                everruns_core::capabilities::user_hooks::disabled_contributions(&config.config),
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
) -> everruns_core::error::Result<(
    Vec<everruns_core::user_hook_types::UserHookSpec>,
    Arc<dyn everruns_core::hook_executor::BashHookDispatcher>,
)> {
    let capability_registry = adapter.capability_registry();
    let harness_chain = adapter
        .harness_store(org_id)
        .get_harness_chain(harness_id)
        .await?;
    if harness_chain.is_empty() {
        return Err(everruns_core::error::AgentLoopError::harness_not_found(
            harness_id,
        ));
    }
    let session = adapter
        .session_store(org_id)
        .get_session(session_id)
        .await?
        .ok_or_else(|| everruns_core::error::AgentLoopError::session_not_found(session_id))?;
    let agent = match agent_id {
        Some(agent_id) => adapter.agent_store(org_id).get_agent(agent_id).await?,
        None => None,
    };
    let resolved = resolve_runtime_capabilities(
        &harness_chain,
        agent.as_ref(),
        &session,
        &capability_registry,
    );
    let specs =
        finalize_specs_from_configs(&resolved.resolved_capability_configs, &capability_registry);
    let dispatcher: Arc<dyn everruns_core::hook_executor::BashHookDispatcher> = Arc::new(
        everruns_core::hook_dispatch::VirtualBashHookDispatcher::new(adapter.file_store()),
    );
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
) -> everruns_core::error::Result<RuntimeExecutionCapabilities> {
    let capability_registry = adapter.capability_registry();
    if let Some(blueprint_id) = blueprint_id {
        let mut registry = ToolRegistry::with_defaults();
        let blueprint = capability_registry.blueprint(blueprint_id).ok_or_else(|| {
            everruns_core::error::AgentLoopError::config(format!(
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
        });
    }

    let harness_chain = adapter
        .harness_store(org_id)
        .get_harness_chain(harness_id)
        .await?;
    if harness_chain.is_empty() {
        return Err(everruns_core::error::AgentLoopError::harness_not_found(
            harness_id,
        ));
    }

    let session = adapter
        .session_store(org_id)
        .get_session(session_id)
        .await?
        .ok_or_else(|| everruns_core::error::AgentLoopError::session_not_found(session_id))?;

    let agent_store = adapter.agent_store(org_id);
    let agent = match agent_id {
        Some(agent_id) => Some(
            agent_store
                .get_agent(agent_id)
                .await?
                .ok_or_else(|| everruns_core::error::AgentLoopError::agent_not_found(agent_id))?,
        ),
        None => None,
    };

    let resolved = resolve_runtime_capabilities(
        &harness_chain,
        agent.as_ref(),
        &session,
        &capability_registry,
    );
    // Executor (act) path: this builds the worker-side tool registry, not the
    // model-visible tool list. The model is left unset, so a model-adaptive
    // capability like `auto_tool_search` resolves to its provider-agnostic
    // client-side mechanism here. That registers the `tool_search` tool in the
    // executor, which is a harmless superset: on native models the reason path
    // never shows that tool to the model, so it is simply never called.
    let prompt_ctx = SystemPromptContext {
        session_id,
        locale: locale.or(session.locale.clone()),
        file_store: Some(adapter.file_store()),
        model: None,
    };
    let collected = collect_capabilities_with_configs(
        &resolved.resolved_capability_configs,
        &capability_registry,
        &prompt_ctx,
    )
    .await;

    let mut registry = ToolRegistry::with_defaults();
    for tool in collected.tools {
        registry.register_boxed(tool);
    }

    // Only `Available` capabilities contribute hooks, matching
    // `collect_capabilities_with_configs` (which skips non-available
    // capabilities). This keeps a `ComingSoon`/unavailable capability from
    // affecting execution via any of its hook seams.
    let mut post_tool_hooks: Vec<Arc<dyn everruns_core::PostToolExecHook>> = resolved
        .resolved_capability_configs
        .iter()
        .flat_map(|config| {
            capability_registry
                .get(config.capability_id())
                .filter(|capability| capability.status() == CapabilityStatus::Available)
                .map(|capability| capability.post_tool_exec_hooks())
                .unwrap_or_default()
        })
        .collect();

    // User-hook contributions (see `specs/user-hooks.md`). `finalize_specs_from_configs`
    // gathers specs across every resolved capability — both the user-facing
    // `user_hooks` capability and any capability that bundles hooks — and applies
    // `finalize_hook_specs` (namespace stamping, stable ids, `disabled_contributions`
    // muting; TM-HOOK-004). The same helper backs the lifecycle firing points so
    // every event finalizes specs identically.
    let user_hook_specs =
        finalize_specs_from_configs(&resolved.resolved_capability_configs, &capability_registry);
    // Capability-contributed pre-tool hooks run first (e.g. approval gating),
    // then user-hook (`PreToolUse`) specs. The first hook to block wins.
    let mut pre_tool_hooks: Vec<Arc<dyn everruns_core::atoms::PreToolUseHook>> = resolved
        .resolved_capability_configs
        .iter()
        .flat_map(|config| {
            capability_registry
                .get(config.capability_id())
                .filter(|capability| capability.status() == CapabilityStatus::Available)
                .map(|capability| capability.pre_tool_use_hooks())
                .unwrap_or_default()
        })
        .collect();
    if !user_hook_specs.is_empty() {
        let dispatcher: Arc<dyn everruns_core::hook_executor::BashHookDispatcher> = Arc::new(
            everruns_core::hook_dispatch::VirtualBashHookDispatcher::new(adapter.file_store()),
        );
        post_tool_hooks.extend(everruns_core::hook_adapter::build_post_tool_use_hooks(
            &user_hook_specs,
            dispatcher.clone(),
        ));
        pre_tool_hooks.extend(everruns_core::hook_adapter::build_pre_tool_use_hooks(
            &user_hook_specs,
            dispatcher,
        ));
    }

    let tool_call_hooks = resolved
        .resolved_capability_configs
        .iter()
        .flat_map(|config| {
            capability_registry
                .get(config.capability_id())
                .filter(|capability| capability.status() == CapabilityStatus::Available)
                .map(|capability| capability.tool_call_hooks())
                .unwrap_or_default()
        })
        .collect();

    Ok(RuntimeExecutionCapabilities {
        tool_registry: registry,
        post_tool_hooks,
        pre_tool_hooks,
        tool_call_hooks,
    })
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

    async fn set_session_status(&self, status: SessionStatus, action: &'static str) {
        if let Err(error) = self
            .adapter
            .set_session_status(self.org_id, self.session_id, status)
            .await
        {
            warn!(
                session_id = %self.session_id,
                org_id = self.org_id,
                action,
                %error,
                "runtime host lifecycle status update failed"
            );
        }
    }

    async fn emit_event(&self, request: EventRequest) {
        let event_type = request.event_type.clone();
        if let Err(error) = self.adapter.event_emitter().emit(request).await {
            warn!(
                session_id = %self.session_id,
                org_id = self.org_id,
                event_type,
                %error,
                "runtime host lifecycle event emission failed"
            );
        }
    }

    pub async fn turn_started(&self, turn_id: TurnId, input_message_id: MessageId) {
        let input_content = self
            .adapter
            .message_store()
            .get(self.session_id, input_message_id)
            .await
            .ok()
            .flatten()
            .map(|message| message.content_to_llm_string());

        self.set_session_status(SessionStatus::Active, "turn_started")
            .await;

        self.emit_event(EventRequest::new(
            self.session_id,
            EventContext::turn(turn_id, input_message_id),
            SessionActivatedData {
                turn_id,
                input_message_id,
            },
        ))
        .await;

        self.emit_event(EventRequest::new(
            self.session_id,
            EventContext::turn(turn_id, input_message_id),
            TurnStartedData {
                turn_id,
                input_message_id,
                input_content,
            },
        ))
        .await;
    }

    pub async fn emit_turn_completed(&self, input_message_id: MessageId, data: TurnCompletedData) {
        let turn_id = data.turn_id;
        self.emit_event(EventRequest::new(
            self.session_id,
            EventContext::turn(turn_id, input_message_id),
            data,
        ))
        .await;
    }

    pub async fn emit_session_idled(
        &self,
        turn_id: TurnId,
        input_message_id: MessageId,
        iterations: Option<u32>,
        usage: Option<TokenUsage>,
    ) {
        self.set_session_status(SessionStatus::Idle, "emit_session_idled")
            .await;

        self.emit_event(EventRequest::new(
            self.session_id,
            EventContext::turn(turn_id, input_message_id),
            SessionIdledData {
                turn_id,
                iterations,
                usage,
            },
        ))
        .await;
    }

    pub async fn turn_completed(
        &self,
        turn_id: TurnId,
        input_message_id: MessageId,
        iterations: u32,
        usage: Option<TokenUsage>,
        input_content: Option<String>,
    ) {
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
        .await;
        self.emit_session_idled(turn_id, input_message_id, Some(iterations), usage)
            .await;
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
    ) {
        let user_error =
            UserFacingError::new(everruns_core::user_facing_error_codes::BLOCKED_BY_HOOK);
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
        .await;

        self.turn_failed(turn_id, input_message_id, reason, Some(&user_error))
            .await;
    }

    pub async fn turn_failed(
        &self,
        turn_id: TurnId,
        input_message_id: MessageId,
        error: &str,
        user_error: Option<&UserFacingError>,
    ) {
        self.set_session_status(SessionStatus::Idle, "turn_failed")
            .await;

        self.emit_event(EventRequest::new(
            self.session_id,
            EventContext::turn(turn_id, input_message_id),
            {
                let mut data = TurnFailedData {
                    turn_id,
                    error: error.to_string(),
                    error_code: None,
                    error_fields: None,
                };
                if let Some(user_error) = user_error {
                    user_error.apply_to_event_fields(&mut data.error_code, &mut data.error_fields);
                }
                data
            },
        ))
        .await;

        self.emit_event(EventRequest::new(
            self.session_id,
            EventContext::turn(turn_id, input_message_id),
            SessionIdledData {
                turn_id,
                iterations: None,
                usage: None,
            },
        ))
        .await;
    }

    pub async fn waiting_for_tool_results(&self) {
        self.set_session_status(
            SessionStatus::WaitingForToolResults,
            "waiting_for_tool_results",
        )
        .await;
    }

    pub async fn dependency_blocked(
        &self,
        turn_id: TurnId,
        input_message_id: MessageId,
        blocker: DependencyBlocker,
    ) {
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
        .await;

        self.turn_failed(
            turn_id,
            input_message_id,
            blocker.message(),
            Some(&user_error),
        )
        .await;
    }
}

pub async fn detect_dependency_blocker<A: RuntimeHostAdapter>(
    adapter: &A,
    org_id: i64,
    harness_id: HarnessId,
    agent_id: Option<AgentId>,
) -> everruns_core::error::Result<Option<DependencyBlocker>> {
    let harness_store = adapter.harness_store(org_id);
    let agent_store = adapter.agent_store(org_id);
    everruns_core::detect_dependency_blocker(
        harness_store.as_ref(),
        agent_store.as_ref(),
        harness_id,
        agent_id,
    )
    .await
}

pub async fn execute_input_activity<A: RuntimeHostAdapter>(
    adapter: &A,
    org_id: i64,
    input: InputAtomInput,
) -> everruns_core::error::Result<InputAtomResult> {
    RuntimeSessionLifecycle::new(adapter.clone(), org_id, input.context.session_id)
        .turn_started(input.context.turn_id, input.context.input_message_id)
        .await;

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
) -> everruns_core::error::Result<Option<UserPromptHookResult>> {
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
) -> everruns_core::error::Result<ReasonResult> {
    if let Some(blocker) =
        detect_dependency_blocker(adapter, org_id, input.harness_id, input.agent_id).await?
    {
        RuntimeSessionLifecycle::new(adapter.clone(), org_id, input.context.session_id)
            .dependency_blocked(
                input.context.turn_id,
                input.context.input_message_id,
                blocker,
            )
            .await;
        return Ok(ReasonResult {
            success: false,
            text: blocker.message().to_string(),
            tool_calls: vec![],
            has_tool_calls: false,
            tool_definitions: vec![],
            max_iterations: everruns_core::runtime_agent::default_max_iterations(),
            error: Some("dependency_unavailable".to_string()),
            usage: None,
            output_message_id: None,
            time_to_first_token_ms: None,
            response_id: None,
            locale: None,
            network_access: None,
        });
    }

    // user_prompt_submit hook (see `specs/user-hooks.md`). Fires once per turn,
    // on the first reason iteration, before the LLM is consulted — the closest
    // choke point to "inbound user message accepted, before reason" that both
    // the in-process loop and the durable worker share. A `Block` aborts the
    // turn by reusing the same failure path as `dependency_blocked`: emit a
    // user-facing message + turn.failed, idle the session, and return a
    // non-success `ReasonResult` so no LLM/act work runs.
    let mut user_prompt_message_override = None;
    if input.iteration <= 1
        && let Some(hook_result) = run_user_prompt_submit_for_turn(adapter, org_id, &input).await?
    {
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
                    .await;
                return Ok(ReasonResult {
                    success: false,
                    text: user_message.unwrap_or_else(|| reason.clone()),
                    tool_calls: vec![],
                    has_tool_calls: false,
                    tool_definitions: vec![],
                    max_iterations: everruns_core::runtime_agent::default_max_iterations(),
                    error: Some("blocked_by_user_prompt_hook".to_string()),
                    usage: None,
                    output_message_id: None,
                    time_to_first_token_ms: None,
                    response_id: None,
                    locale: None,
                    network_access: None,
                });
            }
            everruns_core::lifecycle_hooks::UserPromptDecision::Continue { message } => {
                if message != hook_result.original_message {
                    user_prompt_message_override = Some(message);
                }
            }
        }
    }

    let turn_context = adapter
        .load_turn_context(org_id, input.context.session_id)
        .await?;

    let mut atom = ReasonAtom::new(
        adapter.harness_store(org_id),
        adapter.agent_store(org_id),
        adapter.session_store(org_id),
        adapter.message_store(),
        adapter.provider_store(org_id),
        adapter.capability_registry(),
        adapter.driver_registry(),
        adapter.event_emitter(),
    )
    .with_file_store(adapter.file_store());
    if let Some(image_resolver) = adapter.image_resolver(org_id) {
        atom = atom.with_image_resolver(image_resolver);
    }

    let input = ReasonInput {
        mcp_tool_definitions: turn_context.mcp_tool_definitions,
        ..input
    };

    if let Some(message_override) = user_prompt_message_override {
        let mut assembled = assemble_turn_context(
            adapter.harness_store(org_id).as_ref(),
            adapter.agent_store(org_id).as_ref(),
            adapter.session_store(org_id).as_ref(),
            adapter.message_store().as_ref(),
            adapter.provider_store(org_id).as_ref(),
            &adapter.capability_registry(),
            input.context.session_id,
            input.harness_id,
            input.agent_id,
            &input.mcp_tool_definitions,
            Some(adapter.file_store()),
        )
        .await?;

        if let Some(message) = assembled
            .messages
            .iter_mut()
            .find(|message| message.id == input.context.input_message_id)
        {
            // user_prompt_submit mutations are enforcement controls for the
            // provider-bound prompt. Apply them to the assembled context only
            // so persisted user history remains an audit record of the input.
            message.content = vec![ContentPart::text(message_override)];
        }

        return atom.execute_with_assembled_context(input, assembled).await;
    }

    atom.execute(input).await
}

pub async fn execute_act_activity<A: RuntimeHostAdapter>(
    adapter: &A,
    input: ActInput,
) -> everruns_core::error::Result<ActResult> {
    let org_id = input.org_id.ok_or_else(|| {
        everruns_core::error::AgentLoopError::config(
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
            .await;
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

    // Register the session's MCP tools as first-class registry tools, so they
    // execute through the regular `ToolExecutor` path and are visible to
    // everything that introspects the registry (spawn_background, tool_search,
    // openai_tool_search namespaces, ...). The turn's tool definitions already
    // include the discovered MCP tools, so no re-discovery is needed; the host's
    // MCP executor supplies execution (specs/runtime-mcp.md D5).
    if let Some(mcp) = adapter.mcp_executor(org_id, input.context.session_id).await {
        let invoker: Arc<dyn everruns_core::McpToolInvoker> = mcp;
        for tool in everruns_core::build_mcp_proxy_tools(&input.tool_definitions, invoker) {
            tool_registry.register_boxed(tool);
        }
    }

    let builtin_tool_registry = Arc::new(tool_registry.clone());
    let executor: Arc<dyn everruns_core::traits::ToolExecutor> = Arc::new(tool_registry);

    let mut atom =
        ActAtom::with_file_store(executor, adapter.event_emitter(), adapter.file_store())
            .with_session_store(adapter.session_store(org_id))
            .with_session_mutator(adapter.session_mutator(org_id))
            .with_agent_store(adapter.agent_store(org_id))
            .with_tool_registry(builtin_tool_registry)
            .with_org_id(
                org_public_id_from_internal(org_id)
                    .parse()
                    .expect("internal org id converts to valid public org id"),
            )
            .with_capability_registry(adapter.capability_registry())
            .with_post_tool_hooks(execution_capabilities.post_tool_hooks)
            .with_pre_tool_hooks(execution_capabilities.pre_tool_hooks)
            .with_tool_call_hooks(execution_capabilities.tool_call_hooks);

    if let Some(storage_store) = adapter.storage_store() {
        atom = atom.with_storage_store(storage_store);
    }
    if let Some(image_store) = adapter.image_artifact_store(org_id) {
        atom = atom.with_image_store(image_store);
    }
    if let Some(provider_credential_store) = adapter.provider_credential_store(org_id) {
        atom = atom.with_provider_credential_store(provider_credential_store);
    }
    if let Some(utility_llm_service) = adapter.utility_llm_service() {
        atom = atom.with_utility_llm_service(utility_llm_service);
    }
    if let Some(egress_service) = adapter.egress_service() {
        atom = atom.with_egress_service(egress_service);
    }
    if let Some(memory_store) = adapter.memory_store(org_id) {
        atom = atom.with_memory_store(memory_store);
    }
    if let Some(connection_resolver) = adapter.connection_resolver() {
        atom = atom.with_connection_resolver(connection_resolver);
    }
    if let Some(sqldb_store) = adapter.sqldb_store() {
        atom = atom.with_sqldb_store(sqldb_store);
    }
    if let Some(leased_resource_store) = adapter.leased_resource_store() {
        atom = atom.with_leased_resource_store(leased_resource_store);
    }
    if let Some(registry) = adapter.session_resource_registry() {
        atom = atom.with_session_resource_registry(registry);
    }
    if let Some(schedule_store) = adapter.schedule_store(org_id) {
        atom = atom.with_schedule_store(schedule_store);
    }
    if let Some(platform_store) = adapter.platform_store(org_id, input.context.session_id) {
        atom = atom.with_platform_store(platform_store);
    }
    if let Some(budget_checker) = adapter.budget_checker(org_id, input.agent_id) {
        atom = atom.with_budget_checker(budget_checker);
    }
    if let Some(payment_authority) = adapter.payment_authority(org_id, input.agent_id) {
        atom = atom.with_payment_authority(payment_authority);
    }
    if let Some(limiter) = adapter.outbound_tool_rate_limiter(org_id) {
        atom = atom.with_outbound_tool_rate_limiter(limiter);
    }

    atom.execute(input).await
}
