use async_trait::async_trait;
use everruns::{
    Agent, AgentLoopError, ChatDriver, LlmCallConfig, LlmMessage, LlmResponseStream,
    LlmStreamEvent, ModelSpec, Provider, ProviderEndpoint,
};

#[derive(Clone)]
struct DownstreamProtocol;

#[async_trait]
impl ChatDriver for DownstreamProtocol {
    async fn chat_completion_stream(
        &self,
        _endpoint: &ProviderEndpoint,
        _messages: Vec<LlmMessage>,
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
        .model(ModelSpec::on("my-gateway", "custom-model"))
        .provider(
            Provider::new("my-gateway", DownstreamProtocol)
                .base_url("https://gateway.example/v1")
                .header("x-tenant", "tenant-a"),
        )
        .build()
        .unwrap();

    let turn = agent.session().run("hello").await.unwrap();
    assert_eq!(turn.response, "downstream works");
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
    let source = include_str!("../src/agent.rs");
    assert!(!source.contains("DriverId"));
    assert!(!source.contains(concat!("is_", "openai")));
    assert!(!source.contains("provider_type =="));
}
