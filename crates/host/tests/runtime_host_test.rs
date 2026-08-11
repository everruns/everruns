use async_trait::async_trait;
use chrono::Utc;
use everruns_core::MessageRetriever;
use everruns_core::ToolContext;
use everruns_core::atoms::ReasonResult;
use everruns_core::atoms::{ActInput, AtomContext, InputAtomInput};
use everruns_core::capabilities::{
    Capability, CapabilityStatus, SystemPromptContext, collect_capabilities_with_configs,
};
use everruns_core::driver_registry::DriverRegistry;
use everruns_core::in_memory::{
    InMemoryAgentStore, InMemoryEventEmitter, InMemoryHarnessStore, InMemoryMessageRetriever,
    InMemoryProviderStore,
};
use everruns_core::session_task::{
    CreateSessionTask, NewTaskMessage, SessionTask, SessionTaskFilter, SessionTaskRegistry,
    SessionTaskUpdate, TaskMessage, apply_task_update, new_session_task,
};
use everruns_core::traits::{
    AgentStore, EventEmitter, HarnessStore, ProviderStore, SessionFileSystem, SessionMutator,
    SessionStore,
};
use everruns_core::typed_id::{AgentId, HarnessId, MessageId, SessionId, TurnId};
use everruns_core::{
    AgentCapabilityConfig, AgentDefinition, CapabilityRegistry, DriverId, EventData,
    ExecutionSession, HarnessDefinition, InputMessage, ResolvedModel, SessionExecutionState,
    TokenUsage, Tool, ToolCall, ToolExecutionResult, ToolRegistry, ToolResult,
    inspect_turn_context, user_facing_error_codes,
};
use everruns_host::{
    InMemorySessionFileStore, ResolvedTurnInputs, RuntimeHostAdapter, RuntimeSessionLifecycle,
    RuntimeTurnPlan, RuntimeTurnState, TurnStopReason, execute_act_activity,
    execute_input_activity, plan_next_host_turn,
};
use everruns_test_support::TestMathCapability;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Clone, Default)]
struct TestSessionStore {
    sessions: Arc<RwLock<HashMap<SessionId, ExecutionSession>>>,
}

impl TestSessionStore {
    async fn insert(&self, session: ExecutionSession) {
        self.sessions.write().await.insert(session.id, session);
    }

    async fn set_status(
        &self,
        session_id: SessionId,
        status: SessionExecutionState,
    ) -> ExecutionSession {
        let mut sessions = self.sessions.write().await;
        let session = sessions.get_mut(&session_id).expect("session exists");
        session.status = status;
        session.clone()
    }
}

#[async_trait]
impl SessionStore for TestSessionStore {
    async fn get_session(
        &self,
        session_id: SessionId,
    ) -> everruns_core::error::Result<Option<ExecutionSession>> {
        Ok(self.sessions.read().await.get(&session_id).cloned())
    }
}

#[async_trait]
impl SessionMutator for TestSessionStore {
    async fn update_session_title(
        &self,
        session_id: SessionId,
        title: String,
    ) -> everruns_core::error::Result<ExecutionSession> {
        let mut sessions = self.sessions.write().await;
        let session = sessions.get_mut(&session_id).expect("session exists");
        session.title = Some(title);
        Ok(session.clone())
    }
}

#[derive(Clone)]
struct MockHostAdapter {
    capability_registry: CapabilityRegistry,
    driver_registry: DriverRegistry,
    harness_store: Arc<InMemoryHarnessStore>,
    agent_store: Arc<InMemoryAgentStore>,
    session_store: Arc<TestSessionStore>,
    message_store: Arc<InMemoryMessageRetriever>,
    provider_store: Arc<InMemoryProviderStore>,
    event_emitter: Arc<InMemoryEventEmitter>,
    file_store: Arc<InMemorySessionFileStore>,
    session_task_registry: Option<Arc<dyn SessionTaskRegistry>>,
}

#[async_trait]
impl RuntimeHostAdapter for MockHostAdapter {
    async fn set_session_status(
        &self,
        _org_id: i64,
        session_id: SessionId,
        status: SessionExecutionState,
    ) -> everruns_core::error::Result<()> {
        self.session_store.set_status(session_id, status).await;
        Ok(())
    }

    async fn load_resolved_turn(
        &self,
        _org_id: i64,
        session_id: SessionId,
    ) -> everruns_core::error::Result<ResolvedTurnInputs> {
        let session = self
            .session_store
            .get_session(session_id)
            .await?
            .expect("session exists");
        let agent = match session.agent_id {
            Some(agent_id) => self.agent_store.get_agent(agent_id).await?,
            None => None,
        };
        let harness = self
            .harness_store
            .get_harness(session.harness_id)
            .await?
            .expect("harness exists");
        let snapshot =
            everruns_core::ResolvedExecutionSnapshot::project(&harness, agent.as_ref(), &session)?;
        Ok(ResolvedTurnInputs {
            snapshot,
            messages: self.message_store.load(session_id).await?,
            mcp_tool_definitions: vec![],
        })
    }

    fn capability_registry(&self) -> CapabilityRegistry {
        self.capability_registry.clone()
    }

    fn driver_registry(&self) -> DriverRegistry {
        self.driver_registry.clone()
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

    fn message_store(&self) -> Arc<dyn everruns_core::MessageRetriever> {
        self.message_store.clone()
    }

    fn event_emitter(&self) -> Arc<dyn EventEmitter> {
        self.event_emitter.clone()
    }

    fn file_store(&self) -> Arc<dyn SessionFileSystem> {
        self.file_store.clone()
    }

    fn session_task_registry(&self) -> Option<Arc<dyn SessionTaskRegistry>> {
        self.session_task_registry.clone()
    }
}

#[derive(Default)]
struct TestTaskRegistry {
    tasks: RwLock<Vec<SessionTask>>,
}

#[async_trait]
impl SessionTaskRegistry for TestTaskRegistry {
    async fn create(&self, input: CreateSessionTask) -> everruns_core::error::Result<SessionTask> {
        let task = new_session_task(input, Utc::now());
        self.tasks.write().await.push(task.clone());
        Ok(task)
    }

    async fn update(
        &self,
        session_id: SessionId,
        task_id: &str,
        update: SessionTaskUpdate,
    ) -> everruns_core::error::Result<Option<SessionTask>> {
        let mut tasks = self.tasks.write().await;
        let Some(task) = tasks
            .iter_mut()
            .find(|task| task.session_id == session_id && task.id == task_id)
        else {
            return Ok(None);
        };
        apply_task_update(task, update, Utc::now());
        Ok(Some(task.clone()))
    }

    async fn get(
        &self,
        session_id: SessionId,
        task_id: &str,
    ) -> everruns_core::error::Result<Option<SessionTask>> {
        Ok(self
            .tasks
            .read()
            .await
            .iter()
            .find(|task| task.session_id == session_id && task.id == task_id)
            .cloned())
    }

    async fn list(
        &self,
        session_id: SessionId,
        filter: Option<&SessionTaskFilter>,
    ) -> everruns_core::error::Result<Vec<SessionTask>> {
        Ok(self
            .tasks
            .read()
            .await
            .iter()
            .filter(|task| {
                task.session_id == session_id
                    && filter
                        .and_then(|filter| filter.kind.as_deref())
                        .is_none_or(|kind| task.kind == kind)
            })
            .cloned()
            .collect())
    }

    async fn request_cancel(
        &self,
        session_id: SessionId,
        task_id: &str,
    ) -> everruns_core::error::Result<Option<SessionTask>> {
        self.get(session_id, task_id).await
    }

    async fn record_message(
        &self,
        _session_id: SessionId,
        _task_id: &str,
        _message: NewTaskMessage,
    ) -> everruns_core::error::Result<TaskMessage> {
        Err(everruns_core::error::AgentLoopError::tool("not needed"))
    }

    async fn list_messages(
        &self,
        _session_id: SessionId,
        _task_id: &str,
        _limit: Option<u32>,
        _after_id: Option<&str>,
    ) -> everruns_core::error::Result<Vec<TaskMessage>> {
        Ok(vec![])
    }
}

async fn build_registry(
    capability_registry: &CapabilityRegistry,
    session_id: SessionId,
    capabilities: &[AgentCapabilityConfig],
) -> everruns_core::error::Result<ToolRegistry> {
    let ctx = SystemPromptContext::without_file_store(session_id);
    let collected =
        collect_capabilities_with_configs(capabilities, capability_registry, &ctx).await;
    let mut registry = ToolRegistry::with_defaults();
    for tool in collected.tools {
        registry.register_boxed(tool);
    }
    Ok(registry)
}

struct OverlayEchoTool;

#[async_trait]
impl Tool for OverlayEchoTool {
    fn name(&self) -> &str {
        "overlay_echo"
    }

    fn description(&self) -> &str {
        "Returns the provided value."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "value": {"type": "string"}
            },
            "required": ["value"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: serde_json::Value) -> ToolExecutionResult {
        ToolExecutionResult::success(json!({
            "value": arguments["value"].as_str().unwrap_or_default(),
        }))
    }
}

struct ContextParityTool;

#[async_trait]
impl Tool for ContextParityTool {
    fn name(&self) -> &str {
        "context_parity"
    }

    fn description(&self) -> &str {
        "Reports whether runtime host services reached ToolContext."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({"type": "object", "additionalProperties": false})
    }

    async fn execute(&self, _arguments: serde_json::Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("context required")
    }

    async fn execute_with_context(
        &self,
        _arguments: serde_json::Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        ToolExecutionResult::success(json!({
            "file_store": context.file_store.is_some(),
            "message_retriever": context.message_retriever.is_some(),
            "session_store": context.session_store.is_some(),
            "session_mutator": context.session_mutator.is_some(),
            "agent_store": context.agent_store.is_some(),
            "session_task_registry": context.session_task_registry.is_some(),
            "capability_registry": context.capability_registry.is_some(),
            "tool_registry": context.tool_registry.is_some(),
            "org_id": context.org_id.is_some(),
            "event_emitter": context.event_emitter.is_some(),
            "event_context": context.event_context.is_some(),
            "tool_call_id": context.tool_call_id.is_some(),
        }))
    }
}

struct ContextParityCapability;

impl Capability for ContextParityCapability {
    fn id(&self) -> &str {
        "context_parity"
    }

    fn name(&self) -> &str {
        "Context Parity"
    }

    fn description(&self) -> &str {
        "Test capability for runtime-owned ToolContext service propagation."
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(ContextParityTool)]
    }
}

struct OverlayHook;

#[async_trait]
impl everruns_core::atoms::PostToolExecHook for OverlayHook {
    async fn after_exec(
        &self,
        _tool_call: &ToolCall,
        _tool_def: &everruns_core::ToolDefinition,
        result: &mut ToolResult,
        _context: &ToolContext,
    ) {
        if let Some(value) = result
            .result
            .as_mut()
            .and_then(|value| value.as_object_mut())
        {
            value.insert("hooked".to_string(), json!(true));
        }
    }
}

/// Test capability that contributes a pre-tool hook blocking `overlay_echo`.
/// Exercises the `Capability::pre_tool_use_hooks` seam used for approval gating.
struct GateCapability;

impl Capability for GateCapability {
    fn id(&self) -> &str {
        "gate"
    }

    fn name(&self) -> &str {
        "Gate"
    }

    fn description(&self) -> &str {
        "Test capability contributing a blocking pre-tool hook."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn pre_tool_use_hooks(&self) -> Vec<Arc<dyn everruns_core::atoms::PreToolUseHook>> {
        vec![Arc::new(BlockEchoHook)]
    }
}

/// Same blocking pre-tool hook as `GateCapability`, but the capability reports
/// a non-`Available` status. Used to assert hooks from unavailable capabilities
/// are not collected (mirrors `collect_capabilities_with_configs`).
struct ComingSoonGateCapability;

impl Capability for ComingSoonGateCapability {
    fn id(&self) -> &str {
        "coming_soon_gate"
    }

    fn name(&self) -> &str {
        "Coming Soon Gate"
    }

    fn description(&self) -> &str {
        "Unavailable test capability contributing a blocking pre-tool hook."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::ComingSoon
    }

    fn pre_tool_use_hooks(&self) -> Vec<Arc<dyn everruns_core::atoms::PreToolUseHook>> {
        vec![Arc::new(BlockEchoHook)]
    }
}

struct BlockEchoHook;

#[async_trait]
impl everruns_core::atoms::PreToolUseHook for BlockEchoHook {
    async fn before_exec(
        &self,
        tool_call: ToolCall,
        _tool_def: &everruns_core::ToolDefinition,
        _context: &ToolContext,
    ) -> everruns_core::atoms::PreToolUseDecision {
        if tool_call.name == "overlay_echo" {
            everruns_core::atoms::PreToolUseDecision::Block {
                tool_call,
                reason: "denied by test policy".into(),
                user_message: None,
            }
        } else {
            everruns_core::atoms::PreToolUseDecision::Continue(tool_call)
        }
    }
}

struct OverlayEchoCapability;

impl Capability for OverlayEchoCapability {
    fn id(&self) -> &str {
        "overlay_echo"
    }

    fn name(&self) -> &str {
        "Overlay Echo"
    }

    fn description(&self) -> &str {
        "Test capability that contributes a tool and a post-tool hook."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(OverlayEchoTool)]
    }

    fn post_tool_exec_hooks(&self) -> Vec<Arc<dyn everruns_core::atoms::PostToolExecHook>> {
        vec![Arc::new(OverlayHook)]
    }
}

struct OverlayAliasCapability;

impl Capability for OverlayAliasCapability {
    fn id(&self) -> &str {
        "overlay_alias"
    }

    fn name(&self) -> &str {
        "Overlay Alias"
    }

    fn description(&self) -> &str {
        "Test capability that depends on overlay_echo."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["overlay_echo"]
    }
}

/// Tool whose `Tool::narrate()` returns a distinctive line, so the act path
/// must surface capability-owned narration rather than the generic
/// `Running {display_name}` / `Ran {display_name}` fallback (EVE-601).
struct NarratingTool;

#[async_trait]
impl Tool for NarratingTool {
    fn name(&self) -> &str {
        "narrating_tool"
    }

    fn display_name(&self) -> Option<&str> {
        // Distinct from the narration so a generic fallback is detectable.
        Some("Narrating Tool")
    }

    fn description(&self) -> &str {
        "Returns the provided value with capability-owned narration."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {"value": {"type": "string"}},
            "required": ["value"],
            "additionalProperties": false
        })
    }

    fn narrate(
        &self,
        _tool_call: &ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        _locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        use everruns_core::tool_narration::ToolNarrationPhase;
        match phase {
            ToolNarrationPhase::Started | ToolNarrationPhase::Waiting => {
                Some("Narrating tool: starting work".to_string())
            }
            ToolNarrationPhase::Completed => Some("Narrating tool: finished work".to_string()),
            ToolNarrationPhase::Failed => Some("Narrating tool: failed".to_string()),
        }
    }

    async fn execute(&self, arguments: serde_json::Value) -> ToolExecutionResult {
        ToolExecutionResult::success(json!({
            "value": arguments["value"].as_str().unwrap_or_default(),
        }))
    }
}

/// Capability contributing `NarratingTool`. Relies on the default
/// `Capability::narrate()` dispatch to the tool's `narrate()`, which the runtime
/// act path must consult via the collected `CapabilityNarrationHook`.
struct NarratingCapability;

impl Capability for NarratingCapability {
    fn id(&self) -> &str {
        "narrating"
    }

    fn name(&self) -> &str {
        "Narrating"
    }

    fn description(&self) -> &str {
        "Test capability whose tool owns its narration."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(NarratingTool)]
    }
}

/// Explicit `ToolCallHook` whose narration must win over the default capability
/// `Tool::narrate()` when both are present (AC #7: model-authored narration such
/// as `human_intent` keeps precedence).
struct ExplicitNarrationHook;

impl everruns_core::capabilities::ToolCallHook for ExplicitNarrationHook {
    fn narration(
        &self,
        _tool_def: Option<&everruns_core::ToolDefinition>,
        tool_call: &ToolCall,
        _phase: everruns_core::tool_narration::ToolNarrationPhase,
        _locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        if tool_call.name == "narrating_tool" {
            Some("Explicit hook narration wins".to_string())
        } else {
            None
        }
    }
}

/// Capability contributing both `NarratingTool` (which owns `Tool::narrate()`)
/// and an explicit `tool_call_hooks()` entry. Collection orders the explicit
/// hook before the generated `CapabilityNarrationHook`, so the explicit hook
/// must take precedence on the act path.
struct ExplicitNarrationCapability;

impl Capability for ExplicitNarrationCapability {
    fn id(&self) -> &str {
        "explicit_narration"
    }

    fn name(&self) -> &str {
        "Explicit Narration"
    }

    fn description(&self) -> &str {
        "Test capability with an explicit tool-call narration hook."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(NarratingTool)]
    }

    fn tool_call_hooks(&self) -> Vec<Arc<dyn everruns_core::capabilities::ToolCallHook>> {
        vec![Arc::new(ExplicitNarrationHook)]
    }
}

fn harness() -> HarnessDefinition {
    HarnessDefinition {
        capabilities: vec![AgentCapabilityConfig::new("test_math")],
        ..HarnessDefinition::new("math", "You are a math harness.")
    }
}

fn session(session_id: SessionId, harness_id: HarnessId) -> ExecutionSession {
    ExecutionSession {
        id: session_id,
        workspace_id: everruns_core::WorkspaceId::from_uuid((session_id).uuid()),
        organization_id: everruns_core::DEFAULT_ORG_PUBLIC_ID.to_string(),
        harness_id,
        agent_id: None,
        title: Some("Runtime Host".into()),
        goal: None,
        locale: None,
        tags: vec![],
        model_id: None,
        capabilities: vec![],
        tools: vec![],
        mcp_servers: Default::default(),
        system_prompt: None,
        initial_files: vec![],
        hints: None,
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
        status: SessionExecutionState::Started,
        usage: None,
        parent_session_id: None,
        forked_from_session_id: None,
        blueprint_id: None,
        blueprint_config: None,
    }
}

fn agent(agent_id: AgentId, capabilities: Vec<AgentCapabilityConfig>) -> AgentDefinition {
    AgentDefinition {
        display_name: Some("Test Agent".into()),
        capabilities,
        max_iterations: Some(8),
        ..AgentDefinition::new(agent_id, "test-agent", "Use tools when needed.")
    }
}

fn turn_state(session_id: SessionId, harness_id: HarnessId) -> RuntimeTurnState {
    RuntimeTurnState {
        org_id: 1,
        session_id,
        harness_id,
        agent_id: None,
        input_message_id: MessageId::from_uuid(Uuid::now_v7()),
        turn_id: Some(TurnId::from_uuid(Uuid::now_v7())),
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
    }
}

fn mock_host() -> MockHostAdapter {
    let mut capability_registry = CapabilityRegistry::new();
    capability_registry.register(TestMathCapability);
    MockHostAdapter {
        capability_registry,
        driver_registry: DriverRegistry::new(),
        harness_store: Arc::new(InMemoryHarnessStore::new()),
        agent_store: Arc::new(InMemoryAgentStore::new()),
        session_store: Arc::new(TestSessionStore::default()),
        message_store: Arc::new(InMemoryMessageRetriever::new()),
        provider_store: Arc::new(InMemoryProviderStore::new()),
        event_emitter: Arc::new(InMemoryEventEmitter::new()),
        file_store: Arc::new(InMemorySessionFileStore::new()),
        session_task_registry: None,
    }
}

async fn set_default_model(adapter: &MockHostAdapter) {
    adapter
        .provider_store
        .set_default_model(ResolvedModel {
            model: "llmsim-model".into(),
            provider_type: DriverId::LlmSim,
            api_key: None,
            base_url: None,
            provider_metadata: None,
        })
        .await;
}

async fn reason_tool_definitions(
    adapter: &MockHostAdapter,
    session_id: SessionId,
    harness_id: HarnessId,
    agent_id: Option<AgentId>,
) -> Vec<everruns_core::ToolDefinition> {
    inspect_turn_context(
        adapter.harness_store.as_ref(),
        adapter.agent_store.as_ref(),
        adapter.session_store.as_ref(),
        adapter.message_store.as_ref(),
        adapter.provider_store.as_ref(),
        &adapter.capability_registry,
        session_id,
        harness_id,
        agent_id,
        &[],
        Some(adapter.file_store.clone()),
    )
    .await
    .expect("reason context")
    .runtime_agent
    .tools
}

#[tokio::test]
async fn input_activity_emits_lifecycle_events_and_marks_session_active() {
    let adapter = mock_host();
    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    adapter
        .harness_store
        .add_harness(harness_id, harness())
        .await;
    adapter
        .session_store
        .insert(session(session_id, harness_id))
        .await;

    let input_message = adapter
        .message_store
        .add(session_id, InputMessage::user("hello"))
        .await
        .unwrap();
    let turn_id = TurnId::from_uuid(Uuid::now_v7());

    execute_input_activity(
        &adapter,
        1,
        InputAtomInput {
            context: AtomContext::new(session_id, turn_id, input_message.id),
        },
    )
    .await
    .unwrap();

    let session = adapter
        .session_store
        .get_session(session_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(session.status, SessionExecutionState::Active);

    let event_types: Vec<_> = adapter
        .event_emitter
        .events()
        .await
        .into_iter()
        .map(|event| event.data.event_type().to_string())
        .collect();
    let turn_started_pos = event_types
        .iter()
        .position(|event_type| event_type == "turn.started")
        .expect("turn.started event");
    let session_activated_pos = event_types
        .iter()
        .position(|event_type| event_type == "session.activated")
        .expect("session.activated event");
    assert!(session_activated_pos < turn_started_pos);
}

#[tokio::test]
async fn act_activity_executes_capability_tools_from_harness_registry() {
    let adapter = mock_host();
    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    let input_message_id = MessageId::from_uuid(Uuid::now_v7());
    adapter
        .harness_store
        .add_harness(harness_id, harness())
        .await;
    adapter
        .session_store
        .insert(session(session_id, harness_id))
        .await;
    let tool_definitions = build_registry(
        &adapter.capability_registry,
        session_id,
        &[AgentCapabilityConfig::new("test_math")],
    )
    .await
    .unwrap()
    .tool_definitions();

    let result = execute_act_activity(
        &adapter,
        ActInput {
            org_id: Some(1),
            context: AtomContext::new(
                session_id,
                TurnId::from_uuid(Uuid::now_v7()),
                input_message_id,
            ),
            harness_id,
            agent_id: None,
            tool_calls: vec![ToolCall {
                id: "call_mul".into(),
                name: "multiply".into(),
                arguments: serde_json::json!({"a": 6, "b": 7}),
            }],
            tool_definitions,
            locale: None,
            blueprint_id: None,
            network_access: None,
            parallel_tool_calls: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(result.success_count, 1);
    assert_eq!(result.error_count, 0);
    assert_eq!(result.results.len(), 1);
}

#[tokio::test]
async fn runtime_host_services_reach_final_tool_context_in_parity() {
    let mut adapter = mock_host();
    adapter
        .capability_registry
        .register(ContextParityCapability);
    adapter.session_task_registry = Some(Arc::new(TestTaskRegistry::default()));
    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    let input_message_id = MessageId::from_uuid(Uuid::now_v7());
    let mut test_harness = harness();
    test_harness.capabilities = vec![AgentCapabilityConfig::new("context_parity")];
    adapter
        .harness_store
        .add_harness(harness_id, test_harness)
        .await;
    adapter
        .session_store
        .insert(session(session_id, harness_id))
        .await;

    let tool_definitions = build_registry(
        &adapter.capability_registry,
        session_id,
        &[AgentCapabilityConfig::new("context_parity")],
    )
    .await
    .unwrap()
    .tool_definitions();
    let result = execute_act_activity(
        &adapter,
        ActInput {
            org_id: Some(1),
            context: AtomContext::new(
                session_id,
                TurnId::from_uuid(Uuid::now_v7()),
                input_message_id,
            ),
            harness_id,
            agent_id: None,
            tool_calls: vec![ToolCall {
                id: "call_context_parity".into(),
                name: "context_parity".into(),
                arguments: json!({}),
            }],
            tool_definitions,
            locale: None,
            blueprint_id: None,
            network_access: None,
            parallel_tool_calls: None,
        },
    )
    .await
    .unwrap();

    let payload = result.results[0]
        .result
        .result
        .as_ref()
        .expect("context parity payload");
    for (service, available) in payload.as_object().expect("object payload") {
        assert_eq!(
            available,
            &json!(true),
            "{service} did not reach ToolContext"
        );
    }
}

/// EVE-601: the runtime act path must surface capability-owned `Tool::narrate()`
/// output (via the collected `CapabilityNarrationHook`) on `tool.started` /
/// `tool.completed`, not the generic `Running {display_name}` fallback.
#[tokio::test]
async fn act_activity_uses_capability_tool_narration_on_act_path() {
    let mut adapter = mock_host();
    adapter.capability_registry.register(NarratingCapability);
    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    let input_message_id = MessageId::from_uuid(Uuid::now_v7());
    adapter
        .harness_store
        .add_harness(
            harness_id,
            HarnessDefinition {
                capabilities: vec![AgentCapabilityConfig::new("narrating")],
                ..harness()
            },
        )
        .await;
    adapter
        .session_store
        .insert(session(session_id, harness_id))
        .await;
    let tool_definitions = build_registry(
        &adapter.capability_registry,
        session_id,
        &[AgentCapabilityConfig::new("narrating")],
    )
    .await
    .unwrap()
    .tool_definitions();

    let result = execute_act_activity(
        &adapter,
        ActInput {
            org_id: Some(1),
            context: AtomContext::new(
                session_id,
                TurnId::from_uuid(Uuid::now_v7()),
                input_message_id,
            ),
            harness_id,
            agent_id: None,
            tool_calls: vec![ToolCall {
                id: "call_narrate".into(),
                name: "narrating_tool".into(),
                arguments: json!({"value": "x"}),
            }],
            tool_definitions,
            locale: None,
            blueprint_id: None,
            network_access: None,
            parallel_tool_calls: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(result.success_count, 1);

    let events = adapter.event_emitter.events().await;
    let started = events
        .iter()
        .find_map(|event| match &event.data {
            EventData::ToolStarted(data) => Some(data.narration.clone()),
            _ => None,
        })
        .expect("tool.started event");
    let completed = events
        .iter()
        .find_map(|event| match &event.data {
            EventData::ToolCompleted(data) => Some(data.narration.clone()),
            _ => None,
        })
        .expect("tool.completed event");
    assert_eq!(
        started.as_deref(),
        Some("Narrating tool: starting work"),
        "tool.started must use capability-owned narration, not the generic fallback"
    );
    assert_eq!(
        completed.as_deref(),
        Some("Narrating tool: finished work"),
        "tool.completed must use capability-owned narration, not the generic fallback"
    );
}

/// AC #7: an explicit `Capability::tool_call_hooks()` narration must win over the
/// default capability `Tool::narrate()` when both are present, because collection
/// orders explicit hooks before the generated `CapabilityNarrationHook`.
#[tokio::test]
async fn act_activity_explicit_tool_call_hook_wins_over_capability_narration() {
    let mut adapter = mock_host();
    adapter
        .capability_registry
        .register(ExplicitNarrationCapability);
    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    let input_message_id = MessageId::from_uuid(Uuid::now_v7());
    adapter
        .harness_store
        .add_harness(
            harness_id,
            HarnessDefinition {
                capabilities: vec![AgentCapabilityConfig::new("explicit_narration")],
                ..harness()
            },
        )
        .await;
    adapter
        .session_store
        .insert(session(session_id, harness_id))
        .await;
    let tool_definitions = build_registry(
        &adapter.capability_registry,
        session_id,
        &[AgentCapabilityConfig::new("explicit_narration")],
    )
    .await
    .unwrap()
    .tool_definitions();

    let result = execute_act_activity(
        &adapter,
        ActInput {
            org_id: Some(1),
            context: AtomContext::new(
                session_id,
                TurnId::from_uuid(Uuid::now_v7()),
                input_message_id,
            ),
            harness_id,
            agent_id: None,
            tool_calls: vec![ToolCall {
                id: "call_narrate".into(),
                name: "narrating_tool".into(),
                arguments: json!({"value": "x"}),
            }],
            tool_definitions,
            locale: None,
            blueprint_id: None,
            network_access: None,
            parallel_tool_calls: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(result.success_count, 1);

    let events = adapter.event_emitter.events().await;
    let started = events
        .iter()
        .find_map(|event| match &event.data {
            EventData::ToolStarted(data) => Some(data.narration.clone()),
            _ => None,
        })
        .expect("tool.started event");
    assert_eq!(
        started.as_deref(),
        Some("Explicit hook narration wins"),
        "explicit tool-call hook narration must take precedence over default Tool::narrate()"
    );
}

#[tokio::test]
async fn act_activity_agent_session_executes_harness_overlay_tools_from_reason_path() {
    let mut adapter = mock_host();
    adapter.capability_registry.register(OverlayEchoCapability);
    set_default_model(&adapter).await;

    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    let agent_id = AgentId::from_uuid(Uuid::now_v7());
    let input_message_id = MessageId::from_uuid(Uuid::now_v7());

    adapter
        .harness_store
        .add_harness(
            harness_id,
            HarnessDefinition {
                capabilities: vec![AgentCapabilityConfig::new("overlay_echo")],
                ..harness()
            },
        )
        .await;
    adapter.agent_store.add_agent(agent(agent_id, vec![])).await;
    adapter
        .session_store
        .insert(ExecutionSession {
            agent_id: Some(agent_id),
            ..session(session_id, harness_id)
        })
        .await;

    let result = execute_act_activity(
        &adapter,
        ActInput {
            org_id: Some(1),
            context: AtomContext::new(
                session_id,
                TurnId::from_uuid(Uuid::now_v7()),
                input_message_id,
            ),
            harness_id,
            agent_id: Some(agent_id),
            tool_calls: vec![ToolCall {
                id: "call_overlay_echo".into(),
                name: "overlay_echo".into(),
                arguments: json!({"value": "merged"}),
            }],
            tool_definitions: reason_tool_definitions(
                &adapter,
                session_id,
                harness_id,
                Some(agent_id),
            )
            .await,
            locale: None,
            blueprint_id: None,
            network_access: None,
            parallel_tool_calls: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(result.success_count, 1);
    assert_eq!(result.error_count, 0);
    assert_eq!(
        result.results[0].result.result.as_ref().unwrap()["value"],
        "merged"
    );
}

#[tokio::test]
async fn act_activity_agent_session_resolves_transitive_overlay_capabilities() {
    let mut adapter = mock_host();
    adapter.capability_registry.register(OverlayEchoCapability);
    adapter.capability_registry.register(OverlayAliasCapability);
    set_default_model(&adapter).await;

    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    let agent_id = AgentId::from_uuid(Uuid::now_v7());
    let input_message_id = MessageId::from_uuid(Uuid::now_v7());

    adapter
        .harness_store
        .add_harness(
            harness_id,
            HarnessDefinition {
                capabilities: vec![AgentCapabilityConfig::new("overlay_alias")],
                ..harness()
            },
        )
        .await;
    adapter.agent_store.add_agent(agent(agent_id, vec![])).await;
    adapter
        .session_store
        .insert(ExecutionSession {
            agent_id: Some(agent_id),
            ..session(session_id, harness_id)
        })
        .await;

    let result = execute_act_activity(
        &adapter,
        ActInput {
            org_id: Some(1),
            context: AtomContext::new(
                session_id,
                TurnId::from_uuid(Uuid::now_v7()),
                input_message_id,
            ),
            harness_id,
            agent_id: Some(agent_id),
            tool_calls: vec![ToolCall {
                id: "call_overlay_dep".into(),
                name: "overlay_echo".into(),
                arguments: json!({"value": "dependency"}),
            }],
            tool_definitions: reason_tool_definitions(
                &adapter,
                session_id,
                harness_id,
                Some(agent_id),
            )
            .await,
            locale: None,
            blueprint_id: None,
            network_access: None,
            parallel_tool_calls: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(result.success_count, 1);
    assert_eq!(result.error_count, 0);
    assert_eq!(
        result.results[0].result.result.as_ref().unwrap()["value"],
        "dependency"
    );
}

#[tokio::test]
async fn act_activity_agent_session_runs_post_tool_hooks_from_merged_overlay() {
    let mut adapter = mock_host();
    adapter.capability_registry.register(OverlayEchoCapability);
    set_default_model(&adapter).await;

    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    let agent_id = AgentId::from_uuid(Uuid::now_v7());
    let input_message_id = MessageId::from_uuid(Uuid::now_v7());

    adapter
        .harness_store
        .add_harness(
            harness_id,
            HarnessDefinition {
                capabilities: vec![AgentCapabilityConfig::new("overlay_echo")],
                ..harness()
            },
        )
        .await;
    adapter.agent_store.add_agent(agent(agent_id, vec![])).await;
    adapter
        .session_store
        .insert(ExecutionSession {
            agent_id: Some(agent_id),
            ..session(session_id, harness_id)
        })
        .await;

    let result = execute_act_activity(
        &adapter,
        ActInput {
            org_id: Some(1),
            context: AtomContext::new(
                session_id,
                TurnId::from_uuid(Uuid::now_v7()),
                input_message_id,
            ),
            harness_id,
            agent_id: Some(agent_id),
            tool_calls: vec![ToolCall {
                id: "call_overlay_hook".into(),
                name: "overlay_echo".into(),
                arguments: json!({"value": "hook"}),
            }],
            tool_definitions: reason_tool_definitions(
                &adapter,
                session_id,
                harness_id,
                Some(agent_id),
            )
            .await,
            locale: None,
            blueprint_id: None,
            network_access: None,
            parallel_tool_calls: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(result.success_count, 1);
    assert_eq!(result.error_count, 0);
    assert_eq!(
        result.results[0].result.result.as_ref().unwrap()["hooked"],
        true
    );
}

#[tokio::test]
async fn act_activity_runs_capability_pre_tool_hook_and_blocks() {
    let mut adapter = mock_host();
    adapter.capability_registry.register(OverlayEchoCapability);
    adapter.capability_registry.register(GateCapability);
    set_default_model(&adapter).await;

    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    let agent_id = AgentId::from_uuid(Uuid::now_v7());
    let input_message_id = MessageId::from_uuid(Uuid::now_v7());

    adapter
        .harness_store
        .add_harness(
            harness_id,
            HarnessDefinition {
                capabilities: vec![
                    AgentCapabilityConfig::new("overlay_echo"),
                    AgentCapabilityConfig::new("gate"),
                ],
                ..harness()
            },
        )
        .await;
    adapter.agent_store.add_agent(agent(agent_id, vec![])).await;
    adapter
        .session_store
        .insert(ExecutionSession {
            agent_id: Some(agent_id),
            ..session(session_id, harness_id)
        })
        .await;

    let result = execute_act_activity(
        &adapter,
        ActInput {
            org_id: Some(1),
            context: AtomContext::new(
                session_id,
                TurnId::from_uuid(Uuid::now_v7()),
                input_message_id,
            ),
            harness_id,
            agent_id: Some(agent_id),
            tool_calls: vec![ToolCall {
                id: "call_blocked".into(),
                name: "overlay_echo".into(),
                arguments: json!({"value": "hook"}),
            }],
            tool_definitions: reason_tool_definitions(
                &adapter,
                session_id,
                harness_id,
                Some(agent_id),
            )
            .await,
            locale: None,
            blueprint_id: None,
            network_access: None,
            parallel_tool_calls: None,
        },
    )
    .await
    .unwrap();

    // The capability-contributed pre-tool hook blocked execution: the tool
    // never ran (no `value` payload) and the error names the block reason.
    assert_eq!(result.success_count, 0);
    assert_eq!(result.error_count, 1);
    let blocked = &result.results[0].result;
    assert!(
        blocked.result.is_none(),
        "blocked tool should not produce output"
    );
    let error = blocked
        .error
        .as_ref()
        .expect("blocked tool yields an error");
    assert!(
        error.contains("denied by test policy"),
        "error should carry the hook's reason: {error}"
    );
}

#[tokio::test]
async fn act_activity_skips_pre_tool_hooks_from_unavailable_capability() {
    let mut adapter = mock_host();
    adapter.capability_registry.register(OverlayEchoCapability);
    adapter
        .capability_registry
        .register(ComingSoonGateCapability);
    set_default_model(&adapter).await;

    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    let agent_id = AgentId::from_uuid(Uuid::now_v7());
    let input_message_id = MessageId::from_uuid(Uuid::now_v7());

    adapter
        .harness_store
        .add_harness(
            harness_id,
            HarnessDefinition {
                capabilities: vec![
                    AgentCapabilityConfig::new("overlay_echo"),
                    AgentCapabilityConfig::new("coming_soon_gate"),
                ],
                ..harness()
            },
        )
        .await;
    adapter.agent_store.add_agent(agent(agent_id, vec![])).await;
    adapter
        .session_store
        .insert(ExecutionSession {
            agent_id: Some(agent_id),
            ..session(session_id, harness_id)
        })
        .await;

    let result = execute_act_activity(
        &adapter,
        ActInput {
            org_id: Some(1),
            context: AtomContext::new(
                session_id,
                TurnId::from_uuid(Uuid::now_v7()),
                input_message_id,
            ),
            harness_id,
            agent_id: Some(agent_id),
            tool_calls: vec![ToolCall {
                id: "call_unavailable_gate".into(),
                name: "overlay_echo".into(),
                arguments: json!({"value": "hook"}),
            }],
            tool_definitions: reason_tool_definitions(
                &adapter,
                session_id,
                harness_id,
                Some(agent_id),
            )
            .await,
            locale: None,
            blueprint_id: None,
            network_access: None,
            parallel_tool_calls: None,
        },
    )
    .await
    .unwrap();

    // The gating capability is `ComingSoon`, so its pre-tool hook must not be
    // collected: the tool executes normally instead of being blocked.
    assert_eq!(result.success_count, 1);
    assert_eq!(result.error_count, 0);
    assert_eq!(
        result.results[0].result.result.as_ref().unwrap()["hooked"],
        true
    );
}

#[tokio::test]
async fn lifecycle_helper_sets_waiting_for_tool_results_status() {
    let adapter = mock_host();
    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    adapter
        .harness_store
        .add_harness(harness_id, harness())
        .await;
    adapter
        .session_store
        .insert(session(session_id, harness_id))
        .await;

    RuntimeSessionLifecycle::new(adapter.clone(), 1, session_id)
        .waiting_for_tool_results()
        .await;

    let session = adapter
        .session_store
        .get_session(session_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(session.status, SessionExecutionState::WaitingForToolResults);
}

#[tokio::test]
async fn plan_next_host_turn_schedules_reason_after_process_input() {
    let adapter = mock_host();
    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    adapter
        .harness_store
        .add_harness(harness_id, harness())
        .await;
    adapter
        .session_store
        .insert(session(session_id, harness_id))
        .await;

    let input = turn_state(session_id, harness_id);
    let turn_id = TurnId::from_uuid(Uuid::now_v7());
    let output = serde_json::json!({ "turn_id": turn_id.to_string() });

    let plan = plan_next_host_turn(&adapter, "process_input", &input, &output, 0)
        .await
        .unwrap();

    match plan {
        RuntimeTurnPlan::ScheduleReason(next) => {
            assert_eq!(next.session_id, session_id);
            assert_eq!(next.harness_id, harness_id);
            assert_eq!(next.turn_id, Some(turn_id));
            assert_eq!(next.iteration, 1);
            assert_eq!(next.previous_response_id, None);
        }
        other => panic!("expected ScheduleReason, got {other:?}"),
    }
}

#[tokio::test]
async fn plan_next_host_turn_schedules_act_after_reason_tool_calls() {
    let adapter = mock_host();
    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    adapter
        .harness_store
        .add_harness(harness_id, harness())
        .await;
    adapter
        .session_store
        .insert(session(session_id, harness_id))
        .await;

    let input = turn_state(session_id, harness_id);
    let output = serde_json::to_value(ReasonResult {
        success: true,
        text: "Calling a tool".into(),
        tool_calls: vec![ToolCall {
            id: "call_mul".into(),
            name: "multiply".into(),
            arguments: serde_json::json!({ "a": 6, "b": 7 }),
        }],
        has_tool_calls: true,
        tool_definitions: vec![],
        max_iterations: 8,
        error: None,
        user_facing_error: None,
        error_disclosure: None,
        usage: None,
        output_message_id: None,
        time_to_first_token_ms: None,
        response_id: Some("resp_123".into()),
        finish_reason: Some("tool_calls".into()),
        locale: Some("en-US".into()),
        network_access: None,
        parallel_tool_calls: None,
    })
    .unwrap();

    let plan = plan_next_host_turn(&adapter, "reason", &input, &output, 0)
        .await
        .unwrap();

    match plan {
        RuntimeTurnPlan::ScheduleAct(plan) => {
            assert_eq!(plan.input.context.session_id, session_id);
            assert_eq!(plan.input.context.turn_id, input.turn_id.unwrap());
            assert_eq!(plan.input.harness_id, harness_id);
            assert_eq!(plan.iteration, 1);
            assert_eq!(plan.previous_response_id.as_deref(), Some("resp_123"));
            assert_eq!(plan.input.tool_calls.len(), 1);
            // EVE-598: default None preference threads through unchanged.
            assert_eq!(plan.input.parallel_tool_calls, None);
        }
        other => panic!("expected ScheduleAct, got {other:?}"),
    }
}

#[tokio::test]
async fn plan_next_host_turn_surfaces_max_turn_requests_before_another_act() {
    let adapter = mock_host();
    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    adapter
        .harness_store
        .add_harness(harness_id, harness())
        .await;
    adapter
        .session_store
        .insert(session(session_id, harness_id))
        .await;

    let input = turn_state(session_id, harness_id);
    let output = serde_json::to_value(ReasonResult {
        success: true,
        text: "Calling another tool".into(),
        tool_calls: vec![ToolCall {
            id: "call_mul".into(),
            name: "multiply".into(),
            arguments: json!({ "a": 6, "b": 7 }),
        }],
        has_tool_calls: true,
        tool_definitions: vec![],
        max_iterations: 1,
        error: None,
        user_facing_error: None,
        error_disclosure: None,
        usage: None,
        output_message_id: None,
        time_to_first_token_ms: None,
        response_id: Some("resp_limit".into()),
        finish_reason: Some("tool_calls".into()),
        locale: None,
        network_access: None,
        parallel_tool_calls: None,
    })
    .unwrap();

    let plan = plan_next_host_turn(&adapter, "reason", &input, &output, 0)
        .await
        .unwrap();

    assert!(matches!(
        plan,
        RuntimeTurnPlan::Complete {
            stop_reason: TurnStopReason::MaxTurnRequests,
            error: None,
        }
    ));
}

/// EVE-598: the request-level `parallel_tool_calls` preference set on the
/// reason result threads through to the scheduled `ActInput` for every value.
#[tokio::test]
async fn plan_next_host_turn_threads_parallel_tool_calls_into_act() {
    for preference in [Some(true), Some(false), None] {
        let adapter = mock_host();
        let harness_id = HarnessId::from_uuid(Uuid::now_v7());
        let session_id = SessionId::from_uuid(Uuid::now_v7());
        adapter
            .harness_store
            .add_harness(harness_id, harness())
            .await;
        adapter
            .session_store
            .insert(session(session_id, harness_id))
            .await;

        let input = turn_state(session_id, harness_id);
        let output = serde_json::to_value(ReasonResult {
            success: true,
            text: "Calling a tool".into(),
            tool_calls: vec![ToolCall {
                id: "call_mul".into(),
                name: "multiply".into(),
                arguments: serde_json::json!({ "a": 6, "b": 7 }),
            }],
            has_tool_calls: true,
            tool_definitions: vec![],
            max_iterations: 8,
            error: None,
            user_facing_error: None,
            error_disclosure: None,
            usage: None,
            output_message_id: None,
            time_to_first_token_ms: None,
            response_id: Some("resp_123".into()),
            finish_reason: Some("tool_calls".into()),
            locale: None,
            network_access: None,
            parallel_tool_calls: preference,
        })
        .unwrap();

        let plan = plan_next_host_turn(&adapter, "reason", &input, &output, 0)
            .await
            .unwrap();

        match plan {
            RuntimeTurnPlan::ScheduleAct(plan) => {
                assert_eq!(
                    plan.input.parallel_tool_calls, preference,
                    "parallel_tool_calls must thread through unchanged"
                );
            }
            other => panic!("expected ScheduleAct, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn plan_next_host_turn_schedules_act_with_session_blueprint_id() {
    let adapter = mock_host();
    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    adapter
        .harness_store
        .add_harness(harness_id, harness())
        .await;
    adapter
        .session_store
        .insert(ExecutionSession {
            blueprint_id: Some("blueprint.private".into()),
            ..session(session_id, harness_id)
        })
        .await;

    let input = turn_state(session_id, harness_id);
    let output = serde_json::to_value(ReasonResult {
        success: true,
        text: "Calling a tool".into(),
        tool_calls: vec![ToolCall {
            id: "call_mul".into(),
            name: "multiply".into(),
            arguments: serde_json::json!({ "a": 6, "b": 7 }),
        }],
        has_tool_calls: true,
        tool_definitions: vec![],
        max_iterations: 8,
        error: None,
        user_facing_error: None,
        error_disclosure: None,
        usage: None,
        output_message_id: None,
        time_to_first_token_ms: None,
        response_id: Some("resp_blueprint".into()),
        finish_reason: Some("tool_calls".into()),
        locale: Some("en-US".into()),
        network_access: None,
        parallel_tool_calls: None,
    })
    .unwrap();

    let plan = plan_next_host_turn(&adapter, "reason", &input, &output, 0)
        .await
        .unwrap();

    match plan {
        RuntimeTurnPlan::ScheduleAct(plan) => {
            assert_eq!(
                plan.input.blueprint_id.as_deref(),
                Some("blueprint.private")
            );
        }
        other => panic!("expected ScheduleAct, got {other:?}"),
    }
}

#[tokio::test]
async fn plan_next_host_turn_continues_reason_when_steering_messages_are_pending() {
    let adapter = mock_host();
    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    adapter
        .harness_store
        .add_harness(harness_id, harness())
        .await;
    adapter
        .session_store
        .insert(session(session_id, harness_id))
        .await;

    let input = turn_state(session_id, harness_id);
    let output = serde_json::to_value(ReasonResult {
        success: true,
        text: "Continuing".into(),
        tool_calls: vec![],
        has_tool_calls: false,
        tool_definitions: vec![],
        max_iterations: 8,
        error: None,
        user_facing_error: None,
        error_disclosure: None,
        usage: None,
        output_message_id: None,
        time_to_first_token_ms: None,
        response_id: Some("resp_steer".into()),
        finish_reason: Some("stop".into()),
        locale: None,
        network_access: None,
        parallel_tool_calls: None,
    })
    .unwrap();

    let plan = plan_next_host_turn(&adapter, "reason", &input, &output, 2)
        .await
        .unwrap();

    match plan {
        RuntimeTurnPlan::ScheduleReason(next) => {
            assert_eq!(next.iteration, 2);
            assert_eq!(next.previous_response_id.as_deref(), Some("resp_steer"));
            assert_eq!(next.turn_id, input.turn_id);
        }
        other => panic!("expected ScheduleReason, got {other:?}"),
    }
}

#[tokio::test]
async fn plan_next_host_turn_emits_turn_completed_summary_fields() {
    let adapter = mock_host();
    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    let final_message_id = MessageId::from_uuid(Uuid::now_v7());
    adapter
        .harness_store
        .add_harness(harness_id, harness())
        .await;
    adapter
        .session_store
        .insert(session(session_id, harness_id))
        .await;

    let mut input = turn_state(session_id, harness_id);
    input.started_at = Some(Utc::now());
    input.cumulative_usage = Some(TokenUsage::new(10, 4));
    input.tool_call_count = 2;
    input.llm_call_count = 1;
    input.time_to_first_token_ms = Some(35);

    let output = serde_json::to_value(ReasonResult {
        success: true,
        text: "Final answer for export".into(),
        tool_calls: vec![],
        has_tool_calls: false,
        tool_definitions: vec![],
        max_iterations: 8,
        error: None,
        user_facing_error: None,
        error_disclosure: None,
        usage: Some(TokenUsage::new(20, 8)),
        output_message_id: Some(final_message_id),
        time_to_first_token_ms: Some(50),
        response_id: Some("resp_done".into()),
        finish_reason: Some("length".into()),
        locale: None,
        network_access: None,
        parallel_tool_calls: None,
    })
    .unwrap();

    let plan = plan_next_host_turn(&adapter, "reason", &input, &output, 0)
        .await
        .unwrap();
    assert!(matches!(
        plan,
        RuntimeTurnPlan::Complete {
            stop_reason: everruns_host::TurnStopReason::MaxTokens,
            error: None,
        }
    ));

    let events = adapter.event_emitter.events().await;
    let completed = events
        .into_iter()
        .find_map(|event| match event.data {
            EventData::TurnCompleted(data) => Some(data),
            _ => None,
        })
        .expect("turn.completed event emitted");

    assert_eq!(completed.final_message_id, Some(final_message_id));
    assert_eq!(
        completed.final_answer_preview.as_deref(),
        Some("Final answer for export")
    );
    assert_eq!(completed.time_to_first_token_ms, Some(35));
    assert_eq!(completed.tool_call_count, Some(2));
    assert_eq!(completed.llm_call_count, Some(2));
    assert_eq!(completed.status.as_deref(), Some("completed"));
    let usage = completed.usage.expect("aggregated usage");
    assert_eq!(usage.input_tokens, 30);
    assert_eq!(usage.output_tokens, 12);
    assert!(completed.duration_ms.is_some());
}

#[tokio::test]
async fn plan_next_host_turn_preserves_reason_failure_message() {
    let adapter = mock_host();
    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    adapter
        .harness_store
        .add_harness(harness_id, harness())
        .await;
    adapter
        .session_store
        .insert(session(session_id, harness_id))
        .await;

    let input = turn_state(session_id, harness_id);
    let output = serde_json::to_value(ReasonResult {
        success: false,
        text: "Budget exhausted. 100.00 tokens spent reached the 100.00 tokens limit. Increase the budget to continue."
            .into(),
        tool_calls: vec![],
        has_tool_calls: false,
        tool_definitions: vec![],
        max_iterations: 8,
        error: Some("Budget exhausted".into()),
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
    })
    .unwrap();

    let plan = plan_next_host_turn(&adapter, "reason", &input, &output, 0)
        .await
        .unwrap();

    assert!(matches!(plan, RuntimeTurnPlan::Complete { .. }));

    let events = adapter.event_emitter.events().await;
    let turn_failed = events
        .into_iter()
        .find_map(|event| match event.data {
            EventData::TurnFailed(data) => Some(data),
            _ => None,
        })
        .expect("turn.failed event emitted");

    assert_eq!(
        turn_failed.error,
        "Budget exhausted. 100.00 tokens spent reached the 100.00 tokens limit. Increase the budget to continue."
    );
    assert_eq!(
        turn_failed.error_code.as_deref(),
        Some(user_facing_error_codes::BUDGET_EXHAUSTED)
    );
    let error_fields = turn_failed.error_fields.expect("budget fields");
    assert_eq!(error_fields.get("spent"), Some(&json!(100.0)));
    assert_eq!(error_fields.get("limit"), Some(&json!(100.0)));
    assert_eq!(error_fields.get("currency"), Some(&json!("tokens")));
}

#[tokio::test]
async fn plan_next_host_turn_classifies_missing_api_key_as_provider_misconfigured() {
    let adapter = mock_host();
    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    adapter
        .harness_store
        .add_harness(harness_id, harness())
        .await;
    adapter
        .session_store
        .insert(session(session_id, harness_id))
        .await;

    let input = turn_state(session_id, harness_id);
    let output = serde_json::to_value(ReasonResult {
        success: false,
        text: "I encountered an error while processing your request. Please try again later."
            .into(),
        tool_calls: vec![],
        has_tool_calls: false,
        tool_definitions: vec![],
        max_iterations: 8,
        error: Some(
            "LLM error: API key is required. Configure the API key in provider settings.".into(),
        ),
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
    })
    .unwrap();

    let plan = plan_next_host_turn(&adapter, "reason", &input, &output, 0)
        .await
        .unwrap();

    assert!(matches!(plan, RuntimeTurnPlan::Complete { .. }));

    let events = adapter.event_emitter.events().await;
    let turn_failed = events
        .into_iter()
        .find_map(|event| match event.data {
            EventData::TurnFailed(data) => Some(data),
            _ => None,
        })
        .expect("turn.failed event emitted");

    assert_eq!(
        turn_failed.error_code.as_deref(),
        Some(user_facing_error_codes::PROVIDER_MISCONFIGURED)
    );
}

#[tokio::test]
async fn plan_next_host_turn_prefers_disclosed_user_facing_error_from_reason() {
    // Generic disclosure mode: the reason atom already collapsed a quota
    // failure into processing_error. The turn.failed event must reuse that
    // disclosed error instead of re-classifying the raw strings, otherwise
    // it would leak past the generic mode.
    let adapter = mock_host();
    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    adapter
        .harness_store
        .add_harness(harness_id, harness())
        .await;
    adapter
        .session_store
        .insert(session(session_id, harness_id))
        .await;

    let input = turn_state(session_id, harness_id);
    let output = serde_json::to_value(ReasonResult {
        success: false,
        text: "I encountered an error while processing your request. Please try again later."
            .into(),
        tool_calls: vec![],
        has_tool_calls: false,
        tool_definitions: vec![],
        max_iterations: 8,
        error: Some("LLM error: OpenAI API error (429): insufficient_quota".into()),
        user_facing_error: Some(everruns_core::UserFacingError::new(
            user_facing_error_codes::PROCESSING_ERROR,
        )),
        error_disclosure: Some(everruns_core::ErrorDisclosure::Generic),
        usage: None,
        output_message_id: None,
        time_to_first_token_ms: None,
        response_id: None,
        finish_reason: None,
        locale: None,
        network_access: None,
        parallel_tool_calls: None,
    })
    .unwrap();

    let plan = plan_next_host_turn(&adapter, "reason", &input, &output, 0)
        .await
        .unwrap();

    assert!(matches!(plan, RuntimeTurnPlan::Complete { .. }));

    let events = adapter.event_emitter.events().await;
    let turn_failed = events
        .into_iter()
        .find_map(|event| match event.data {
            EventData::TurnFailed(data) => Some(data),
            _ => None,
        })
        .expect("turn.failed event emitted");

    assert_eq!(
        turn_failed.error_code.as_deref(),
        Some(user_facing_error_codes::PROCESSING_ERROR)
    );
    assert_eq!(turn_failed.error_fields, None);
    assert_eq!(turn_failed.error_disclosure.as_deref(), Some("generic"));
}

#[tokio::test]
async fn plan_next_host_turn_waits_for_tool_results_when_session_hint_requests_it() {
    let adapter = mock_host();
    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    adapter
        .harness_store
        .add_harness(harness_id, harness())
        .await;

    let mut host_session = session(session_id, harness_id);
    host_session.hints = Some(std::collections::HashMap::from([(
        "setup_connection".to_string(),
        serde_json::json!(true),
    )]));
    adapter.session_store.insert(host_session).await;

    let input = turn_state(session_id, harness_id);
    let output = serde_json::json!({
        "waiting_for_tool_results": true,
        "blocked": false
    });

    let plan = plan_next_host_turn(&adapter, "act", &input, &output, 0)
        .await
        .unwrap();

    match plan {
        RuntimeTurnPlan::WaitForToolResults { resume } => {
            assert_eq!(resume.iteration, 2);
            assert_eq!(resume.turn_id, input.turn_id);
            let session = adapter
                .session_store
                .get_session(session_id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(session.status, SessionExecutionState::WaitingForToolResults);
        }
        other => panic!("expected WaitForToolResults, got {other:?}"),
    }
}

// ============================================================================
// Lifecycle hook wire-in tests (user_prompt_submit, turn_end)
// ============================================================================

/// Test capability that contributes one lifecycle hook via `user_hooks()`.
/// The bash command is run for real through the in-process `bashkit_shell`
/// dispatcher, so these tests exercise the full collect -> build -> dispatch
/// -> decision path, not a mock.
struct LifecycleHookCapability {
    event: everruns_core::user_hook_types::HookEvent,
    command: String,
}

impl Capability for LifecycleHookCapability {
    fn id(&self) -> &str {
        "lifecycle_hook_test"
    }
    fn name(&self) -> &str {
        "Lifecycle Hook Test"
    }
    fn description(&self) -> &str {
        "Test capability contributing a single lifecycle hook."
    }
    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }
    fn user_hooks(&self) -> Vec<everruns_core::user_hook_types::UserHookSpec> {
        use everruns_core::user_hook_types::{
            ExecutorSpec, HookMatcher, HookSource, OnError, UserHookSpec,
        };
        vec![UserHookSpec {
            id: Some("lc".into()),
            event: self.event,
            matcher: HookMatcher::default(),
            executor: ExecutorSpec::Bash {
                command: self.command.clone(),
                env: Default::default(),
            },
            timeout_ms: 5000,
            on_error: OnError::Warn,
            description: None,
            source: HookSource::UserConfig,
        }]
    }
}

/// A `user_prompt_submit` hook that emits a `block` decision must abort the
/// turn *before* the reason atom runs — verified by `execute_reason_activity`
/// returning a non-success result with the `blocked_by_user_prompt_hook`
/// error, without ever consulting an LLM (the mock host has no usable driver
/// for a real reason step, so reaching it would surface a different error).
#[tokio::test]
async fn user_prompt_submit_hook_blocks_turn_before_reason() {
    use everruns_core::atoms::ReasonInput;
    use everruns_core::user_hook_types::HookEvent;
    use everruns_host::execute_reason_activity;

    let mut adapter = mock_host();
    adapter.capability_registry.register(LifecycleHookCapability {
        event: HookEvent::UserPromptSubmit,
        command: r#"echo '{"decision":"block","reason":"policy","user_message":"Your message was blocked."}'"#
            .into(),
    });

    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    adapter
        .harness_store
        .add_harness(
            harness_id,
            HarnessDefinition {
                capabilities: vec![AgentCapabilityConfig::new("lifecycle_hook_test")],
                ..harness()
            },
        )
        .await;
    adapter
        .session_store
        .insert(session(session_id, harness_id))
        .await;
    let user_message = adapter
        .message_store
        .add(session_id, InputMessage::user("delete everything"))
        .await
        .unwrap();

    let turn_id = TurnId::from_uuid(Uuid::now_v7());
    let result = execute_reason_activity(
        &adapter,
        1,
        ReasonInput {
            context: AtomContext::new(session_id, turn_id, user_message.id),
            harness_id,
            agent_id: None,
            org_id: 1,
            mcp_tool_definitions: vec![],
            previous_response_id: None,
            iteration: 1,
        },
    )
    .await
    .unwrap();

    assert!(!result.success, "blocked turn must not succeed");
    assert_eq!(result.error.as_deref(), Some("blocked_by_user_prompt_hook"));
    assert_eq!(result.text, "Your message was blocked.");
    assert!(result.tool_calls.is_empty());

    // The block is observable as a turn.failed event carrying the hook code.
    let events = adapter.event_emitter.events().await;
    assert!(
        events.iter().any(|e| e.event_type == "turn.failed"),
        "expected a turn.failed event, got: {:?}",
        events.iter().map(|e| &e.event_type).collect::<Vec<_>>()
    );
}

/// Synthetic user messages injected between iterations must cross the same
/// prompt-policy boundary as the turn's original input.
#[tokio::test]
async fn user_prompt_submit_hook_blocks_injected_mid_turn_message() {
    use everruns_core::atoms::ReasonInput;
    use everruns_core::user_hook_types::HookEvent;
    use everruns_host::execute_reason_activity_with_prompt_messages;

    let mut adapter = mock_host();
    adapter
        .capability_registry
        .register(LifecycleHookCapability {
            event: HookEvent::UserPromptSubmit,
            command: r#"echo '{"decision":"block","reason":"policy"}'"#.into(),
        });

    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    adapter
        .harness_store
        .add_harness(
            harness_id,
            HarnessDefinition {
                capabilities: vec![AgentCapabilityConfig::new("lifecycle_hook_test")],
                ..harness()
            },
        )
        .await;
    adapter
        .session_store
        .insert(session(session_id, harness_id))
        .await;
    let original = adapter
        .message_store
        .add(session_id, InputMessage::user("continue working"))
        .await
        .unwrap();
    let wake = adapter
        .message_store
        .add(
            session_id,
            InputMessage::user("Task summary with disallowed content"),
        )
        .await
        .unwrap();

    let result = execute_reason_activity_with_prompt_messages(
        &adapter,
        1,
        ReasonInput {
            context: AtomContext::new(session_id, TurnId::from_uuid(Uuid::now_v7()), original.id),
            harness_id,
            agent_id: None,
            org_id: 1,
            mcp_tool_definitions: vec![],
            previous_response_id: None,
            iteration: 3,
        },
        vec![wake.id],
    )
    .await
    .unwrap();

    assert!(!result.success, "injected message must be blocked");
    assert_eq!(result.error.as_deref(), Some("blocked_by_user_prompt_hook"));
}

/// A `user_prompt_submit` hook that emits `allow` (empty stdout, exit 0) must
/// not block — `execute_reason_activity` proceeds past the hook to the reason
/// step. We only assert the hook did not short-circuit with the block error.
#[tokio::test]
async fn user_prompt_submit_hook_allow_does_not_block() {
    use everruns_core::atoms::ReasonInput;
    use everruns_core::user_hook_types::HookEvent;
    use everruns_host::execute_reason_activity;

    let mut adapter = mock_host();
    set_default_model(&adapter).await;
    adapter
        .capability_registry
        .register(LifecycleHookCapability {
            event: HookEvent::UserPromptSubmit,
            command: "true".into(), // empty stdout + exit 0 = allow
        });

    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    adapter
        .harness_store
        .add_harness(
            harness_id,
            HarnessDefinition {
                capabilities: vec![AgentCapabilityConfig::new("lifecycle_hook_test")],
                ..harness()
            },
        )
        .await;
    adapter
        .session_store
        .insert(session(session_id, harness_id))
        .await;
    let user_message = adapter
        .message_store
        .add(session_id, InputMessage::user("hello"))
        .await
        .unwrap();

    let turn_id = TurnId::from_uuid(Uuid::now_v7());
    let result = execute_reason_activity(
        &adapter,
        1,
        ReasonInput {
            context: AtomContext::new(session_id, turn_id, user_message.id),
            harness_id,
            agent_id: None,
            org_id: 1,
            mcp_tool_definitions: vec![],
            previous_response_id: None,
            iteration: 1,
        },
    )
    .await
    .unwrap();

    // Whatever the reason step does with the mock model, it must not be the
    // user-prompt-hook block path.
    assert_ne!(
        result.error.as_deref(),
        Some("blocked_by_user_prompt_hook"),
        "allow hook must not block the turn"
    );
}

/// A `user_prompt_submit` hook that emits `mutate` must rewrite the prompt
/// before the provider-bound reason call, without changing persisted history.
#[tokio::test]
async fn user_prompt_submit_hook_mutate_rewrites_reason_context() {
    use everruns_core::atoms::ReasonInput;
    use everruns_core::capabilities::InfinityContextCapability;
    use everruns_core::user_hook_types::HookEvent;
    use everruns_host::execute_reason_activity;
    use everruns_test_support::llmsim_driver::{LlmSimConfig, register_driver_with_config};

    let mut adapter = mock_host();
    let provider_messages = Arc::new(std::sync::Mutex::new(Vec::new()));
    register_driver_with_config(
        &mut adapter.driver_registry,
        LlmSimConfig::echo().with_message_capture(provider_messages.clone()),
    );
    set_default_model(&adapter).await;
    adapter
        .capability_registry
        .register(LifecycleHookCapability {
            event: HookEvent::UserPromptSubmit,
            command: r#"echo '{"decision":"mutate","patch":{"message":"sanitized prompt"}}'"#
                .into(),
        });
    adapter
        .capability_registry
        .register(InfinityContextCapability);

    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    adapter
        .harness_store
        .add_harness(
            harness_id,
            HarnessDefinition {
                capabilities: vec![
                    AgentCapabilityConfig::new("lifecycle_hook_test"),
                    AgentCapabilityConfig::with_config(
                        "infinity_context",
                        json!({ "max_recent_messages": 1 }),
                    ),
                ],
                ..harness()
            },
        )
        .await;
    adapter
        .session_store
        .insert(session(session_id, harness_id))
        .await;
    adapter
        .message_store
        .add(session_id, InputMessage::user("PAST RAW SECRET"))
        .await
        .unwrap();
    let user_message = adapter
        .message_store
        .add(session_id, InputMessage::user("SECRET prompt"))
        .await
        .unwrap();

    let turn_id = TurnId::from_uuid(Uuid::now_v7());
    let result = execute_reason_activity(
        &adapter,
        1,
        ReasonInput {
            context: AtomContext::new(session_id, turn_id, user_message.id),
            harness_id,
            agent_id: None,
            org_id: 1,
            mcp_tool_definitions: vec![],
            previous_response_id: None,
            iteration: 1,
        },
    )
    .await
    .unwrap();

    assert!(result.success, "mutated turn should reach the LLM");
    assert_eq!(result.text, "Echo: sanitized prompt");
    assert!(
        result
            .tool_definitions
            .iter()
            .all(|definition| definition.name() != "query_history"),
        "raw persisted history must not be exposed when prompt hooks can rewrite user text"
    );

    // The provider-visible context must still respect Infinity Context's
    // history-trimming filter (max_recent_messages: 1). Before the fix, the
    // whole `infinity_context` capability — including its message filter — was
    // unregistered when `query_history` was suppressed, so the older
    // "PAST RAW SECRET" audit message leaked into the provider prompt. Assert
    // it never reaches the driver while the current (mutated) prompt does.
    // Scope the guard in a block (no await inside) so it is dropped before the
    // trailing await below (clippy::await_holding_lock).
    let sent = {
        let calls = provider_messages.lock().unwrap();
        assert!(!calls.is_empty(), "the driver should have been called");
        format!("{calls:?}")
    };
    assert!(
        !sent.contains("PAST RAW SECRET"),
        "trimmed older history must not reach the provider prompt, got: {sent}"
    );
    assert!(
        sent.contains("sanitized prompt"),
        "the current mutated prompt should reach the provider, got: {sent}"
    );

    let stored_message = adapter
        .message_store
        .get(session_id, user_message.id)
        .await
        .unwrap()
        .expect("stored user message");
    assert_eq!(stored_message.content_to_llm_string(), "SECRET prompt");
}

#[tokio::test]
async fn reason_activity_injects_schema_tools_for_agent_handoff_child() {
    use everruns_core::atoms::ReasonInput;
    use everruns_core::session_task::{
        SessionTaskState, TASK_KIND_AGENT_HANDOFF, TaskLinks, TaskWakePolicy,
    };
    use everruns_host::execute_reason_activity;
    use everruns_test_support::llmsim_driver::{LlmSimConfig, register_driver_with_config};

    let mut adapter = mock_host();
    register_driver_with_config(&mut adapter.driver_registry, LlmSimConfig::fixed("ready"));
    set_default_model(&adapter).await;
    let registry = Arc::new(TestTaskRegistry::default());
    adapter.session_task_registry = Some(registry.clone());

    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let parent_id = SessionId::from_uuid(Uuid::now_v7());
    let child_id = SessionId::from_uuid(Uuid::now_v7());
    adapter
        .harness_store
        .add_harness(harness_id, harness())
        .await;
    let parent = session(parent_id, harness_id);
    let mut child = session(child_id, harness_id);
    child.parent_session_id = Some(parent_id);
    child.workspace_id = parent.workspace_id;
    adapter.session_store.insert(parent).await;
    adapter.session_store.insert(child).await;
    registry
        .create(CreateSessionTask {
            session_id: parent_id,
            id: Some("task_handoff_schema".to_string()),
            kind: TASK_KIND_AGENT_HANDOFF.to_string(),
            display_name: "Structured handoff".to_string(),
            spec: json!({
                "result_schema": {
                    "type": "object",
                    "properties": {"answer": {"type": "string"}},
                    "required": ["answer"]
                },
                "message_schema": {
                    "type": "object",
                    "properties": {"step": {"type": "string"}},
                    "required": ["step"]
                }
            }),
            state: SessionTaskState::Running,
            links: TaskLinks {
                child_session_id: Some(child_id),
                ..Default::default()
            },
            wake_policy: TaskWakePolicy::OnActivity,
        })
        .await
        .unwrap();
    let input = adapter
        .message_store
        .add(child_id, InputMessage::user("work"))
        .await
        .unwrap();

    let result = execute_reason_activity(
        &adapter,
        1,
        ReasonInput {
            context: AtomContext::new(child_id, TurnId::from_uuid(Uuid::now_v7()), input.id),
            harness_id,
            agent_id: None,
            org_id: 1,
            mcp_tool_definitions: vec![],
            previous_response_id: None,
            iteration: 1,
        },
    )
    .await
    .unwrap();

    let report_result = result
        .tool_definitions
        .iter()
        .find(|definition| definition.name() == "report_result")
        .expect("handoff child receives report_result");
    assert_eq!(report_result.parameters()["required"], json!(["answer"]));
    let report_progress = result
        .tool_definitions
        .iter()
        .find(|definition| definition.name() == "report_task_progress")
        .expect("handoff child receives report_task_progress");
    assert_eq!(report_progress.parameters()["required"], json!(["step"]));

    let invalid = execute_act_activity(
        &adapter,
        ActInput {
            org_id: Some(1),
            context: AtomContext::new(child_id, TurnId::from_uuid(Uuid::now_v7()), input.id),
            harness_id,
            agent_id: None,
            tool_calls: vec![ToolCall {
                id: "call_invalid_result".into(),
                name: "report_result".into(),
                arguments: json!({"answer": 42}),
            }],
            tool_definitions: result.tool_definitions.clone(),
            locale: None,
            blueprint_id: None,
            network_access: None,
            parallel_tool_calls: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        invalid.error_count, 1,
        "invalid schema payload must be retryable"
    );

    let valid = execute_act_activity(
        &adapter,
        ActInput {
            org_id: Some(1),
            context: AtomContext::new(child_id, TurnId::from_uuid(Uuid::now_v7()), input.id),
            harness_id,
            agent_id: None,
            tool_calls: vec![ToolCall {
                id: "call_valid_result".into(),
                name: "report_result".into(),
                arguments: json!({"answer": "done"}),
            }],
            tool_definitions: result.tool_definitions,
            locale: None,
            blueprint_id: None,
            network_access: None,
            parallel_tool_calls: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(valid.success_count, 1);
    let task = registry
        .get(parent_id, "task_handoff_schema")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        task.result_path.as_deref(),
        Some("/.tasks/task_handoff_schema/result.json")
    );
}

/// A `turn_end` hook fires (advisory) when a turn completes. We drive a full
/// in-process turn and assert the hook's side effect: a sentinel file written
/// to the session VFS by the hook command.
#[tokio::test]
async fn turn_end_hook_fires_on_turn_completion() {
    use everruns_core::user_hook_types::HookEvent;

    let mut adapter = mock_host();
    set_default_model(&adapter).await;
    adapter
        .capability_registry
        .register(LifecycleHookCapability {
            event: HookEvent::TurnEnd,
            command: "echo turn-ended > /workspace/.turn_end_sentinel".into(),
        });

    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    adapter
        .harness_store
        .add_harness(
            harness_id,
            HarnessDefinition {
                capabilities: vec![AgentCapabilityConfig::new("lifecycle_hook_test")],
                ..harness()
            },
        )
        .await;
    adapter
        .session_store
        .insert(session(session_id, harness_id))
        .await;

    // Fire the turn_end hooks directly through the lifecycle helper — this is
    // the exact call the in-process loop and durable strategy make at turn
    // completion, so it verifies the wired path without needing a full LLM turn.
    let lifecycle = RuntimeSessionLifecycle::new(adapter.clone(), 1, session_id);
    lifecycle
        .fire_turn_end_hooks(harness_id, None, TurnId::from_uuid(Uuid::now_v7()), true)
        .await;

    let sentinel = adapter
        .file_store
        .read_file(session_id, "/.turn_end_sentinel")
        .await;
    assert!(
        sentinel.is_ok() && sentinel.as_ref().unwrap().is_some(),
        "turn_end hook should have written the sentinel file, got: {sentinel:?}"
    );
}
