use chrono::Utc;
use everruns_core::capabilities::TestMathCapability;
use everruns_core::llm_driver_registry::DriverRegistry;
use everruns_core::llmsim_driver::LlmSimConfig;
use everruns_core::memory::{
    InMemoryAgentStore, InMemoryEventEmitter, InMemoryHarnessStore, InMemoryLlmProviderStore,
    InMemoryMemoryStore, InMemoryMessageRetriever,
};
use everruns_core::{
    Agent, AgentCapabilityConfig, AgentStatus, CapabilityRegistry, Harness, HarnessStatus,
    LlmProviderType, MessageRole, ModelWithProvider, PlatformDefinition, Session, SessionStatus,
    ToolCall,
};
use everruns_runtime::{
    InMemorySessionFileStore, InMemorySessionStorageStore, InMemorySessionStore,
    InProcessRuntimeBuilder, RuntimeBackends,
};
use std::sync::Arc;
use uuid::Uuid;

fn minimal_platform() -> PlatformDefinition {
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(TestMathCapability);
    PlatformDefinition::new(capabilities, DriverRegistry::new())
}

fn harness(harness_id: everruns_core::HarnessId) -> Harness {
    Harness {
        id: harness_id,
        name: "math".into(),
        display_name: Some("Math".into()),
        description: None,
        system_prompt: "You are a math assistant.".into(),
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

fn agent(agent_id: everruns_core::AgentId) -> Agent {
    Agent {
        public_id: agent_id,
        internal_id: Uuid::nil(),
        name: "math-agent".into(),
        display_name: Some("Math Agent".into()),
        description: None,
        system_prompt: "Use tools when needed.".into(),
        default_model_id: None,
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![],
        network_access: None,
        max_iterations: Some(8),
        tools: vec![],
        status: AgentStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        archived_at: None,
        deleted_at: None,
        usage: None,
    }
}

fn session(
    session_id: everruns_core::SessionId,
    harness_id: everruns_core::HarnessId,
    agent_id: Option<everruns_core::AgentId>,
) -> Session {
    Session {
        id: session_id,
        organization_id: everruns_core::DEFAULT_ORG_PUBLIC_ID.to_string(),
        harness_id,
        agent_id,
        agent_identity_id: None,
        title: Some("Embedded Session".into()),
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

#[tokio::test]
async fn runtime_executes_tool_loop_and_persists_messages() {
    let harness_id = "harness_00000000000000000000000000000021".parse().unwrap();
    let agent_id = "agent_00000000000000000000000000000021".parse().unwrap();
    let session_id = "session_00000000000000000000000000000021".parse().unwrap();

    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(minimal_platform())
        .llm_sim(
            LlmSimConfig::fixed("Let me calculate that.").with_tool_call_sequence(vec![
                vec![ToolCall {
                    id: "call_mul_1".into(),
                    name: "multiply".into(),
                    arguments: serde_json::json!({"a": 6, "b": 7}),
                }],
                vec![],
            ]),
        )
        .default_model(ModelWithProvider {
            model: "llmsim-model".into(),
            provider_type: LlmProviderType::LlmSim,
            api_key: Some("fake-key".into()),
            base_url: None,
        })
        .harness(harness(harness_id))
        .agent(agent(agent_id))
        .session(session(session_id, harness_id, Some(agent_id)))
        .build()
        .await
        .unwrap();

    let result = runtime
        .run_text_turn(session_id, "What is 6 * 7?")
        .await
        .unwrap();
    assert!(result.success);

    let messages = runtime.messages(session_id).await.unwrap();
    assert_eq!(
        messages.len(),
        4,
        "user + assistant(tool call) + tool result + assistant"
    );
    assert_eq!(messages[0].role, MessageRole::User);
    assert!(
        messages[1].has_tool_calls(),
        "assistant tool call must be persisted"
    );
    assert_eq!(messages[2].role, MessageRole::ToolResult);
    assert_eq!(messages[2].tool_call_id(), Some("call_mul_1"));
    assert_eq!(messages[3].role, MessageRole::Agent);

    let event_types: Vec<_> = runtime
        .events()
        .await
        .unwrap()
        .into_iter()
        .map(|event| event.data.event_type().to_string())
        .collect();
    assert!(
        event_types
            .iter()
            .any(|event_type| event_type == "input.message"),
        "run_turn should emit the same input event shape as the API path",
    );
    assert!(
        event_types
            .iter()
            .any(|event_type| event_type == "tool.completed"),
        "tool execution events must be captured for embedders",
    );
}

#[tokio::test]
async fn runtime_seeds_initial_files_from_harness_chain() {
    let harness_id = "harness_00000000000000000000000000000031".parse().unwrap();
    let agent_id = "agent_00000000000000000000000000000031".parse().unwrap();
    let session_id = "session_00000000000000000000000000000031".parse().unwrap();

    let mut math_harness = harness(harness_id);
    math_harness
        .initial_files
        .push(everruns_core::session_file::InitialFile {
            path: "/workspace/notes.txt".into(),
            content: "hello embedded runtime".into(),
            encoding: "text".into(),
            is_readonly: true,
        });

    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(minimal_platform())
        .llm_sim(LlmSimConfig::fixed("No-op"))
        .default_model(ModelWithProvider {
            model: "llmsim-model".into(),
            provider_type: LlmProviderType::LlmSim,
            api_key: Some("fake-key".into()),
            base_url: None,
        })
        .harness(math_harness)
        .agent(agent(agent_id))
        .session(session(session_id, harness_id, Some(agent_id)))
        .build()
        .await
        .unwrap();

    let file = runtime
        .read_file(session_id, "/workspace/notes.txt")
        .await
        .unwrap()
        .expect("seeded file");

    assert_eq!(file.content.as_deref(), Some("hello embedded runtime"));
    assert!(file.is_readonly);
}

#[tokio::test]
async fn runtime_runs_session_without_agent_entity() {
    let harness_id = "harness_00000000000000000000000000000041".parse().unwrap();
    let session_id = "session_00000000000000000000000000000041".parse().unwrap();

    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(minimal_platform())
        .llm_sim(LlmSimConfig::fixed("Harness-only runtime works"))
        .default_model(ModelWithProvider {
            model: "llmsim-model".into(),
            provider_type: LlmProviderType::LlmSim,
            api_key: Some("fake-key".into()),
            base_url: None,
        })
        .harness(harness(harness_id))
        .session(session(session_id, harness_id, None))
        .build()
        .await
        .unwrap();

    let result = runtime
        .run_text_turn(session_id, "Say hello from the harness")
        .await
        .unwrap();

    assert!(result.success);
    assert_eq!(result.response, "Harness-only runtime works");

    let messages = runtime.messages(session_id).await.unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, MessageRole::User);
    assert_eq!(messages[1].role, MessageRole::Agent);
}

#[tokio::test]
async fn runtime_accepts_explicit_backend_bundle() {
    let harness_id = "harness_00000000000000000000000000000051".parse().unwrap();
    let agent_id = "agent_00000000000000000000000000000051".parse().unwrap();
    let session_id = "session_00000000000000000000000000000051".parse().unwrap();

    let event_emitter = InMemoryEventEmitter::new();
    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(minimal_platform())
        .backends(RuntimeBackends {
            harness_store: Arc::new(InMemoryHarnessStore::new()),
            agent_store: Arc::new(InMemoryAgentStore::new()),
            session_store: Arc::new(InMemorySessionStore::new()),
            message_store: Arc::new(InMemoryMessageRetriever::new()),
            provider_store: Arc::new(InMemoryLlmProviderStore::new()),
            event_emitter: Arc::new(event_emitter.clone()),
            event_collector: Some(Arc::new(event_emitter)),
            file_store: Arc::new(InMemorySessionFileStore::new()),
            storage_store: Arc::new(InMemorySessionStorageStore::new()),
            memory_store: Arc::new(InMemoryMemoryStore::new()),
        })
        .llm_sim(LlmSimConfig::fixed("Custom backend bundle works"))
        .default_model(ModelWithProvider {
            model: "llmsim-model".into(),
            provider_type: LlmProviderType::LlmSim,
            api_key: Some("fake-key".into()),
            base_url: None,
        })
        .harness(harness(harness_id))
        .agent(agent(agent_id))
        .session(session(session_id, harness_id, Some(agent_id)))
        .build()
        .await
        .unwrap();

    let result = runtime
        .run_text_turn(session_id, "Use the explicit backend bundle")
        .await
        .unwrap();

    assert!(result.success);
    assert_eq!(result.response, "Custom backend bundle works");
}
