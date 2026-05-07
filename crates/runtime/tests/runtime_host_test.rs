use async_trait::async_trait;
use chrono::Utc;
use everruns_core::MessageRetriever;
use everruns_core::ToolContext;
use everruns_core::atoms::ReasonResult;
use everruns_core::atoms::{ActInput, AtomContext, InputAtomInput};
use everruns_core::capabilities::{
    Capability, CapabilityStatus, MemoryCapability, SystemPromptContext, TestMathCapability,
    collect_capabilities_with_configs,
};
use everruns_core::llm_driver_registry::DriverRegistry;
use everruns_core::memory::{
    InMemoryAgentStore, InMemoryEventEmitter, InMemoryHarnessStore, InMemoryLlmProviderStore,
    InMemoryMemoryStore, InMemoryMessageRetriever,
};
use everruns_core::memory_store::MemoryStoreBackend;
use everruns_core::traits::{
    AgentStore, EventEmitter, HarnessStore, LlmProviderStore, SessionFileStore, SessionMutator,
    SessionStore,
};
use everruns_core::typed_id::{AgentId, HarnessId, MessageId, SessionId, TurnId};
use everruns_core::{
    Agent, AgentCapabilityConfig, AgentStatus, CapabilityRegistry, EventData, Harness,
    HarnessStatus, InputMessage, LlmProviderType, ModelWithProvider, Session, SessionStatus, Tool,
    ToolCall, ToolExecutionResult, ToolRegistry, ToolResult, inspect_turn_context,
    user_facing_error_codes,
};
use everruns_runtime::{
    InMemorySessionFileStore, RuntimeHostAdapter, RuntimeHostTurnContext, RuntimeSessionLifecycle,
    RuntimeTurnPlan, RuntimeTurnState, execute_act_activity, execute_input_activity,
    plan_next_host_turn,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Clone, Default)]
struct TestSessionStore {
    sessions: Arc<RwLock<HashMap<SessionId, Session>>>,
}

impl TestSessionStore {
    async fn insert(&self, session: Session) {
        self.sessions.write().await.insert(session.id, session);
    }

    async fn set_status(&self, session_id: SessionId, status: SessionStatus) -> Session {
        let mut sessions = self.sessions.write().await;
        let session = sessions.get_mut(&session_id).expect("session exists");
        session.status = status;
        session.updated_at = Utc::now();
        session.clone()
    }
}

#[async_trait]
impl SessionStore for TestSessionStore {
    async fn get_session(
        &self,
        session_id: SessionId,
    ) -> everruns_core::error::Result<Option<Session>> {
        Ok(self.sessions.read().await.get(&session_id).cloned())
    }
}

#[async_trait]
impl SessionMutator for TestSessionStore {
    async fn update_session_title(
        &self,
        session_id: SessionId,
        title: String,
    ) -> everruns_core::error::Result<Session> {
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
    provider_store: Arc<InMemoryLlmProviderStore>,
    event_emitter: Arc<InMemoryEventEmitter>,
    file_store: Arc<InMemorySessionFileStore>,
    memory_store: Option<Arc<dyn MemoryStoreBackend>>,
}

#[async_trait]
impl RuntimeHostAdapter for MockHostAdapter {
    async fn get_agent(
        &self,
        _org_id: i64,
        agent_id: AgentId,
    ) -> everruns_core::error::Result<Option<Agent>> {
        self.agent_store.get_agent(agent_id).await
    }

    async fn get_harness(
        &self,
        _org_id: i64,
        harness_id: HarnessId,
    ) -> everruns_core::error::Result<Option<Harness>> {
        Ok(self
            .harness_store
            .get_harness_chain(harness_id)
            .await?
            .into_iter()
            .last())
    }

    async fn set_session_status(
        &self,
        _org_id: i64,
        session_id: SessionId,
        status: SessionStatus,
    ) -> everruns_core::error::Result<Session> {
        Ok(self.session_store.set_status(session_id, status).await)
    }

    async fn load_turn_context(
        &self,
        _org_id: i64,
        session_id: SessionId,
    ) -> everruns_core::error::Result<RuntimeHostTurnContext> {
        Ok(RuntimeHostTurnContext {
            agent: None,
            session: self
                .session_store
                .get_session(session_id)
                .await?
                .expect("session exists"),
            messages: self.message_store.load(session_id).await?,
            model: None,
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

    fn provider_store(&self, _org_id: i64) -> Arc<dyn LlmProviderStore> {
        self.provider_store.clone()
    }

    fn message_store(&self) -> Arc<dyn everruns_core::MessageRetriever> {
        self.message_store.clone()
    }

    fn event_emitter(&self) -> Arc<dyn EventEmitter> {
        self.event_emitter.clone()
    }

    fn file_store(&self) -> Arc<dyn SessionFileStore> {
        self.file_store.clone()
    }

    fn memory_store(&self, _org_id: i64) -> Option<Arc<dyn MemoryStoreBackend>> {
        self.memory_store.clone()
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

fn harness(harness_id: HarnessId) -> Harness {
    Harness {
        id: harness_id,
        name: "math".into(),
        display_name: Some("Math".into()),
        description: None,
        system_prompt: "You are a math harness.".into(),
        parent_harness_id: None,
        default_model_id: None,
        tags: vec![],
        capabilities: vec![AgentCapabilityConfig::new("test_math")],
        initial_files: vec![],
        network_access: None,
        mcp_servers: Default::default(),
        is_built_in: false,
        status: HarnessStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        archived_at: None,
        deleted_at: None,
    }
}

fn session(session_id: SessionId, harness_id: HarnessId) -> Session {
    Session {
        id: session_id,
        organization_id: everruns_core::DEFAULT_ORG_PUBLIC_ID.to_string(),
        harness_id,
        agent_id: None,
        agent_identity_id: None,
        owner_principal_id: everruns_core::PrincipalId::from_seed(1),
        resolved_owner_user_id: None,
        owner: None,
        effective_owner: None,
        title: Some("Runtime Host".into()),
        locale: None,
        preview: None,
        output_preview: None,
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
        status: SessionStatus::Started,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        started_at: None,
        finished_at: None,
        usage: None,
        is_pinned: None,
        active_schedule_count: None,
        features: vec![],
        parent_session_id: None,
        subagent_name: None,
        subagent_task: None,
        subagent_status: None,
        blueprint_id: None,
        blueprint_config: None,
    }
}

fn agent(agent_id: AgentId, capabilities: Vec<AgentCapabilityConfig>) -> Agent {
    Agent {
        public_id: agent_id,
        internal_id: Uuid::nil(),
        name: "test-agent".into(),
        display_name: Some("Test Agent".into()),
        description: None,
        system_prompt: "Use tools when needed.".into(),
        default_model_id: None,
        tags: vec![],
        capabilities,
        initial_files: vec![],
        network_access: None,
        max_iterations: Some(8),
        tools: vec![],
        mcp_servers: Default::default(),
        status: AgentStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        archived_at: None,
        deleted_at: None,
        usage: None,
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
        provider_store: Arc::new(InMemoryLlmProviderStore::new()),
        event_emitter: Arc::new(InMemoryEventEmitter::new()),
        file_store: Arc::new(InMemorySessionFileStore::new()),
        memory_store: None,
    }
}

async fn set_default_model(adapter: &MockHostAdapter) {
    adapter
        .provider_store
        .set_default_model(ModelWithProvider {
            model: "llmsim-model".into(),
            provider_type: LlmProviderType::LlmSim,
            api_key: None,
            base_url: None,
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
    adapter.harness_store.add_harness(harness(harness_id)).await;
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
    assert_eq!(session.status, SessionStatus::Active);

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
    adapter.harness_store.add_harness(harness(harness_id)).await;
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
        },
    )
    .await
    .unwrap();

    assert_eq!(result.success_count, 1);
    assert_eq!(result.error_count, 0);
    assert_eq!(result.results.len(), 1);
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
        .add_harness(Harness {
            capabilities: vec![AgentCapabilityConfig::new("overlay_echo")],
            ..harness(harness_id)
        })
        .await;
    adapter.agent_store.add_agent(agent(agent_id, vec![])).await;
    adapter
        .session_store
        .insert(Session {
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
        .add_harness(Harness {
            capabilities: vec![AgentCapabilityConfig::new("overlay_alias")],
            ..harness(harness_id)
        })
        .await;
    adapter.agent_store.add_agent(agent(agent_id, vec![])).await;
    adapter
        .session_store
        .insert(Session {
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
        .add_harness(Harness {
            capabilities: vec![AgentCapabilityConfig::new("overlay_echo")],
            ..harness(harness_id)
        })
        .await;
    adapter.agent_store.add_agent(agent(agent_id, vec![])).await;
    adapter
        .session_store
        .insert(Session {
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
async fn act_activity_passes_public_org_id_to_memory_tools() {
    let mut adapter = mock_host();
    adapter.capability_registry.register(MemoryCapability);
    let memory_store = Arc::new(InMemoryMemoryStore::new());
    adapter.memory_store = Some(memory_store.clone());

    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    let input_message_id = MessageId::from_uuid(Uuid::now_v7());
    adapter
        .harness_store
        .add_harness(Harness {
            capabilities: vec![AgentCapabilityConfig::new("memory")],
            ..harness(harness_id)
        })
        .await;
    adapter
        .session_store
        .insert(session(session_id, harness_id))
        .await;

    let result = execute_act_activity(
        &adapter,
        ActInput {
            org_id: Some(42),
            context: AtomContext::new(
                session_id,
                TurnId::from_uuid(Uuid::now_v7()),
                input_message_id,
            ),
            harness_id,
            agent_id: None,
            tool_calls: vec![ToolCall {
                id: "call_remember".into(),
                name: "remember".into(),
                arguments: serde_json::json!({
                    "content": "Runtime host should pass org id to memory tools",
                    "kind": "fact",
                    "importance": 6,
                    "tags": ["runtime"]
                }),
            }],
            tool_definitions: build_registry(
                &adapter.capability_registry,
                session_id,
                &[AgentCapabilityConfig::new("memory")],
            )
            .await
            .unwrap()
            .tool_definitions(),
            locale: None,
            blueprint_id: None,
            network_access: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(result.success_count, 1);
    assert_eq!(result.error_count, 0);

    let store = memory_store
        .get_or_create_default_store(
            everruns_core::org_public_id_from_internal(42)
                .parse()
                .expect("valid public org id"),
        )
        .await
        .unwrap();
    assert_eq!(memory_store.count_active(store.id).await.unwrap(), 1);
}

#[tokio::test]
async fn lifecycle_helper_sets_waiting_for_tool_results_status() {
    let adapter = mock_host();
    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    adapter.harness_store.add_harness(harness(harness_id)).await;
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
    assert_eq!(session.status, SessionStatus::WaitingForToolResults);
}

#[tokio::test]
async fn plan_next_host_turn_schedules_reason_after_process_input() {
    let adapter = mock_host();
    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    adapter.harness_store.add_harness(harness(harness_id)).await;
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
    adapter.harness_store.add_harness(harness(harness_id)).await;
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
        usage: None,
        response_id: Some("resp_123".into()),
        locale: Some("en-US".into()),
        network_access: None,
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
        }
        other => panic!("expected ScheduleAct, got {other:?}"),
    }
}

#[tokio::test]
async fn plan_next_host_turn_schedules_act_with_session_blueprint_id() {
    let adapter = mock_host();
    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    adapter.harness_store.add_harness(harness(harness_id)).await;
    adapter
        .session_store
        .insert(Session {
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
        usage: None,
        response_id: Some("resp_blueprint".into()),
        locale: Some("en-US".into()),
        network_access: None,
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
    adapter.harness_store.add_harness(harness(harness_id)).await;
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
        usage: None,
        response_id: Some("resp_steer".into()),
        locale: None,
        network_access: None,
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
async fn plan_next_host_turn_preserves_reason_failure_message() {
    let adapter = mock_host();
    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    adapter.harness_store.add_harness(harness(harness_id)).await;
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
        usage: None,
        response_id: None,
        locale: None,
        network_access: None,
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
    adapter.harness_store.add_harness(harness(harness_id)).await;
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
        usage: None,
        response_id: None,
        locale: None,
        network_access: None,
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
async fn plan_next_host_turn_waits_for_tool_results_when_session_hint_requests_it() {
    let adapter = mock_host();
    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    adapter.harness_store.add_harness(harness(harness_id)).await;

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
            assert_eq!(session.status, SessionStatus::WaitingForToolResults);
        }
        other => panic!("expected WaitForToolResults, got {other:?}"),
    }
}
