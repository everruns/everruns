use async_trait::async_trait;
use chrono::Utc;
use everruns_core::MessageRetriever;
use everruns_core::atoms::{ActInput, AtomContext, InputAtomInput};
use everruns_core::capabilities::{
    MemoryCapability, SystemPromptContext, TestMathCapability, collect_capabilities_with_configs,
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
    Agent, AgentCapabilityConfig, CapabilityRegistry, Harness, HarnessStatus, InputMessage,
    Session, SessionStatus, ToolCall, ToolRegistry,
};
use everruns_runtime::{
    InMemorySessionFileStore, RuntimeHostAdapter, RuntimeHostTurnContext, RuntimeSessionLifecycle,
    execute_act_activity, execute_input_activity,
};
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

    async fn build_tool_registry_for_agent(
        &self,
        _org_id: i64,
        agent_id: AgentId,
    ) -> everruns_core::error::Result<ToolRegistry> {
        let agent = self
            .agent_store
            .get_agent(agent_id)
            .await?
            .expect("agent exists");
        build_registry(
            &self.capability_registry,
            SessionId::new(),
            &agent.capabilities,
        )
        .await
    }

    async fn build_tool_registry_for_harness(
        &self,
        _org_id: i64,
        harness_id: HarnessId,
    ) -> everruns_core::error::Result<ToolRegistry> {
        let harness = self
            .harness_store
            .get_harness_chain(harness_id)
            .await?
            .into_iter()
            .last()
            .expect("harness exists");
        build_registry(
            &self.capability_registry,
            SessionId::new(),
            &harness.capabilities,
        )
        .await
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

    fn memory_store(&self) -> Option<Arc<dyn MemoryStoreBackend>> {
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
        title: Some("Runtime Host".into()),
        locale: None,
        preview: None,
        output_preview: None,
        tags: vec![],
        model_id: None,
        capabilities: vec![],
        tools: vec![],
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
    assert!(event_types.contains(&"session.activated".to_string()));
    assert!(event_types.contains(&"turn.started".to_string()));
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
