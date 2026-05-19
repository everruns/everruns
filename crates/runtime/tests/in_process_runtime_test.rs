use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use everruns_core::capabilities::TestMathCapability;
use everruns_core::llm_driver_registry::DriverRegistry;
use everruns_core::llmsim_driver::LlmSimConfig;
use everruns_core::{
    Agent, CapabilityRegistry, Harness, InitialFile, LlmProviderType, MessageRole,
    ModelWithProvider, PlatformDefinition, Session, SessionFileSystem, SessionFileSystemFactory,
    SessionFileSystemFactoryContext, ToolCall,
};
use everruns_runtime::{
    AgentBuilder, HarnessBuilder, InProcessRuntimeBuilder, RealDiskFileStore, RuntimeBackends,
    SessionBuilder,
};
use std::path::PathBuf;
use std::sync::Arc;

fn minimal_platform() -> PlatformDefinition {
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(TestMathCapability);
    PlatformDefinition::new(capabilities, DriverRegistry::new())
}

fn harness(harness_id: everruns_core::HarnessId) -> Harness {
    HarnessBuilder::new("math", "You are a math assistant.")
        .id(harness_id)
        .display_name("Math")
        .capability("test_math")
        .build()
}

fn agent(agent_id: everruns_core::AgentId) -> Agent {
    AgentBuilder::new("math-agent", "Use tools when needed.")
        .id(agent_id)
        .display_name("Math Agent")
        .max_iterations(8)
        .build()
}

fn session(
    session_id: everruns_core::SessionId,
    harness_id: everruns_core::HarnessId,
    agent_id: Option<everruns_core::AgentId>,
) -> Session {
    let builder = SessionBuilder::new(harness_id)
        .id(session_id)
        .title("Embedded Session");
    match agent_id {
        Some(agent_id) => builder.agent(agent_id).build(),
        None => builder.build(),
    }
}

#[derive(Debug)]
struct ContextRealDiskFactory;

#[async_trait]
impl SessionFileSystemFactory for ContextRealDiskFactory {
    fn name(&self) -> &'static str {
        "ContextRealDiskFactory"
    }

    async fn create_session_file_system(
        &self,
        context: SessionFileSystemFactoryContext,
    ) -> everruns_core::Result<Arc<dyn SessionFileSystem>> {
        let root = context
            .get::<PathBuf>()
            .ok_or_else(|| everruns_core::AgentLoopError::config("missing real-disk root"))?;
        Ok(Arc::new(RealDiskFileStore::new(root.as_path())?))
    }
}

#[test]
fn per_type_builders_accept_explicit_timestamps() {
    let timestamp = Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap();
    let harness_id = everruns_core::HarnessId::from_seed(51);
    let agent_id = everruns_core::AgentId::from_seed(51);
    let session_id = everruns_core::SessionId::from_seed(51);

    let harness = HarnessBuilder::new("math", "prompt")
        .id(harness_id)
        .created_at(timestamp)
        .updated_at(timestamp)
        .build();
    let agent = AgentBuilder::new("math-agent", "prompt")
        .id(agent_id)
        .created_at(timestamp)
        .updated_at(timestamp)
        .build();
    let session = SessionBuilder::new(harness_id)
        .id(session_id)
        .agent(agent_id)
        .created_at(timestamp)
        .updated_at(timestamp)
        .build();

    assert_eq!(harness.created_at, timestamp);
    assert_eq!(harness.updated_at, timestamp);
    assert_eq!(agent.created_at, timestamp);
    assert_eq!(agent.updated_at, timestamp);
    assert_eq!(session.created_at, timestamp);
    assert_eq!(session.updated_at, timestamp);
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
async fn single_session_builder_seeds_runnable_runtime() {
    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(minimal_platform())
        .llm_sim(LlmSimConfig::fixed("single session works"))
        .single_session(|s| {
            s.harness("math", "You are a math assistant.")
                .with_capability("test_math")
                .agent("math-agent", "Use tools when needed.")
                .agent_max_iterations(8)
                .session_title("Embedded Session")
        })
        .build()
        .await
        .unwrap();

    let session_id = runtime.default_session_id().expect("default session id");
    let context = runtime.load_context(session_id).await.unwrap();

    assert_eq!(context.harness_chain.last().expect("harness").name, "math");
    assert_eq!(
        context.agent.expect("agent").system_prompt,
        "Use tools when needed."
    );

    let result = runtime
        .run_text_turn(session_id, "Say this is working.")
        .await
        .unwrap();
    assert!(result.success);
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

    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(minimal_platform())
        .backends(RuntimeBackends::in_memory())
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

#[tokio::test]
async fn runtime_uses_platform_session_file_system_factory() {
    let harness_id = "harness_00000000000000000000000000000053".parse().unwrap();
    let session_id = "session_00000000000000000000000000000053".parse().unwrap();
    let tempdir = tempfile::tempdir().unwrap();

    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(TestMathCapability);
    let platform = PlatformDefinition::builder()
        .capability_registry(capabilities)
        .driver_registry(DriverRegistry::new())
        .session_file_system_factory(Arc::new(ContextRealDiskFactory))
        .build();

    let mut harness = harness(harness_id);
    harness.initial_files = vec![InitialFile {
        path: "/seed.txt".into(),
        content: "from platform factory".into(),
        encoding: "text".into(),
        is_readonly: false,
    }];

    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(platform)
        .session_file_system_factory_context(
            SessionFileSystemFactoryContext::new().with(Arc::new(tempdir.path().to_path_buf())),
        )
        .llm_sim(LlmSimConfig::fixed("ok"))
        .harness(harness)
        .session(session(session_id, harness_id, None))
        .build()
        .await
        .unwrap();

    let file = runtime
        .read_file(session_id, "/seed.txt")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(file.content.as_deref(), Some("from platform factory"));
    assert!(tempdir.path().join("seed.txt").exists());
}

#[tokio::test]
async fn runtime_exposes_assembled_context() {
    let harness_id = "harness_00000000000000000000000000000061".parse().unwrap();
    let agent_id = "agent_00000000000000000000000000000061".parse().unwrap();
    let session_id = "session_00000000000000000000000000000061".parse().unwrap();

    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(minimal_platform())
        .llm_sim(LlmSimConfig::fixed("Context inspection"))
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

    let initial_context = runtime.load_context(session_id).await.unwrap();
    assert!(initial_context.messages.is_empty());
    assert_eq!(initial_context.session.id, session_id);
    assert_eq!(
        initial_context.agent.as_ref().map(|agent| agent.public_id),
        Some(agent_id)
    );

    runtime
        .run_text_turn(session_id, "What locale and tools do I have?")
        .await
        .unwrap();

    let context = runtime.load_context(session_id).await.unwrap();

    assert_eq!(context.session.id, session_id);
    assert_eq!(context.harness_chain.len(), 1);
    assert_eq!(context.messages.len(), 2);
    assert_eq!(context.model_with_provider.model, "llmsim-model");
    assert!(
        context
            .runtime_agent
            .tools
            .iter()
            .any(|tool| tool.name() == "multiply"),
        "assembled context should expose effective capability tools",
    );
}
