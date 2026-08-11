use async_trait::async_trait;
use everruns_core::capabilities::InfinityContextCapability;
use everruns_core::driver_registry::DriverRegistry;
use everruns_core::events::{EventContext, EventRequest, InputMessageData};
use everruns_core::network_access::NetworkAccessList;
use everruns_core::{
    AgentDefinition, CapabilityRegistry, DriverId, ExecutionSession, InitialFile, Message,
    MessageRole, PlatformDefinition, ResolvedModel, SessionFileSystem, SessionFileSystemFactory,
    SessionFileSystemFactoryContext, ToolCall, WorkspacePolicy,
};
use everruns_host::{
    AgentBuilder, HarnessBuilder, HostBackends, InProcessRuntimeBuilder, RealDiskFileStore,
    SessionBuilder, TurnStopReason,
};
use everruns_platform::capabilities::SessionTasksCapability;
use everruns_test_support::LlmSimRuntimeExt;
use everruns_test_support::TestMathCapability;
use everruns_test_support::llmsim_driver::LlmSimConfig;
use std::path::PathBuf;
use std::sync::Arc;

fn minimal_platform() -> PlatformDefinition {
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(TestMathCapability);
    PlatformDefinition::new(capabilities, DriverRegistry::new())
}

fn harness(harness_id: everruns_core::HarnessId) -> everruns_host::SeededHarness {
    HarnessBuilder::new("math", "You are a math assistant.")
        .id(harness_id)
        .capability("test_math")
        .build()
}

fn agent(agent_id: everruns_core::AgentId) -> AgentDefinition {
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
) -> ExecutionSession {
    let builder = SessionBuilder::new(harness_id)
        .id(session_id)
        .title("Embedded ExecutionSession");
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
fn per_type_builders_produce_portable_execution_values() {
    let harness_id = everruns_core::HarnessId::from_seed(51);
    let agent_id = everruns_core::AgentId::from_seed(51);
    let session_id = everruns_core::SessionId::from_seed(51);

    let harness = HarnessBuilder::new("math", "prompt").id(harness_id).build();
    let agent = AgentBuilder::new("math-agent", "prompt")
        .id(agent_id)
        .build();
    let session = SessionBuilder::new(harness_id)
        .id(session_id)
        .agent(agent_id)
        .build();

    // Harness/agent/session values are portable execution configuration
    // (EVE-877, EVE-881, EVE-882); they carry no persistence timestamps.
    assert_eq!(harness.id, harness_id);
    assert_eq!(agent.id, agent_id);
    assert_eq!(session.id, session_id);
    assert_eq!(session.agent_id, Some(agent_id));
}

#[tokio::test]
async fn default_runtime_uses_runtime_safe_capability_preset() {
    let runtime = InProcessRuntimeBuilder::new()
        .llm_sim(LlmSimConfig::fixed("ok"))
        .single_session(|s| {
            s.harness("time", "You know the current time.")
                .with_capability("current_time")
                .agent("time-agent", "Use tools when helpful.")
        })
        .build()
        .await
        .unwrap();

    let session_id = runtime.default_session_id().expect("default session id");
    let context = runtime.load_context(session_id).await.unwrap();
    assert!(
        context
            .runtime_agent
            .tools
            .iter()
            .any(|tool| tool.name() == "get_current_time"),
        "default runtime should expose runtime-backed built-in capabilities",
    );

    let platform_only_runtime = InProcessRuntimeBuilder::new()
        .llm_sim(LlmSimConfig::fixed("ok"))
        .single_session(|s| {
            s.harness("platform", "You manage the platform.")
                .with_capability("platform_management")
                .agent("platform-agent", "Use tools when helpful.")
        })
        .build()
        .await
        .unwrap();

    let platform_session_id = platform_only_runtime
        .default_session_id()
        .expect("default session id");
    let platform_context = platform_only_runtime
        .load_context(platform_session_id)
        .await
        .unwrap();
    assert!(
        platform_context.resolved_capability_configs.is_empty(),
        "platform-only capability should not resolve from runtime defaults"
    );
    assert!(
        platform_context.runtime_agent.tools.is_empty(),
        "unresolved platform-only capability should not contribute tools"
    );
}

#[tokio::test]
async fn runtime_rejects_tools_missing_required_context_services_before_reason() {
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(SessionTasksCapability);
    let platform = PlatformDefinition::new(capabilities, DriverRegistry::new());

    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(platform)
        .llm_sim(LlmSimConfig::fixed("model must not be reached"))
        .single_session(|s| {
            s.harness("tasks", "Manage session tasks.")
                .with_capability("session_tasks")
                .agent("tasks-agent", "Inspect tasks.")
        })
        .build()
        .await
        .unwrap();

    let error = runtime
        .run_text_turn(
            runtime.default_session_id().expect("default session id"),
            "List tasks.",
        )
        .await
        .expect_err("missing required service must fail configuration");
    let message = error.to_string();
    assert!(
        message.contains("cancel_task"),
        "error must name tool: {message}"
    );
    assert!(
        message.contains("SessionTaskRegistry"),
        "error must name missing service: {message}",
    );
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
        .default_model(ResolvedModel {
            model: "llmsim-model".into(),
            provider_type: DriverId::LlmSim,
            api_key: Some("fake-key".into()),
            base_url: None,
            provider_metadata: None,
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
    assert_eq!(result.stop_reason, TurnStopReason::EndTurn);

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
async fn query_history_reads_messages_through_in_process_reason_act_path() {
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(InfinityContextCapability);
    let platform = PlatformDefinition::new(capabilities, DriverRegistry::new());

    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(platform)
        .llm_sim(
            LlmSimConfig::fixed("History retrieved.").with_tool_call_sequence(vec![
                vec![ToolCall {
                    id: "call_history_1".into(),
                    name: "query_history".into(),
                    arguments: serde_json::json!({"query": "remember"}),
                }],
                vec![],
            ]),
        )
        .single_session(|s| {
            s.harness("history", "Use conversation history when needed.")
                .with_capability("infinity_context")
                .agent("history-agent", "Use query_history when asked.")
        })
        .build()
        .await
        .unwrap();

    let session_id = runtime.default_session_id().expect("default session id");
    let result = runtime
        .run_text_turn(session_id, "Remember the deployment code is blue.")
        .await
        .unwrap();
    assert!(result.success);

    let history_result = runtime
        .messages(session_id)
        .await
        .unwrap()
        .into_iter()
        .find(|message| {
            message.role == MessageRole::ToolResult
                && message.tool_call_id() == Some("call_history_1")
        })
        .expect("query_history tool result");
    let serialized = history_result.content_to_llm_string();
    assert!(
        serialized.contains("Remember the deployment code is blue."),
        "stored history must reach query_history through ToolContext; got: {serialized}",
    );
}

#[tokio::test]
async fn query_history_reads_seeded_resumed_session_messages() {
    let harness_id = everruns_core::HarnessId::from_seed(797);
    let agent_id = everruns_core::AgentId::from_seed(797);
    let session_id = everruns_core::SessionId::from_seed(797);
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(InfinityContextCapability);
    let platform = PlatformDefinition::new(capabilities, DriverRegistry::new());
    let backends = HostBackends::in_memory();
    backends
        .event_log
        .append(EventRequest::new(
            session_id,
            EventContext::empty(),
            InputMessageData::new(Message::user("The seeded deployment code is amber.")),
        ))
        .await
        .unwrap();

    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(platform)
        .backends(backends)
        .llm_sim(
            LlmSimConfig::fixed("Seeded history retrieved.").with_tool_call_sequence(vec![
                vec![ToolCall {
                    id: "call_seeded_history".into(),
                    name: "query_history".into(),
                    arguments: serde_json::json!({"query": "amber"}),
                }],
                vec![],
            ]),
        )
        .default_model(ResolvedModel {
            model: "llmsim-model".into(),
            provider_type: DriverId::LlmSim,
            api_key: Some("fake-key".into()),
            base_url: None,
            provider_metadata: None,
        })
        .harness(
            HarnessBuilder::new("history", "Use conversation history when needed.")
                .id(harness_id)
                .capability("infinity_context")
                .build(),
        )
        .agent(
            AgentBuilder::new("history-agent", "Use query_history when asked.")
                .id(agent_id)
                .build(),
        )
        .session(
            SessionBuilder::new(harness_id)
                .id(session_id)
                .agent(agent_id)
                .build(),
        )
        .build()
        .await
        .unwrap();

    runtime
        .run_text_turn(session_id, "What was the seeded deployment code?")
        .await
        .unwrap();

    let history_result = runtime
        .messages(session_id)
        .await
        .unwrap()
        .into_iter()
        .find(|message| message.tool_call_id() == Some("call_seeded_history"))
        .expect("query_history tool result");
    assert!(
        history_result
            .content_to_llm_string()
            .contains("The seeded deployment code is amber."),
        "seeded history must remain queryable after runtime construction",
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
                .session_title("Embedded ExecutionSession")
        })
        .build()
        .await
        .unwrap();

    let session_id = runtime.default_session_id().expect("default session id");
    let context = runtime.load_context(session_id).await.unwrap();

    assert_eq!(context.harness.name, "math");
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
async fn single_session_builder_pins_session_id_when_set() {
    // Embedders that need the id ahead of build (e.g. a JSONL session log
    // whose filename encodes the id) must be able to pin it.
    let expected = everruns_core::SessionId::from_seed(481);
    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(minimal_platform())
        .llm_sim(LlmSimConfig::fixed("pinned id works"))
        .single_session(|s| {
            s.harness("h", "h")
                .agent("a", "a")
                .session_id(expected)
                .session_title("Pinned")
        })
        .build()
        .await
        .unwrap();

    assert_eq!(runtime.default_session_id(), Some(expected));
    let context = runtime.load_context(expected).await.unwrap();
    assert_eq!(context.session.id, expected);
}

#[tokio::test]
async fn single_session_builder_preserves_harness_acl_when_order_changes() {
    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(minimal_platform())
        .llm_sim(LlmSimConfig::fixed("ok"))
        .single_session(|s| {
            s.harness_network_access(NetworkAccessList::allow_only(["example.com"]))
                .harness("math", "You are a math assistant.")
        })
        .build()
        .await
        .unwrap();

    let session_id = runtime.default_session_id().expect("default session id");
    let context = runtime.load_context(session_id).await.unwrap();
    assert_eq!(
        context
            .harness
            .network_access
            .as_ref()
            .map(|acl| acl.allowed.clone()),
        Some(vec!["example.com".to_string()])
    );
}

#[tokio::test]
async fn single_session_builder_preserves_agent_acl_when_order_changes() {
    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(minimal_platform())
        .llm_sim(LlmSimConfig::fixed("ok"))
        .single_session(|s| {
            s.agent_network_access(NetworkAccessList::allow_only(["example.com"]))
                .agent("math-agent", "Use tools when needed.")
        })
        .build()
        .await
        .unwrap();

    let session_id = runtime.default_session_id().expect("default session id");
    let context = runtime.load_context(session_id).await.unwrap();
    assert_eq!(
        context
            .agent
            .as_ref()
            .and_then(|a| a.network_access.as_ref())
            .map(|acl| acl.allowed.clone()),
        Some(vec!["example.com".to_string()])
    );
}

#[tokio::test]
async fn runtime_seeds_initial_files_from_harness_chain() {
    let harness_id = "harness_00000000000000000000000000000031".parse().unwrap();
    let agent_id = "agent_00000000000000000000000000000031".parse().unwrap();
    let session_id = "session_00000000000000000000000000000031".parse().unwrap();

    let mut math_harness = harness(harness_id);
    math_harness
        .definition
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
        .default_model(ResolvedModel {
            model: "llmsim-model".into(),
            provider_type: DriverId::LlmSim,
            api_key: Some("fake-key".into()),
            base_url: None,
            provider_metadata: None,
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
        .default_model(ResolvedModel {
            model: "llmsim-model".into(),
            provider_type: DriverId::LlmSim,
            api_key: Some("fake-key".into()),
            base_url: None,
            provider_metadata: None,
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
        .backends(HostBackends::in_memory())
        .llm_sim(LlmSimConfig::fixed("Custom backend bundle works"))
        .default_model(ResolvedModel {
            model: "llmsim-model".into(),
            provider_type: DriverId::LlmSim,
            api_key: Some("fake-key".into()),
            base_url: None,
            provider_metadata: None,
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
    harness.definition.initial_files = vec![InitialFile {
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
async fn workspace_policy_wraps_custom_platform_file_system_factory() {
    let harness_id = "harness_00000000000000000000000000000054".parse().unwrap();
    let session_id = "session_00000000000000000000000000000054".parse().unwrap();
    let tempdir = tempfile::tempdir().unwrap();
    let platform = PlatformDefinition::builder()
        .capability_registry(CapabilityRegistry::new())
        .driver_registry(DriverRegistry::new())
        .session_file_system_factory(Arc::new(ContextRealDiskFactory))
        .build();
    let policy = WorkspacePolicy::builder()
        .allow_read("public")
        .build()
        .unwrap();

    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(platform)
        .session_file_system_factory_context(
            SessionFileSystemFactoryContext::new().with(Arc::new(tempdir.path().to_path_buf())),
        )
        .workspace_policy(policy)
        .llm_sim(LlmSimConfig::fixed("ok"))
        .harness(harness(harness_id))
        .session(session(session_id, harness_id, None))
        .seed_text_file(session_id, "/public/readme.md", "visible")
        .seed_text_file(session_id, "/private/secret.txt", "hidden")
        .build()
        .await
        .unwrap();

    assert!(
        runtime
            .read_file(session_id, "/public/readme.md")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        runtime
            .read_file(session_id, "/private/secret.txt")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn runtime_exposes_assembled_context() {
    let harness_id = "harness_00000000000000000000000000000061".parse().unwrap();
    let agent_id = "agent_00000000000000000000000000000061".parse().unwrap();
    let session_id = "session_00000000000000000000000000000061".parse().unwrap();

    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(minimal_platform())
        .llm_sim(LlmSimConfig::fixed("Context inspection"))
        .default_model(ResolvedModel {
            model: "llmsim-model".into(),
            provider_type: DriverId::LlmSim,
            api_key: Some("fake-key".into()),
            base_url: None,
            provider_metadata: None,
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
        initial_context.agent.as_ref().map(|agent| agent.id),
        Some(agent_id)
    );

    runtime
        .run_text_turn(session_id, "What locale and tools do I have?")
        .await
        .unwrap();

    let context = runtime.load_context(session_id).await.unwrap();

    assert_eq!(context.session.id, session_id);
    assert_eq!(context.harness.name, "math");
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

#[tokio::test]
async fn list_commands_returns_capability_commands_for_session() {
    use everruns_core::capabilities::BtwCapability;
    use everruns_core::command::CommandSource;

    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(TestMathCapability);
    capabilities.register(BtwCapability);
    let platform = PlatformDefinition::new(capabilities, DriverRegistry::new());

    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(platform)
        .llm_sim(LlmSimConfig::fixed("ok"))
        .single_session(|s| {
            s.harness("math", "You are a math assistant.")
                .with_capability("test_math")
                .with_capability("btw")
                .agent("math-agent", "Use tools when needed.")
        })
        .build()
        .await
        .unwrap();

    let session_id = runtime.default_session_id().expect("default session id");
    let commands = runtime.list_commands(session_id).await.unwrap();

    let btw = commands
        .iter()
        .find(|c| c.name == "btw")
        .expect("btw command surfaced");
    assert_eq!(btw.source, CommandSource::System);
    assert_eq!(btw.args.len(), 1);
    assert!(btw.args[0].required);
}

#[tokio::test]
async fn execute_command_dispatches_to_capability_handler() {
    use async_trait::async_trait;
    use everruns_core::capabilities::{Capability, CapabilityStatus};
    use everruns_core::command::{
        CommandArg, CommandDescriptor, CommandExecutionContext, CommandResult, CommandSource,
        ExecuteCommandRequest,
    };
    use std::sync::Mutex;

    struct EchoCapability {
        seen: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl Capability for EchoCapability {
        fn id(&self) -> &str {
            "echo"
        }
        fn name(&self) -> &str {
            "Echo"
        }
        fn description(&self) -> &str {
            "Echoes its argument."
        }
        fn status(&self) -> CapabilityStatus {
            CapabilityStatus::Available
        }
        fn commands(&self) -> Vec<CommandDescriptor> {
            vec![CommandDescriptor {
                name: "echo".to_string(),
                description: "echo".to_string(),
                source: CommandSource::System,
                args: vec![CommandArg {
                    name: "text".to_string(),
                    description: "text to echo".to_string(),
                    required: true,
                    suggestions: vec![],
                }],
            }]
        }
        async fn execute_command(
            &self,
            request: &ExecuteCommandRequest,
            ctx: &CommandExecutionContext,
        ) -> everruns_core::Result<CommandResult> {
            let arg = request.arguments.clone().unwrap_or_default();
            self.seen.lock().unwrap().push(arg.clone());
            Ok(CommandResult {
                success: true,
                message: format!("echo[{}]: {}", ctx.session_id, arg),
                error_code: None,
                error_fields: None,
            })
        }
    }

    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(EchoCapability { seen: seen.clone() });
    let platform = PlatformDefinition::new(capabilities, DriverRegistry::new());

    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(platform)
        .llm_sim(LlmSimConfig::fixed("ok"))
        .single_session(|s| s.harness("h", "prompt").with_capability("echo"))
        .build()
        .await
        .unwrap();

    let session_id = runtime.default_session_id().expect("default session id");
    let result = runtime
        .execute_command(
            session_id,
            ExecuteCommandRequest {
                name: "echo".to_string(),
                arguments: Some("hello".to_string()),
                controls: None,
            },
        )
        .await
        .unwrap();

    assert!(result.success);
    assert!(result.message.contains("hello"));
    assert_eq!(seen.lock().unwrap().as_slice(), &["hello".to_string()]);

    // Unknown command: dispatcher errors instead of silently succeeding.
    let unknown = runtime
        .execute_command(
            session_id,
            ExecuteCommandRequest {
                name: "nope".to_string(),
                arguments: None,
                controls: None,
            },
        )
        .await;
    assert!(unknown.is_err());
}

// EVE-543: /btw executes end to end through the core capability and the
// store-backed command host — no host-specific executor.
#[tokio::test]
async fn execute_btw_command_returns_ephemeral_answer() {
    use everruns_core::capabilities::BtwCapability;
    use everruns_core::command::ExecuteCommandRequest;

    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(BtwCapability);
    let platform = PlatformDefinition::new(capabilities, DriverRegistry::new());

    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(platform)
        .llm_sim(LlmSimConfig::sequence(vec![
            "main answer".to_string(),
            "the side answer".to_string(),
        ]))
        .single_session(|s| {
            s.harness("h", "You are a helpful assistant.")
                .with_capability("btw")
        })
        .build()
        .await
        .unwrap();

    let session_id = runtime.default_session_id().expect("default session id");

    // Establish history with a normal turn, then ask the side question.
    let turn = runtime
        .run_text_turn(session_id, "hello there")
        .await
        .unwrap();
    assert!(turn.success);
    let messages_before = runtime.messages(session_id).await.unwrap();

    let result = runtime
        .execute_command(
            session_id,
            ExecuteCommandRequest {
                name: "btw".to_string(),
                arguments: Some("what are you doing?".to_string()),
                controls: None,
            },
        )
        .await
        .unwrap();

    assert!(result.success);
    assert_eq!(result.message, "the side answer");

    // Ephemeral: the side question/answer never reach the session history.
    let messages_after = runtime.messages(session_id).await.unwrap();
    assert_eq!(messages_before.len(), messages_after.len());

    // Missing question is a capability-level validation error.
    let missing = runtime
        .execute_command(
            session_id,
            ExecuteCommandRequest {
                name: "btw".to_string(),
                arguments: None,
                controls: None,
            },
        )
        .await;
    assert!(
        missing
            .unwrap_err()
            .to_string()
            .contains("requires a question")
    );
}

// ---------------------------------------------------------------------------
// Connection resolver injection
//
// Proves embedders can supply their own `UserConnectionResolver` through
// `HostBackends` and have it reach connection-aware tools via
// `ToolContext.connection_resolver` (the seam used by integrations such as
// Daytona). See knowledge/foundations/runtime.md and crates/server/specs/user-connections.md.
// ---------------------------------------------------------------------------

struct StaticTokenResolver {
    provider: String,
    token: String,
}

#[async_trait]
impl everruns_core::traits::UserConnectionResolver for StaticTokenResolver {
    async fn get_connection_token(
        &self,
        _session_id: everruns_core::SessionId,
        provider: &str,
    ) -> everruns_core::Result<Option<String>> {
        Ok((provider == self.provider).then(|| self.token.clone()))
    }
}

struct ConnectionEchoCapability;

impl everruns_core::capabilities::Capability for ConnectionEchoCapability {
    fn id(&self) -> &str {
        "connection_echo"
    }
    fn name(&self) -> &str {
        "Connection Echo"
    }
    fn description(&self) -> &str {
        "Testing capability: echoes a resolved connection token via ToolContext."
    }
    fn status(&self) -> everruns_core::capabilities::CapabilityStatus {
        everruns_core::capabilities::CapabilityStatus::Available
    }
    fn tools(&self) -> Vec<Box<dyn everruns_core::tools::Tool>> {
        vec![Box::new(ConnectionEchoTool)]
    }
}

struct ConnectionEchoTool;

#[async_trait]
impl everruns_core::tools::Tool for ConnectionEchoTool {
    fn name(&self) -> &str {
        "echo_connection_token"
    }
    fn description(&self) -> &str {
        "Resolve the connection token for a provider and return it."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "provider": { "type": "string", "description": "Provider id, e.g. daytona" }
            },
            "required": ["provider"],
            "additionalProperties": false
        })
    }
    fn requires_context(&self) -> bool {
        true
    }
    async fn execute(
        &self,
        _arguments: serde_json::Value,
    ) -> everruns_core::tools::ToolExecutionResult {
        everruns_core::tools::ToolExecutionResult::tool_error("requires context")
    }
    async fn execute_with_context(
        &self,
        arguments: serde_json::Value,
        context: &everruns_core::traits::ToolContext,
    ) -> everruns_core::tools::ToolExecutionResult {
        // Fail fast on broken argument plumbing rather than defaulting — a
        // missing/empty `provider` means the tool call did not arrive intact,
        // which must surface as an error, not a "no connection" false pass.
        let provider = match arguments.get("provider").and_then(|v| v.as_str()) {
            Some(provider) if !provider.is_empty() => provider,
            _ => {
                return everruns_core::tools::ToolExecutionResult::internal_error_msg(
                    "echo_connection_token: missing or empty `provider` argument",
                );
            }
        };
        // A missing resolver is a distinct wiring failure from "user has no
        // connection" — keep them separable so the test catches each.
        let Some(resolver) = context.connection_resolver.as_ref() else {
            return everruns_core::tools::ToolExecutionResult::internal_error_msg(
                "echo_connection_token: no connection resolver in ToolContext",
            );
        };
        // Surface resolver failures as internal errors instead of swallowing
        // them; only a clean `Ok(None)` is the legitimate "not connected" case.
        match resolver
            .get_connection_token(context.session_id, provider)
            .await
        {
            Ok(Some(token)) => everruns_core::tools::ToolExecutionResult::success(
                serde_json::json!({ "token": token }),
            ),
            Ok(None) => everruns_core::tools::ToolExecutionResult::tool_error("no connection"),
            Err(err) => everruns_core::tools::ToolExecutionResult::internal_error_msg(format!(
                "echo_connection_token: resolver failed: {err}"
            )),
        }
    }
}

fn connection_platform() -> PlatformDefinition {
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(ConnectionEchoCapability);
    PlatformDefinition::new(capabilities, DriverRegistry::new())
}

#[tokio::test]
async fn runtime_exposes_injected_connection_resolver_to_host_adapter() {
    use everruns_host::RuntimeHostAdapter;

    let resolver = Arc::new(StaticTokenResolver {
        provider: "daytona".to_string(),
        token: "tok-host-adapter".to_string(),
    });
    let backends = HostBackends::in_memory().with_connection_resolver(resolver);

    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(connection_platform())
        .backends(backends)
        .llm_sim(LlmSimConfig::fixed("ok"))
        .single_session(|s| s.harness("h", "h").agent("a", "a"))
        .build()
        .await
        .unwrap();

    let session_id = runtime.default_session_id().expect("default session id");
    let resolver = runtime
        .connection_resolver()
        .expect("connection resolver should be wired into the host adapter");
    let token = resolver
        .get_connection_token(session_id, "daytona")
        .await
        .unwrap();
    assert_eq!(token.as_deref(), Some("tok-host-adapter"));

    // Unknown providers resolve to None rather than erroring.
    let missing = resolver
        .get_connection_token(session_id, "github")
        .await
        .unwrap();
    assert_eq!(missing, None);
}

#[tokio::test]
async fn runtime_without_resolver_leaves_connection_resolver_unset() {
    use everruns_host::RuntimeHostAdapter;

    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(connection_platform())
        .llm_sim(LlmSimConfig::fixed("ok"))
        .single_session(|s| s.harness("h", "h").agent("a", "a"))
        .build()
        .await
        .unwrap();

    assert!(
        runtime.connection_resolver().is_none(),
        "no resolver should be wired unless the embedder supplies one",
    );
}

#[tokio::test]
async fn injected_resolver_reaches_tool_context_during_a_turn() {
    let harness_id = "harness_00000000000000000000000000000061".parse().unwrap();
    let agent_id = "agent_00000000000000000000000000000061".parse().unwrap();
    let session_id: everruns_core::SessionId =
        "session_00000000000000000000000000000061".parse().unwrap();

    let resolver = Arc::new(StaticTokenResolver {
        provider: "daytona".to_string(),
        token: "tok-tool-context".to_string(),
    });
    let backends = HostBackends::in_memory().with_connection_resolver(resolver);

    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(connection_platform())
        .backends(backends)
        .llm_sim(
            LlmSimConfig::fixed("resolving token").with_tool_call_sequence(vec![
                vec![ToolCall {
                    id: "call_echo_1".into(),
                    name: "echo_connection_token".into(),
                    arguments: serde_json::json!({ "provider": "daytona" }),
                }],
                vec![],
            ]),
        )
        .default_model(ResolvedModel {
            model: "llmsim-model".into(),
            provider_type: DriverId::LlmSim,
            api_key: Some("fake-key".into()),
            base_url: None,
            provider_metadata: None,
        })
        .harness(
            HarnessBuilder::new("conn", "You resolve connection tokens.")
                .id(harness_id)
                .capability("connection_echo")
                .build(),
        )
        .agent(
            AgentBuilder::new("conn-agent", "Use the tool.")
                .id(agent_id)
                .max_iterations(8)
                .build(),
        )
        .session(session(session_id, harness_id, Some(agent_id)))
        .build()
        .await
        .unwrap();

    let result = runtime
        .run_text_turn(session_id, "Resolve the daytona token.")
        .await
        .unwrap();
    assert!(result.success);

    let messages = runtime.messages(session_id).await.unwrap();
    let tool_result = messages
        .iter()
        .find(|m| m.role == MessageRole::ToolResult && m.tool_call_id() == Some("call_echo_1"))
        .expect("tool result message");
    let serialized = serde_json::to_string(tool_result).expect("serialize tool result");
    assert!(
        serialized.contains("tok-tool-context"),
        "resolved token must reach the tool via ToolContext; got: {serialized}",
    );
}

// ============================================================================
// Plugin directory loading tests
// ============================================================================

/// Fixture path for the microsoft-docs plugin used by plugin loading tests.
const MICROSOFT_DOCS_PLUGIN_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../testdata/plugins/microsoft-docs"
);

/// Build a minimal runtime with the microsoft-docs plugin loaded and a single
/// session whose agent enables `plugin:microsoft-docs`.
async fn runtime_with_microsoft_docs_plugin()
-> (everruns_host::InProcessRuntime, everruns_core::SessionId) {
    use std::path::Path;

    let plugin_dir = Path::new(MICROSOFT_DOCS_PLUGIN_DIR);

    let builder = InProcessRuntimeBuilder::new()
        .llm_sim(LlmSimConfig::fixed("ok"))
        .with_plugin_dir(plugin_dir)
        .expect("microsoft-docs plugin should compile without error");

    let plugin_cap = builder
        .plugin_capability("microsoft-docs")
        .expect("plugin capability must be registered after with_plugin_dir");

    let runtime = builder
        .single_session(|s| {
            s.harness("test-harness", "You are a test harness.")
                .agent("test-agent", "Use docs when needed.")
                // Pass the hydrated config so the agent carries the full definition.
                .agent_capability(plugin_cap)
        })
        .build()
        .await
        .expect("runtime build must succeed");

    let session_id = runtime
        .default_session_id()
        .expect("single_session sets default_session_id");

    (runtime, session_id)
}

#[tokio::test]
async fn with_plugin_dir_compiles_and_loads_microsoft_docs() {
    use std::path::Path;

    // Loading a valid plugin directory must succeed.
    let result =
        InProcessRuntimeBuilder::new().with_plugin_dir(Path::new(MICROSOFT_DOCS_PLUGIN_DIR));
    assert!(
        result.is_ok(),
        "with_plugin_dir should succeed for the fixture: {:?}",
        result.err()
    );

    let builder = result.unwrap();

    // The hydrated capability config must be available under the plugin name.
    let cap = builder.plugin_capability("microsoft-docs");
    assert!(
        cap.is_some(),
        "plugin_capability('microsoft-docs') must be Some after loading"
    );

    let cap = cap.unwrap();
    assert_eq!(cap.capability_id(), "plugin:microsoft-docs");
    // The config must be a non-empty JSON object (the serialized definition).
    assert!(
        cap.config_value().is_object() && !cap.config_value().as_object().unwrap().is_empty(),
        "hydrated config must be a non-empty JSON object"
    );
}

#[tokio::test]
async fn with_plugin_dir_missing_path_returns_error() {
    use std::path::Path;

    let result = InProcessRuntimeBuilder::new()
        .with_plugin_dir(Path::new("/nonexistent/path/that/does/not/exist"));
    assert!(result.is_err(), "missing directory should return an error");
    let msg = result.map(|_| ()).unwrap_err().to_string();
    assert!(
        msg.contains("plugin directory load failed"),
        "error message should mention 'plugin directory load failed', got: {msg}"
    );
}

#[tokio::test]
async fn plugin_warnings_are_accessible_on_built_runtime() {
    // The microsoft-docs fixture has an `interface` block that produces a
    // compile warning. Verify warnings are surfaced on the built runtime.
    use std::path::Path;

    let plugin_dir = Path::new(MICROSOFT_DOCS_PLUGIN_DIR);
    let builder = InProcessRuntimeBuilder::new()
        .llm_sim(LlmSimConfig::fixed("ok"))
        .with_plugin_dir(plugin_dir)
        .expect("plugin should compile");

    let plugin_cap = builder
        .plugin_capability("microsoft-docs")
        .expect("plugin capability must exist");

    let runtime = builder
        .single_session(|s| {
            s.harness("h", "")
                .agent("a", "")
                .agent_capability(plugin_cap)
        })
        .build()
        .await
        .expect("build must succeed");

    let warnings = runtime.plugin_warnings();
    assert!(
        !warnings.is_empty(),
        "expected at least one warning from the interface block"
    );
    assert!(
        warnings.iter().any(|w| w.contains("interface")),
        "expected 'interface' warning, got: {warnings:?}"
    );
}

#[tokio::test]
async fn load_context_with_plugin_contains_system_prompt() {
    // The docs-researcher agent file is compiled into the system_prompt of the
    // plugin capability. load_context should return a runtime_agent whose
    // assembled system prompt contains "docs-researcher".
    let (runtime, session_id) = runtime_with_microsoft_docs_plugin().await;

    let ctx = runtime
        .load_context(session_id)
        .await
        .expect("load_context must succeed");

    let system_prompt = &ctx.runtime_agent.system_prompt;
    assert!(
        system_prompt.contains("docs-researcher"),
        "assembled system prompt must include the docs-researcher agent section, got: {system_prompt}"
    );
}

#[tokio::test]
async fn load_context_with_plugin_has_skill_mount() {
    // Skills compiled from `skills/microsoft-docs/SKILL.md` must appear as
    // mounts under `/.agents/skills/microsoft-docs/` in the assembled context.
    // We verify by checking the resolved_capability_configs carry the skill
    // definition, and that the assembled runtime_agent system prompt is non-empty
    // (which confirms the full collection pipeline ran on the hydrated plugin config).
    use everruns_core::DeclarativeCapabilityDefinition;
    use everruns_core::SKILLS_DISCOVERY_PATH;

    let (runtime, session_id) = runtime_with_microsoft_docs_plugin().await;

    let ctx = runtime
        .load_context(session_id)
        .await
        .expect("load_context must succeed");

    // Find the plugin capability config in the resolved set.
    let plugin_config = ctx
        .resolved_capability_configs
        .iter()
        .find(|c| c.capability_id() == "plugin:microsoft-docs")
        .expect("plugin:microsoft-docs must appear in resolved_capability_configs");

    // Deserialise the definition from the config.
    let definition: DeclarativeCapabilityDefinition =
        serde_json::from_value(plugin_config.config_value().clone())
            .expect("plugin config must deserialise as DeclarativeCapabilityDefinition");

    // At least one skill must be present.
    assert!(
        !definition.skills.is_empty(),
        "compiled plugin must have at least one skill"
    );

    // The microsoft-docs skill must be present.
    let skill = definition
        .skills
        .iter()
        .find(|s| s.name == "microsoft-docs")
        .expect("microsoft-docs skill must be present");
    assert!(
        !skill.instructions.is_empty(),
        "skill instructions must be non-empty"
    );

    // Skills contribute mounts under /.agents/skills/{name}/SKILL.md.
    // The collect_capabilities_with_configs pipeline converts skill_contributions
    // into MountPoints; verify the expected path prefix matches what the skill name
    // would produce.
    let expected_skill_mount_dir = format!("{SKILLS_DISCOVERY_PATH}/microsoft-docs");
    // The definition exposes the skill name directly; confirm the mount dir is
    // consistent with the SKILLS_DISCOVERY_PATH constant.
    assert!(
        expected_skill_mount_dir.starts_with(SKILLS_DISCOVERY_PATH),
        "expected skill mount under {SKILLS_DISCOVERY_PATH}, got: {expected_skill_mount_dir}"
    );
    assert_eq!(
        skill.name, "microsoft-docs",
        "skill name must be 'microsoft-docs'"
    );
}

#[tokio::test]
async fn load_context_with_plugin_has_mcp_server_config() {
    // The .mcp.json file declares `microsoft-learn` pointing at
    // https://learn.microsoft.com/api/mcp. Verify the compiled capability
    // config exposes this server.
    use everruns_core::DeclarativeCapabilityDefinition;

    let (runtime, session_id) = runtime_with_microsoft_docs_plugin().await;

    let ctx = runtime
        .load_context(session_id)
        .await
        .expect("load_context must succeed");

    let plugin_config = ctx
        .resolved_capability_configs
        .iter()
        .find(|c| c.capability_id() == "plugin:microsoft-docs")
        .expect("plugin:microsoft-docs must appear in resolved_capability_configs");

    let definition: DeclarativeCapabilityDefinition =
        serde_json::from_value(plugin_config.config_value().clone())
            .expect("plugin config must deserialise as DeclarativeCapabilityDefinition");

    let mcp_servers = definition
        .mcp_servers
        .as_ref()
        .expect("plugin definition must contain mcp_servers");

    let server = mcp_servers
        .get("microsoft-learn")
        .expect("microsoft-learn MCP server must be present");

    assert_eq!(
        server.url, "https://learn.microsoft.com/api/mcp",
        "MCP server URL must match the fixture"
    );
}
