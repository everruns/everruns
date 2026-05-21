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
use everruns_core::message::Message;
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
    Agent, CapabilityRegistry, DependencyBlocker, DriverRegistry, Harness, Session, TokenUsage,
    ToolDefinition, ToolRegistry, UserFacingError, org_public_id_from_internal,
    resolve_runtime_capabilities,
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
}

struct RuntimeExecutionCapabilities {
    tool_registry: ToolRegistry,
    post_tool_hooks: Vec<Arc<dyn everruns_core::PostToolExecHook>>,
    tool_call_hooks: Vec<Arc<dyn everruns_core::ToolCallHook>>,
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
    let prompt_ctx = SystemPromptContext {
        session_id,
        locale: locale.or(session.locale.clone()),
        file_store: Some(adapter.file_store()),
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

    let post_tool_hooks = resolved
        .resolved_capability_configs
        .iter()
        .flat_map(|config| {
            capability_registry
                .get(config.capability_id())
                .map(|capability| capability.post_tool_exec_hooks())
                .unwrap_or_default()
        })
        .collect();
    let tool_call_hooks = resolved
        .resolved_capability_configs
        .iter()
        .flat_map(|config| {
            capability_registry
                .get(config.capability_id())
                .map(|capability| capability.tool_call_hooks())
                .unwrap_or_default()
        })
        .collect();

    Ok(RuntimeExecutionCapabilities {
        tool_registry: registry,
        post_tool_hooks,
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

    atom.execute(ReasonInput {
        mcp_tool_definitions: turn_context.mcp_tool_definitions,
        ..input
    })
    .await
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
    let tool_registry = execution_capabilities.tool_registry;
    let builtin_tool_registry = Arc::new(tool_registry.clone());

    let mut atom =
        ActAtom::with_file_store(tool_registry, adapter.event_emitter(), adapter.file_store())
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

    atom.execute(input).await
}
