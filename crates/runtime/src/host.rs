// Shared host orchestration for embedded and durable execution hosts.
// Decision: everruns-runtime owns worker-facing turn phase execution so
// durable/server-backed hosts reuse the same input/reason/act wiring.

use async_trait::async_trait;
use everruns_core::atoms::{
    ActAtom, ActInput, ActResult, Atom, InputAtom, InputAtomInput, InputAtomResult, ReasonAtom,
    ReasonInput, ReasonResult,
};
use everruns_core::events::{
    EventContext, EventRequest, OutputMessageCompletedData, SessionActivatedData, SessionIdledData,
    TurnCompletedData, TurnFailedData, TurnStartedData,
};
use everruns_core::message::Message;
use everruns_core::message_retriever::MessageRetriever;
use everruns_core::platform_store::PlatformStore;
use everruns_core::session::SessionStatus;
use everruns_core::traits::{
    AgentStore, BudgetChecker, EventEmitter, HarnessStore, ImageResolver, LeasedResourceStore,
    LlmProviderStore, ModelWithProvider, SessionFileStore, SessionMutator, SessionResourceRegistry,
    SessionScheduleStore, SessionSqlDbStoreRef, SessionStorageStore, SessionStore,
    UserConnectionResolver,
};
use everruns_core::typed_id::{AgentId, HarnessId, MessageId, SessionId, TurnId};
use everruns_core::{
    Agent, CapabilityRegistry, DependencyBlocker, DriverRegistry, Harness, Session, TokenUsage,
    ToolDefinition, ToolRegistry, org_public_id_from_internal,
};
use std::sync::Arc;
use tracing::warn;

#[derive(Clone)]
struct HostAgentStore(Arc<dyn AgentStore>);

#[async_trait]
impl AgentStore for HostAgentStore {
    async fn get_agent(&self, agent_id: AgentId) -> everruns_core::error::Result<Option<Agent>> {
        self.0.get_agent(agent_id).await
    }
}

#[derive(Clone)]
struct HostHarnessStore(Arc<dyn HarnessStore>);

#[async_trait]
impl HarnessStore for HostHarnessStore {
    async fn get_harness_chain(
        &self,
        harness_id: HarnessId,
    ) -> everruns_core::error::Result<Vec<Harness>> {
        self.0.get_harness_chain(harness_id).await
    }
}

#[derive(Clone)]
struct HostSessionStore(Arc<dyn SessionStore>);

#[async_trait]
impl SessionStore for HostSessionStore {
    async fn get_session(
        &self,
        session_id: SessionId,
    ) -> everruns_core::error::Result<Option<Session>> {
        self.0.get_session(session_id).await
    }
}

#[derive(Clone)]
struct HostMessageStore(Arc<dyn MessageRetriever>);

#[async_trait]
impl MessageRetriever for HostMessageStore {
    async fn get(
        &self,
        session_id: SessionId,
        message_id: MessageId,
    ) -> everruns_core::error::Result<Option<Message>> {
        self.0.get(session_id, message_id).await
    }

    async fn load(&self, session_id: SessionId) -> everruns_core::error::Result<Vec<Message>> {
        self.0.load(session_id).await
    }

    async fn load_filtered(
        &self,
        query: everruns_core::message_filter::MessageQuery,
    ) -> everruns_core::error::Result<Vec<Message>> {
        self.0.load_filtered(query).await
    }

    async fn load_page(
        &self,
        session_id: SessionId,
        offset: usize,
        limit: usize,
    ) -> everruns_core::error::Result<Vec<Message>> {
        self.0.load_page(session_id, offset, limit).await
    }

    async fn count(&self, session_id: SessionId) -> everruns_core::error::Result<usize> {
        self.0.count(session_id).await
    }
}

#[derive(Clone)]
struct HostProviderStore(Arc<dyn LlmProviderStore>);

#[async_trait]
impl LlmProviderStore for HostProviderStore {
    async fn get_model_with_provider(
        &self,
        model_id: everruns_core::typed_id::ModelId,
    ) -> everruns_core::error::Result<Option<ModelWithProvider>> {
        self.0.get_model_with_provider(model_id).await
    }

    async fn get_default_model(&self) -> everruns_core::error::Result<Option<ModelWithProvider>> {
        self.0.get_default_model().await
    }
}

#[derive(Clone)]
struct HostEventEmitter(Arc<dyn EventEmitter>);

#[async_trait]
impl EventEmitter for HostEventEmitter {
    async fn emit(
        &self,
        request: EventRequest,
    ) -> everruns_core::error::Result<everruns_core::events::Event> {
        self.0.emit(request).await
    }
}

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
/// `everruns-runtime` owns the phase execution (`input -> reason -> act`).
/// Host crates implement this trait to provide their own persistence,
/// session-lifecycle plumbing, and tool registry construction.
///
/// The durable scheduler still lives outside this crate. Hosts use this trait
/// to delegate individual turn phases to `everruns-runtime`, while keeping
/// queueing, retries, and workflow resumption in their own layer.
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

    async fn build_tool_registry_for_agent(
        &self,
        org_id: i64,
        agent_id: AgentId,
    ) -> everruns_core::error::Result<ToolRegistry>;

    async fn build_tool_registry_for_harness(
        &self,
        org_id: i64,
        harness_id: HarnessId,
    ) -> everruns_core::error::Result<ToolRegistry>;

    fn harness_store(&self, org_id: i64) -> Arc<dyn HarnessStore>;

    fn agent_store(&self, org_id: i64) -> Arc<dyn AgentStore>;

    fn session_store(&self, org_id: i64) -> Arc<dyn SessionStore>;

    fn session_mutator(&self, org_id: i64) -> Arc<dyn SessionMutator>;

    fn provider_store(&self, org_id: i64) -> Arc<dyn LlmProviderStore>;

    fn message_store(&self) -> Arc<dyn MessageRetriever>;

    fn event_emitter(&self) -> Arc<dyn EventEmitter>;

    fn file_store(&self) -> Arc<dyn SessionFileStore>;

    fn image_resolver(&self, _org_id: i64) -> Option<Arc<dyn ImageResolver>> {
        None
    }

    fn storage_store(&self) -> Option<Arc<dyn SessionStorageStore>> {
        None
    }

    fn memory_store(&self) -> Option<Arc<dyn everruns_core::MemoryStoreBackend>> {
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

    fn platform_store(&self, _org_id: i64) -> Option<Arc<dyn PlatformStore>> {
        None
    }

    fn budget_checker(
        &self,
        _org_id: i64,
        _agent_id: Option<AgentId>,
    ) -> Option<Arc<dyn BudgetChecker>> {
        None
    }

    async fn build_tool_registry(
        &self,
        org_id: i64,
        harness_id: HarnessId,
        agent_id: Option<AgentId>,
        blueprint_id: Option<&str>,
    ) -> everruns_core::error::Result<ToolRegistry> {
        if let Some(blueprint_id) = blueprint_id {
            let mut registry = ToolRegistry::with_defaults();
            let capability_registry = self.capability_registry();
            let blueprint = capability_registry.blueprint(blueprint_id).ok_or_else(|| {
                everruns_core::error::AgentLoopError::config(format!(
                    "Blueprint \"{blueprint_id}\" not found in registry"
                ))
            })?;
            for tool in blueprint.tools {
                registry.register_boxed(tool);
            }
            return Ok(registry);
        }

        match agent_id {
            Some(agent_id) => self.build_tool_registry_for_agent(org_id, agent_id).await,
            None => {
                self.build_tool_registry_for_harness(org_id, harness_id)
                    .await
            }
        }
    }

    async fn post_tool_hooks(
        &self,
        org_id: i64,
        harness_id: HarnessId,
        agent_id: Option<AgentId>,
        blueprint_id: Option<&str>,
    ) -> everruns_core::error::Result<Vec<Arc<dyn everruns_core::PostToolExecHook>>> {
        if blueprint_id.is_some() {
            return Ok(Vec::new());
        }

        let capability_registry = self.capability_registry();
        let capability_configs = match agent_id {
            Some(agent_id) => self
                .get_agent(org_id, agent_id)
                .await?
                .map(|agent| agent.capabilities)
                .unwrap_or_default(),
            None => self
                .get_harness(org_id, harness_id)
                .await?
                .map(|harness| harness.capabilities)
                .unwrap_or_default(),
        };

        Ok(capability_configs
            .iter()
            .flat_map(|config| {
                capability_registry
                    .get(config.capability_id())
                    .map(|capability| capability.post_tool_exec_hooks())
                    .unwrap_or_default()
            })
            .collect())
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

    pub async fn emit_turn_completed(
        &self,
        turn_id: TurnId,
        input_message_id: MessageId,
        iterations: u32,
        usage: Option<TokenUsage>,
        input_content: Option<String>,
    ) {
        self.emit_event(EventRequest::new(
            self.session_id,
            EventContext::turn(turn_id, input_message_id),
            TurnCompletedData {
                turn_id,
                iterations,
                duration_ms: None,
                usage,
                input_content,
            },
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
            turn_id,
            input_message_id,
            iterations,
            usage.clone(),
            input_content,
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
        error_code: Option<&str>,
    ) {
        self.set_session_status(SessionStatus::Idle, "turn_failed")
            .await;

        self.emit_event(EventRequest::new(
            self.session_id,
            EventContext::turn(turn_id, input_message_id),
            TurnFailedData {
                turn_id,
                error: error.to_string(),
                error_code: error_code.map(str::to_string),
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
        self.emit_event(EventRequest::new(
            self.session_id,
            EventContext::turn(turn_id, input_message_id),
            OutputMessageCompletedData::new(Message::assistant(blocker.message())),
        ))
        .await;

        self.turn_failed(
            turn_id,
            input_message_id,
            blocker.message(),
            Some(blocker.error_code()),
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

    let atom = InputAtom::new(HostMessageStore(adapter.message_store()));
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
            response_id: None,
            locale: None,
            network_access: None,
        });
    }

    let turn_context = adapter
        .load_turn_context(org_id, input.context.session_id)
        .await?;

    let mut atom = ReasonAtom::new(
        HostHarnessStore(adapter.harness_store(org_id)),
        HostAgentStore(adapter.agent_store(org_id)),
        HostSessionStore(adapter.session_store(org_id)),
        HostMessageStore(adapter.message_store()),
        HostProviderStore(adapter.provider_store(org_id)),
        adapter.capability_registry(),
        adapter.driver_registry(),
        HostEventEmitter(adapter.event_emitter()),
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

    let tool_registry = adapter
        .build_tool_registry(
            org_id,
            input.harness_id,
            input.agent_id,
            input.blueprint_id.as_deref(),
        )
        .await?;
    let builtin_tool_registry = Arc::new(tool_registry.clone());

    let mut atom = ActAtom::with_file_store(
        tool_registry,
        HostEventEmitter(adapter.event_emitter()),
        adapter.file_store(),
    )
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
    .with_post_tool_hooks(
        adapter
            .post_tool_hooks(
                org_id,
                input.harness_id,
                input.agent_id,
                input.blueprint_id.as_deref(),
            )
            .await?,
    );

    if let Some(storage_store) = adapter.storage_store() {
        atom = atom.with_storage_store(storage_store);
    }
    if let Some(memory_store) = adapter.memory_store() {
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
    if let Some(platform_store) = adapter.platform_store(org_id) {
        atom = atom.with_platform_store(platform_store);
    }
    if let Some(budget_checker) = adapter.budget_checker(org_id, input.agent_id) {
        atom = atom.with_budget_checker(budget_checker);
    }

    atom.execute(input).await
}
