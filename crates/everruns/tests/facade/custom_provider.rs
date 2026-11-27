use async_trait::async_trait;
use everruns::llm::Message;
use everruns::{
    Agent, AgentLoopError, ChatDriver, InMemoryEngine, LlmCallConfig, LlmResponseStream,
    LlmStreamEvent, Provider, ProviderEndpoint,
};

#[derive(Clone)]
struct DownstreamProtocol;

#[cfg(feature = "builtins")]
#[derive(Clone)]
struct PlannedProtocol(std::sync::Arc<std::sync::atomic::AtomicUsize>);

#[cfg(feature = "builtins")]
#[async_trait]
impl ChatDriver for PlannedProtocol {
    fn tool_request_projection(
        &self,
        endpoint: &ProviderEndpoint,
        _messages: &[Message],
        config: &LlmCallConfig,
    ) -> everruns::ToolRequestProjection {
        assert_eq!(endpoint.base_url(), Some("https://application.example/v1"));
        assert!(
            !config
                .driver_options
                .keys()
                .any(|key| key.starts_with("everruns/"))
        );
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        everruns::ToolRequestProjection::Current
    }

    async fn chat_completion_stream(
        &self,
        endpoint: &ProviderEndpoint,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream, AgentLoopError> {
        assert_eq!(endpoint.base_url(), Some("https://application.example/v1"));
        DownstreamProtocol
            .chat_completion_stream(endpoint, messages, config)
            .await
    }
}

#[cfg(feature = "builtins")]
#[tokio::test]
async fn framework_uses_custom_request_planning_with_the_bound_endpoint() {
    let planned = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let agent = Agent::builder()
        .instructions("Reply through the application provider.")
        .provider(
            Provider::new(
                "arbitrary-application-key",
                PlannedProtocol(planned.clone()),
            )
            .base_url("https://application.example/v1"),
        )
        .model("application-model")
        .capability(everruns::CapabilityRef::new("infinity_context"))
        .build()
        .unwrap();
    let session = InMemoryEngine::new().create(agent);
    for _ in 0..2 {
        assert_eq!(
            session.run("hello").await.unwrap().response,
            "downstream works"
        );
    }
    assert!(planned.load(std::sync::atomic::Ordering::SeqCst) >= 2);
    let metadata = everruns::LlmCompletionMetadata::default();
    assert_eq!(metadata.outcome(), everruns::CompletionOutcome::Unknown);
}

#[async_trait]
impl ChatDriver for DownstreamProtocol {
    async fn chat_completion_stream(
        &self,
        _endpoint: &ProviderEndpoint,
        _messages: Vec<Message>,
        _config: &LlmCallConfig,
    ) -> Result<LlmResponseStream, AgentLoopError> {
        Ok(Box::pin(futures::stream::iter([
            Ok(LlmStreamEvent::TextDelta("downstream works".to_string())),
            Ok(LlmStreamEvent::Done(Box::default())),
        ])))
    }
}

#[tokio::test]
async fn downstream_provider_needs_only_the_everruns_api() {
    let agent = Agent::builder()
        .instructions("Reply through the configured provider.")
        .provider(
            Provider::new("my-gateway", DownstreamProtocol)
                .base_url("https://gateway.example/v1")
                .header("x-tenant", "tenant-a"),
        )
        .model("custom-model")
        .build()
        .unwrap();

    let turn = InMemoryEngine::new()
        .create(agent.clone())
        .run("hello")
        .await
        .unwrap();
    assert_eq!(turn.response, "downstream works");
}

/// Records the system text the model call receives.
#[derive(Clone, Default)]
struct CapturingProtocol {
    system: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

#[async_trait]
impl ChatDriver for CapturingProtocol {
    async fn chat_completion_stream(
        &self,
        _endpoint: &ProviderEndpoint,
        messages: Vec<Message>,
        _config: &LlmCallConfig,
    ) -> Result<LlmResponseStream, AgentLoopError> {
        let system = messages
            .iter()
            .filter(|message| message.role == everruns::llm::MessageRole::System)
            .map(|message| message.content.to_text())
            .collect::<Vec<_>>()
            .join("\n\n");
        self.system.lock().unwrap().push(system);
        Ok(Box::pin(futures::stream::iter([
            Ok(LlmStreamEvent::TextDelta("ok".to_string())),
            Ok(LlmStreamEvent::Done(Box::default())),
        ])))
    }
}

#[tokio::test]
async fn model_call_receives_agent_instructions_once() {
    let protocol = CapturingProtocol::default();
    let agent = Agent::builder()
        .instructions("MODEL_CALL_INSTRUCTIONS_SENTINEL")
        .provider(Provider::new("capture", protocol.clone()).base_url("https://capture.example/v1"))
        .model("custom-model")
        .build()
        .unwrap();

    InMemoryEngine::new()
        .create(agent)
        .run("hello")
        .await
        .unwrap();

    let calls = protocol.system.lock().unwrap().clone();
    assert!(!calls.is_empty(), "model was not called");
    for system in calls {
        assert_eq!(
            system.matches("MODEL_CALL_INSTRUCTIONS_SENTINEL").count(),
            1,
            "instructions duplicated in model call: {system:?}"
        );
    }
}

#[test]
fn fixture_has_no_private_runtime_imports() {
    let source = include_str!("custom_provider.rs");
    assert!(!source.contains(concat!("everruns_", "core")));
    assert!(!source.contains(concat!("everruns_", "runtime")));
    assert!(!source.contains(concat!("is_", "openai")));
}

#[test]
fn facade_has_no_builtin_provider_dispatch() {
    let source = include_str!("../../src/agent.rs");
    assert!(!source.contains("DriverId"));
    assert!(!source.contains(concat!("is_", "openai")));
    assert!(!source.contains("provider_type =="));
}
