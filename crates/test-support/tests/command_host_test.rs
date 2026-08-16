// Command-host tests driven through the llmsim simulator.
//
// Moved from `everruns-core` (EVE-875): core no longer ships the simulator,
// so these tests live with the test-support crate that does.

use everruns_core::{CapabilityRegistry, DisabledCommandHost};
use everruns_core::{
    CommandHost, ExecutionSession, SessionCompletionError, SessionCompletionRequest,
};
use everruns_host::StoreCommandHost;
use everruns_provider::driver_registry::DriverRegistry;
use everruns_provider::error::AgentLoopError;
use everruns_provider::model_spec::ModelSpec;
use everruns_provider::typed_id::SessionId;
use everruns_provider::user_facing_error::UserFacingErrorContext;
use everruns_test_support::TestMathCapability;
use std::collections::HashMap;
use std::sync::Arc;

use everruns_core::AgentDefinition;

use everruns_core::harness_definition::HarnessDefinition;
use everruns_core::message_retriever::InputMessage;
use everruns_core::session::SessionExecutionState;
use everruns_host::{
    InMemoryAgentStore, InMemoryHarnessStore, InMemoryProviderStore, InMemorySessionStore,
};
use everruns_provider::driver_registry::LlmStreamEvent;
use everruns_provider::provider::DriverId;
use everruns_provider::typed_id::{AgentId, HarnessId};
use everruns_test_support::{InMemoryMessageRetriever, LlmSimConfig, LlmSimDriver};
use futures::StreamExt;

#[tokio::test]
async fn disabled_host_errors_clearly() {
    let host = DisabledCommandHost;
    let error = host.turn_context().await.unwrap_err();
    assert!(error.to_string().contains("turn-context"));

    let error = host
        .completion(SessionCompletionRequest::default())
        .await
        .unwrap_err();
    assert!(matches!(error, SessionCompletionError::InvalidRequest(_)));

    // The default trait impl covers hosts that never override streaming:
    // a stable variant commands can match to fall back to completion().
    let error = host
        .completion_stream(SessionCompletionRequest::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        SessionCompletionError::StreamingUnsupported
    ));
    let error = error.into_command_result().unwrap_err();
    assert!(error.to_string().contains("streaming"));
}

fn test_harness() -> HarnessDefinition {
    HarnessDefinition {
        capabilities: vec![everruns_capability::CapabilityRef::new("test_math")],
        ..HarnessDefinition::new("h", "You are a test harness.")
    }
}

fn test_agent(agent_id: AgentId) -> AgentDefinition {
    AgentDefinition {
        max_iterations: Some(8),
        ..AgentDefinition::new(agent_id, "a", "Use tools.")
    }
}

fn test_session(
    session_id: SessionId,
    harness_id: HarnessId,
    agent_id: AgentId,
) -> ExecutionSession {
    ExecutionSession {
        id: session_id,
        workspace_id: everruns_provider::typed_id::WorkspaceId::from_uuid((session_id).uuid()),
        organization_id: everruns_core::DEFAULT_ORG_PUBLIC_ID.to_string(),
        harness_id,
        agent_id: Some(agent_id),
        title: None,
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

/// Build a `StoreCommandHost` over in-memory stores with an llm-sim
/// driver returning `response`.
async fn llmsim_host(response: &str) -> StoreCommandHost {
    let harness_id: HarnessId = "harness_000000000000000000000000000000a1".parse().unwrap();
    let agent_id: AgentId = "agent_000000000000000000000000000000a1".parse().unwrap();
    let session_id: SessionId = "session_000000000000000000000000000000a1".parse().unwrap();

    let harness_store = InMemoryHarnessStore::new();
    harness_store.add_harness(harness_id, test_harness()).await;
    let agent_store = InMemoryAgentStore::new();
    agent_store.add_agent(test_agent(agent_id)).await;
    let session_store = InMemorySessionStore::new();
    session_store
        .add_session(test_session(session_id, harness_id, agent_id))
        .await;
    let message_store = InMemoryMessageRetriever::new();
    message_store
        .add(session_id, InputMessage::user("earlier message"))
        .await
        .unwrap();

    let provider_store = InMemoryProviderStore::new();
    provider_store
        .set_default_model_spec(ModelSpec::on((DriverId::LlmSim).as_str(), "llmsim-model"))
        .await;
    provider_store
        .add_model(
            everruns_provider::typed_id::ModelId::from_seed(905),
            ModelSpec::on((DriverId::LlmSim).as_str(), "override-model"),
        )
        .await;

    let mut capability_registry = CapabilityRegistry::new();
    capability_registry.register(TestMathCapability);

    let mut driver_registry = DriverRegistry::new();
    let driver = LlmSimDriver::new(LlmSimConfig::fixed(response));
    driver_registry.register(DriverId::LlmSim, move |_config| Box::new(driver.clone()));

    StoreCommandHost::new(
        session_id,
        Arc::new(harness_store),
        Arc::new(agent_store),
        Arc::new(session_store),
        Arc::new(message_store),
        Arc::new(provider_store),
        capability_registry,
        driver_registry,
    )
}

async fn selected_but_unconfigured_host() -> StoreCommandHost {
    let harness_id: HarnessId = "harness_000000000000000000000000000000b1".parse().unwrap();
    let agent_id: AgentId = "agent_000000000000000000000000000000b1".parse().unwrap();
    let session_id: SessionId = "session_000000000000000000000000000000b1".parse().unwrap();

    let harness_store = InMemoryHarnessStore::new();
    harness_store.add_harness(harness_id, test_harness()).await;
    let agent_store = InMemoryAgentStore::new();
    agent_store.add_agent(test_agent(agent_id)).await;
    let session_store = InMemorySessionStore::new();
    session_store
        .add_session(test_session(session_id, harness_id, agent_id))
        .await;
    let provider_store = InMemoryProviderStore::new();
    provider_store
        .set_default_model_spec(ModelSpec::on("openai", "gpt-test"))
        .await;

    struct NetworkSentinel;
    #[async_trait::async_trait]
    impl everruns_provider::driver_registry::ChatDriver for NetworkSentinel {
        async fn chat_completion_stream(
            &self,
            _endpoint: &everruns_provider::runtime_provider::ProviderEndpoint,
            _messages: Vec<everruns_provider::driver_registry::LlmMessage>,
            _config: &everruns_provider::driver_registry::LlmCallConfig,
        ) -> everruns_provider::error::Result<everruns_provider::driver_registry::LlmResponseStream>
        {
            panic!("missing credentials must be rejected before provider I/O")
        }
    }

    let mut driver_registry = DriverRegistry::new();
    driver_registry.register(DriverId::OpenAI, |_config| Box::new(NetworkSentinel));
    let mut capability_registry = CapabilityRegistry::new();
    capability_registry.register(TestMathCapability);

    StoreCommandHost::new(
        session_id,
        Arc::new(harness_store),
        Arc::new(agent_store),
        Arc::new(session_store),
        Arc::new(InMemoryMessageRetriever::new()),
        Arc::new(provider_store),
        capability_registry,
        driver_registry,
    )
}

#[tokio::test]
async fn configuration_commands_can_inspect_a_selected_but_unconfigured_provider() {
    let host = selected_but_unconfigured_host().await;
    let turn = host
        .turn_context()
        .await
        .expect("credential-free command context remains reachable");
    assert_eq!(turn.provider_type, "openai");
    assert_eq!(turn.model, "gpt-test");

    let error = host
        .completion(SessionCompletionRequest {
            system_prompts: vec![turn.system_prompt],
            messages: turn.messages,
            controls: None,
            metadata: HashMap::new(),
        })
        .await
        .expect_err("model call must fail before network I/O");
    let result = error
        .into_command_result()
        .expect("provider failure is classified");
    assert_eq!(result.error_code.as_deref(), Some("provider_misconfigured"));
}

#[tokio::test]
async fn store_host_resolves_per_completion_model_override() {
    let host = llmsim_host("override answer").await;
    let turn = host.turn_context().await.unwrap();
    let stream = host
        .completion_stream(SessionCompletionRequest {
            system_prompts: vec![turn.system_prompt],
            messages: turn.messages,
            controls: Some(everruns_core::Controls {
                model_id: Some(everruns_provider::typed_id::ModelId::from_seed(905)),
                ..Default::default()
            }),
            metadata: HashMap::new(),
        })
        .await
        .unwrap();

    assert_eq!(stream.context.provider.as_deref(), Some("llmsim"));
    assert_eq!(stream.context.model_id.as_deref(), Some("override-model"));
}

#[tokio::test]
async fn store_host_completion_runs_against_session_model() {
    let host = llmsim_host("the side answer").await;

    let turn = host.turn_context().await.unwrap();
    assert_eq!(turn.model, "llmsim-model");
    assert_eq!(turn.provider_type, "llmsim");
    assert_eq!(turn.messages.len(), 1);
    assert!(!turn.system_prompt.is_empty());

    let completion = host
        .completion(SessionCompletionRequest {
            system_prompts: vec![turn.system_prompt, "Answer once.".into()],
            messages: turn.messages,
            controls: None,
            metadata: HashMap::new(),
        })
        .await
        .unwrap();
    assert_eq!(completion.text, "the side answer");
}

#[tokio::test]
async fn store_host_completion_stream_emits_progressive_deltas() {
    let host = llmsim_host("streamed side answer with several tokens").await;

    let turn = host.turn_context().await.unwrap();
    let stream = host
        .completion_stream(SessionCompletionRequest {
            system_prompts: vec![turn.system_prompt],
            messages: turn.messages,
            controls: None,
            metadata: HashMap::new(),
        })
        .await
        .unwrap();

    // Classification context mirrors the non-streaming error path.
    assert_eq!(stream.context.provider.as_deref(), Some("llmsim"));
    assert_eq!(stream.context.model_id.as_deref(), Some("llmsim-model"));

    let mut deltas = Vec::new();
    let mut done = false;
    let mut events = stream.events;
    while let Some(event) = events.next().await {
        match event.unwrap() {
            LlmStreamEvent::TextDelta(delta) => deltas.push(delta),
            LlmStreamEvent::Done(_) => done = true,
            _ => {}
        }
    }

    assert!(done, "stream must terminate with Done");
    assert!(
        deltas.len() > 1,
        "expected progressive deltas, got {deltas:?}"
    );
    assert_eq!(deltas.concat(), "streamed side answer with several tokens");
}

#[test]
fn completion_error_classifies_provider_failures() {
    let error = SessionCompletionError::Completion {
        error: "OpenAI API error (401): unauthorized".to_string(),
        context: UserFacingErrorContext::default()
            .with_provider("openai")
            .with_model_id("gpt-5"),
    };

    let result = error.into_command_result().expect("classified result");
    assert!(!result.success);
    assert_eq!(result.error_code.as_deref(), Some("provider_misconfigured"));
    let fields = result.error_fields.expect("error_fields populated");
    assert_eq!(
        fields.get("provider").and_then(|v| v.as_str()),
        Some("openai")
    );
    assert_eq!(
        fields.get("model_id").and_then(|v| v.as_str()),
        Some("gpt-5")
    );
}

#[test]
fn completion_error_bubbles_invalid_requests() {
    let error = SessionCompletionError::InvalidRequest(AgentLoopError::config("Model not found"));
    assert!(error.into_command_result().is_err());
}
