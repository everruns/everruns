// Integration tests for ReasonAtom with LlmSimDriver
//
// These tests verify the full ReasonAtom workflow using the simulated LLM driver,
// enabling deterministic testing without real LLM API calls.
//
// Run with: cargo test -p everruns-core --test reason_atom_test

use async_trait::async_trait;
use everruns_core::AgentId;
use everruns_core::MessageRetriever;
use everruns_core::agent::{Agent, AgentStatus};
use everruns_core::atoms::{Atom, AtomContext, ReasonAtom, ReasonInput};
use everruns_core::capabilities::CapabilityRegistry;
use everruns_core::harness::{Harness, HarnessStatus};
use everruns_core::llm_driver_registry::LlmCallConfigBuilder;
use everruns_core::llm_driver_registry::{DriverRegistry, ProviderType};
use everruns_core::llm_models::LlmProviderType;
use everruns_core::llmsim_driver::{LlmSimConfig, LlmSimDriver, register_driver};
use everruns_core::memory::{
    InMemoryAgentStore, InMemoryHarnessStore, InMemoryLlmProviderStore, InMemoryMessageRetriever,
    InMemorySessionStore,
};
use everruns_core::runtime_agent::RuntimeAgent;
use everruns_core::session::{Session, SessionStatus};
use everruns_core::traits::{ModelWithProvider, NoopEventEmitter};
use everruns_core::typed_id::{HarnessId, MessageId, SessionId, TurnId};
use everruns_core::{Message, ToolCall};
use futures::stream;
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use uuid::Uuid;

/// Create a basic test setup with in-memory stores
async fn setup_test_environment() -> (
    InMemoryHarnessStore,
    InMemoryAgentStore,
    InMemorySessionStore,
    InMemoryMessageRetriever,
    InMemoryLlmProviderStore,
    HarnessId, // harness_id
    Uuid,      // agent_id
    Uuid,      // session_id
) {
    let harness_store = InMemoryHarnessStore::new();
    let agent_store = InMemoryAgentStore::new();
    let session_store = InMemorySessionStore::new();
    let message_retriever = InMemoryMessageRetriever::new();
    let provider_store = InMemoryLlmProviderStore::new();

    // Create a test harness
    let harness_id = HarnessId::from_seed(1);
    let now = chrono::Utc::now();
    let harness = Harness {
        id: harness_id,
        name: "Test Harness".to_string(),
        description: None,
        system_prompt: "You are a helpful assistant.".to_string(),
        parent_harness_id: None,
        default_model_id: None,
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![],
        is_built_in: false,
        status: HarnessStatus::Active,
        created_at: now,
        updated_at: now,
        archived_at: None,
        deleted_at: None,
    };
    harness_store.add_harness(harness).await;

    // Create a test agent
    let agent_id = Uuid::now_v7();
    let agent = Agent {
        public_id: AgentId::from_uuid(agent_id),
        internal_id: agent_id,
        name: "Test Agent".to_string(),
        description: None,
        system_prompt: "You are a helpful assistant.".to_string(),
        capabilities: vec![],
        initial_files: vec![],
        tools: vec![],
        default_model_id: None,
        tags: vec![],
        status: AgentStatus::Active,
        created_at: now,
        updated_at: now,
        archived_at: None,
        deleted_at: None,
        usage: None,
    };
    agent_store.add_agent(agent).await;

    // Create a test session
    let session_id = Uuid::now_v7();
    let session = Session {
        id: session_id.into(),
        organization_id: "default".to_string(),
        harness_id,
        agent_id: Some(agent_id.into()),
        agent_identity_id: None,
        title: Some("Test Session".to_string()),
        locale: None,
        preview: None,
        output_preview: None,
        tags: vec![],
        status: SessionStatus::Started,
        model_id: None,
        capabilities: vec![],
        tools: vec![],
        system_prompt: None,
        initial_files: vec![],
        hints: None,
        created_at: now,
        updated_at: now,
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
    };
    session_store.add_session(session).await;

    // Set up a default model using the LlmSim provider
    let model = ModelWithProvider {
        model: "llmsim-test".to_string(),
        provider_type: LlmProviderType::LlmSim,
        api_key: Some("fake-api-key".to_string()), // Required by registry but unused by LlmSim
        base_url: None,
    };
    provider_store.set_default_model(model).await;

    (
        harness_store,
        agent_store,
        session_store,
        message_retriever,
        provider_store,
        harness_id,
        agent_id,
        session_id,
    )
}

/// Create a custom driver registry with a specific LlmSim configuration
fn create_custom_driver_registry(config: LlmSimConfig) -> DriverRegistry {
    let mut registry = DriverRegistry::new();
    registry.register(ProviderType::LlmSim, move |_api_key, _base_url| {
        Box::new(LlmSimDriver::new(config.clone()))
    });
    registry
}

/// Create an AtomContext for testing
fn create_context(session_id: Uuid) -> AtomContext {
    let turn_id = TurnId::new();
    let input_message_id = MessageId::new();
    AtomContext::new(SessionId::from_uuid(session_id), turn_id, input_message_id)
}

#[derive(Clone, Debug)]
struct FlakyStreamDriver {
    attempts: Arc<AtomicUsize>,
}

#[async_trait]
impl everruns_core::LlmDriver for FlakyStreamDriver {
    async fn chat_completion_stream(
        &self,
        _messages: Vec<everruns_core::LlmMessage>,
        config: &everruns_core::LlmCallConfig,
    ) -> everruns_core::Result<everruns_core::LlmResponseStream> {
        let attempt = self.attempts.fetch_add(1, Ordering::SeqCst);

        if attempt == 0 {
            return Ok(Box::pin(stream::iter(vec![Ok(
                everruns_core::LlmStreamEvent::Error(
                    "server_error: transient upstream failure".to_string(),
                ),
            )])));
        }

        Ok(Box::pin(stream::iter(vec![
            Ok(everruns_core::LlmStreamEvent::TextDelta(
                "Recovered after retry.".to_string(),
            )),
            Ok(everruns_core::LlmStreamEvent::Done(Box::new(
                everruns_core::LlmCompletionMetadata {
                    total_tokens: Some(8),
                    prompt_tokens: Some(5),
                    completion_tokens: Some(3),
                    model: Some(config.model.clone()),
                    finish_reason: Some("stop".to_string()),
                    ..Default::default()
                },
            ))),
        ])))
    }
}

#[tokio::test]
async fn test_reason_atom_with_fixed_response() {
    use everruns_core::memory::InMemoryEventEmitter;

    let (
        harness_store,
        agent_store,
        session_store,
        message_retriever,
        provider_store,
        harness_id,
        agent_id,
        session_id,
    ) = setup_test_environment().await;

    // Add a user message
    message_retriever
        .seed(
            session_id.into(),
            vec![Message::user("What is the capital of France?")],
        )
        .await;

    // Create a driver with a fixed response
    let driver_registry =
        create_custom_driver_registry(LlmSimConfig::fixed("The capital of France is Paris."));

    let event_emitter = InMemoryEventEmitter::new();

    let atom = ReasonAtom::new(
        harness_store,
        agent_store,
        session_store,
        message_retriever.clone(),
        provider_store,
        CapabilityRegistry::new(),
        driver_registry,
        event_emitter.clone(),
    );

    let context = create_context(session_id);
    let input = ReasonInput {
        context,
        harness_id,
        agent_id: Some(agent_id.into()),
        org_id: 0,
        mcp_tool_definitions: vec![],
        previous_response_id: None,
        iteration: 1,
    };

    let result = atom
        .execute(input)
        .await
        .expect("ReasonAtom should succeed");

    assert!(result.success);
    assert_eq!(result.text, "The capital of France is Paris.");
    assert!(!result.has_tool_calls);
    assert!(result.tool_calls.is_empty());

    // Verify the assistant message was emitted as an output.message.completed event
    // (ReasonAtom stores messages via EventEmitter, not MessageRetriever)
    let events = event_emitter.events().await;
    let output_completed = events
        .iter()
        .find(|e| e.event_type == "output.message.completed");
    assert!(
        output_completed.is_some(),
        "Should emit output.message.completed event"
    );
    if let Some(event) = output_completed {
        if let everruns_core::EventData::OutputMessageCompleted(data) = &event.data {
            assert_eq!(data.message.text(), Some("The capital of France is Paris."));
        } else {
            panic!("Expected OutputMessageCompleted data");
        }
    }
}

#[tokio::test]
async fn test_reason_atom_with_tool_calls() {
    let (
        harness_store,
        agent_store,
        session_store,
        message_retriever,
        provider_store,
        harness_id,
        agent_id,
        session_id,
    ) = setup_test_environment().await;

    // Add a user message
    message_retriever
        .seed(
            session_id.into(),
            vec![Message::user("What's the weather in Tokyo?")],
        )
        .await;

    // Create a driver that returns tool calls
    let tool_call = ToolCall {
        id: "call_weather_1".to_string(),
        name: "get_weather".to_string(),
        arguments: json!({"city": "Tokyo"}),
    };

    let driver_registry = create_custom_driver_registry(
        LlmSimConfig::fixed("Let me check the weather for you.")
            .with_tool_calls(vec![tool_call.clone()]),
    );

    let atom = ReasonAtom::new(
        harness_store,
        agent_store,
        session_store,
        message_retriever.clone(),
        provider_store,
        CapabilityRegistry::new(),
        driver_registry,
        NoopEventEmitter,
    );

    let context = create_context(session_id);
    let input = ReasonInput {
        context,
        harness_id,
        agent_id: Some(agent_id.into()),
        org_id: 0,
        mcp_tool_definitions: vec![],
        previous_response_id: None,
        iteration: 1,
    };

    let result = atom
        .execute(input)
        .await
        .expect("ReasonAtom should succeed");

    assert!(result.success);
    assert_eq!(result.text, "Let me check the weather for you.");
    assert!(result.has_tool_calls);
    assert_eq!(result.tool_calls.len(), 1);
    assert_eq!(result.tool_calls[0].name, "get_weather");
    assert_eq!(result.tool_calls[0].id, "call_weather_1");
}

#[tokio::test]
async fn test_reason_atom_with_echo_response() {
    let (
        harness_store,
        agent_store,
        session_store,
        message_retriever,
        provider_store,
        harness_id,
        agent_id,
        session_id,
    ) = setup_test_environment().await;

    // Add a user message
    message_retriever
        .seed(
            session_id.into(),
            vec![Message::user("Hello, how are you?")],
        )
        .await;

    // Create a driver that echoes the user input
    let driver_registry = create_custom_driver_registry(LlmSimConfig::echo());

    let atom = ReasonAtom::new(
        harness_store,
        agent_store,
        session_store,
        message_retriever.clone(),
        provider_store,
        CapabilityRegistry::new(),
        driver_registry,
        NoopEventEmitter,
    );

    let context = create_context(session_id);
    let input = ReasonInput {
        context,
        harness_id,
        agent_id: Some(agent_id.into()),
        org_id: 0,
        mcp_tool_definitions: vec![],
        previous_response_id: None,
        iteration: 1,
    };

    let result = atom
        .execute(input)
        .await
        .expect("ReasonAtom should succeed");

    assert!(result.success);
    assert_eq!(result.text, "Echo: Hello, how are you?");
}

#[tokio::test]
async fn test_reason_atom_with_different_configs() {
    // Test that different LlmSimConfig settings produce different results
    // Note: Sequence responses work within a single driver instance, but each
    // registry.create_driver() call creates a fresh driver. For registry-based
    // usage, use fixed responses or test sequences at the driver level.

    let (
        harness_store,
        agent_store,
        session_store,
        message_retriever,
        provider_store,
        harness_id,
        agent_id,
        session_id,
    ) = setup_test_environment().await;

    // First test with one configuration
    message_retriever
        .seed(session_id.into(), vec![Message::user("Question 1")])
        .await;

    let driver_registry1 = create_custom_driver_registry(LlmSimConfig::fixed("Response A"));

    let atom1 = ReasonAtom::new(
        harness_store.clone(),
        agent_store.clone(),
        session_store.clone(),
        message_retriever.clone(),
        provider_store.clone(),
        CapabilityRegistry::new(),
        driver_registry1,
        NoopEventEmitter,
    );

    let context1 = create_context(session_id);
    let result1 = atom1
        .execute(ReasonInput {
            context: context1,
            harness_id,
            agent_id: Some(agent_id.into()),
            org_id: 0,
            mcp_tool_definitions: vec![],
            previous_response_id: None,
            iteration: 1,
        })
        .await
        .expect("First call should succeed");

    assert_eq!(result1.text, "Response A");

    // Second test with a different configuration
    let session_id2 = Uuid::now_v7();
    let now2 = chrono::Utc::now();
    let session2 = Session {
        id: session_id2.into(),
        organization_id: "default".to_string(),
        harness_id,
        agent_id: Some(agent_id.into()),
        agent_identity_id: None,
        title: Some("Test Session 2".to_string()),
        locale: None,
        preview: None,
        output_preview: None,
        tags: vec![],
        status: SessionStatus::Started,
        model_id: None,
        capabilities: vec![],
        tools: vec![],
        system_prompt: None,
        initial_files: vec![],
        hints: None,
        created_at: now2,
        updated_at: now2,
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
    };
    session_store.add_session(session2).await;
    message_retriever
        .seed(session_id2.into(), vec![Message::user("Question 2")])
        .await;

    let driver_registry2 = create_custom_driver_registry(LlmSimConfig::fixed("Response B"));

    let atom2 = ReasonAtom::new(
        harness_store.clone(),
        agent_store.clone(),
        session_store.clone(),
        message_retriever.clone(),
        provider_store.clone(),
        CapabilityRegistry::new(),
        driver_registry2,
        NoopEventEmitter,
    );

    let context2 = create_context(session_id2);
    let result2 = atom2
        .execute(ReasonInput {
            context: context2,
            harness_id,
            agent_id: Some(agent_id.into()),
            org_id: 0,
            mcp_tool_definitions: vec![],
            previous_response_id: None,
            iteration: 1,
        })
        .await
        .expect("Second call should succeed");

    assert_eq!(result2.text, "Response B");
}

#[tokio::test]
async fn test_reason_atom_with_multi_turn_conversation() {
    use everruns_core::memory::InMemoryEventEmitter;

    let (
        harness_store,
        agent_store,
        session_store,
        message_retriever,
        provider_store,
        harness_id,
        agent_id,
        session_id,
    ) = setup_test_environment().await;

    // Seed a multi-turn conversation
    message_retriever
        .seed(
            session_id.into(),
            vec![
                Message::user("Hi, I'm Bob."),
                Message::assistant("Hello Bob! How can I help you today?"),
                Message::user("What's my name?"),
            ],
        )
        .await;

    // The LlmSim driver will receive all messages and can echo the last one
    let driver_registry =
        create_custom_driver_registry(LlmSimConfig::fixed("Your name is Bob, as you mentioned."));

    let event_emitter = InMemoryEventEmitter::new();

    let atom = ReasonAtom::new(
        harness_store,
        agent_store,
        session_store,
        message_retriever.clone(),
        provider_store,
        CapabilityRegistry::new(),
        driver_registry,
        event_emitter.clone(),
    );

    let context = create_context(session_id);
    let input = ReasonInput {
        context,
        harness_id,
        agent_id: Some(agent_id.into()),
        org_id: 0,
        mcp_tool_definitions: vec![],
        previous_response_id: None,
        iteration: 1,
    };

    let result = atom
        .execute(input)
        .await
        .expect("ReasonAtom should succeed");

    assert!(result.success);
    assert!(result.text.contains("Bob"));

    // Verify original messages still in retriever (untouched)
    let messages = message_retriever.load(session_id.into()).await.unwrap();
    assert_eq!(messages.len(), 3); // 3 original messages

    // Verify assistant response was emitted as output.message.completed event
    let events = event_emitter.events().await;
    let output_completed = events
        .iter()
        .find(|e| e.event_type == "output.message.completed");
    assert!(
        output_completed.is_some(),
        "Should emit output.message.completed for assistant response"
    );
    if let Some(event) = output_completed {
        if let everruns_core::EventData::OutputMessageCompleted(data) = &event.data {
            assert!(data.message.text().unwrap().contains("Bob"));
        } else {
            panic!("Expected OutputMessageCompleted data");
        }
    }
}

#[tokio::test]
async fn test_reason_atom_with_tool_result_continuation() {
    let (
        harness_store,
        agent_store,
        session_store,
        message_retriever,
        provider_store,
        harness_id,
        agent_id,
        session_id,
    ) = setup_test_environment().await;

    // Simulate a conversation where tool was called and result is available
    let tool_call = ToolCall {
        id: "call_123".to_string(),
        name: "get_weather".to_string(),
        arguments: json!({"city": "Tokyo"}),
    };

    message_retriever
        .seed(
            session_id.into(),
            vec![
                Message::user("What's the weather in Tokyo?"),
                Message::assistant_with_tools("Let me check that.", vec![tool_call]),
                Message::tool_result(
                    "call_123",
                    Some(json!({"temperature": 22, "condition": "sunny"})),
                    None,
                ),
            ],
        )
        .await;

    // LlmSim should now provide a response based on the tool result
    let driver_registry =
        create_custom_driver_registry(LlmSimConfig::fixed("It's 22\u{00b0}C and sunny in Tokyo!"));

    let atom = ReasonAtom::new(
        harness_store,
        agent_store,
        session_store,
        message_retriever.clone(),
        provider_store,
        CapabilityRegistry::new(),
        driver_registry,
        NoopEventEmitter,
    );

    let context = create_context(session_id);
    let input = ReasonInput {
        context,
        harness_id,
        agent_id: Some(agent_id.into()),
        org_id: 0,
        mcp_tool_definitions: vec![],
        previous_response_id: None,
        iteration: 1,
    };

    let result = atom
        .execute(input)
        .await
        .expect("ReasonAtom should succeed");

    assert!(result.success);
    assert!(result.text.contains("22"));
    assert!(!result.has_tool_calls);
}

#[tokio::test]
async fn test_reason_atom_with_lorem_response() {
    let (
        harness_store,
        agent_store,
        session_store,
        message_retriever,
        provider_store,
        harness_id,
        agent_id,
        session_id,
    ) = setup_test_environment().await;

    message_retriever
        .seed(
            session_id.into(),
            vec![Message::user("Tell me a long story")],
        )
        .await;

    // Use lorem ipsum generator
    let driver_registry = create_custom_driver_registry(LlmSimConfig::lorem(100));

    let atom = ReasonAtom::new(
        harness_store,
        agent_store,
        session_store,
        message_retriever.clone(),
        provider_store,
        CapabilityRegistry::new(),
        driver_registry,
        NoopEventEmitter,
    );

    let context = create_context(session_id);
    let input = ReasonInput {
        context,
        harness_id,
        agent_id: Some(agent_id.into()),
        org_id: 0,
        mcp_tool_definitions: vec![],
        previous_response_id: None,
        iteration: 1,
    };

    let result = atom
        .execute(input)
        .await
        .expect("ReasonAtom should succeed");

    assert!(result.success);
    // Lorem ipsum should generate substantial text
    assert!(result.text.len() > 50);
    assert!(result.text.split_whitespace().count() > 10);
}

#[tokio::test]
async fn test_reason_atom_handles_llm_error() {
    use everruns_core::memory::InMemoryEventEmitter;

    let (
        harness_store,
        agent_store,
        session_store,
        message_retriever,
        provider_store,
        harness_id,
        agent_id,
        session_id,
    ) = setup_test_environment().await;

    // Add a user message
    message_retriever
        .seed(session_id.into(), vec![Message::user("Hello!")])
        .await;

    // Create a driver that returns an error (simulating API key missing, rate limit, etc.)
    let driver_registry = create_custom_driver_registry(LlmSimConfig::error("API key is required"));

    // Use an in-memory event emitter to capture events
    let event_emitter = InMemoryEventEmitter::new();

    let atom = ReasonAtom::new(
        harness_store,
        agent_store,
        session_store,
        message_retriever.clone(),
        provider_store,
        CapabilityRegistry::new(),
        driver_registry,
        event_emitter.clone(),
    );

    let context = create_context(session_id);
    let input = ReasonInput {
        context,
        harness_id,
        agent_id: Some(agent_id.into()),
        org_id: 0,
        mcp_tool_definitions: vec![],
        previous_response_id: None,
        iteration: 1,
    };

    let result = atom
        .execute(input)
        .await
        .expect("ReasonAtom should handle error gracefully");

    // Verify the result indicates failure
    assert!(!result.success, "Result should indicate failure");
    assert!(
        result.error.is_some(),
        "Result should contain error message"
    );
    assert!(
        result.error.as_ref().unwrap().contains("API key"),
        "Error should mention API key"
    );

    // Verify user-friendly error message is returned (not internal details)
    assert!(
        result.text.contains("error"),
        "User-facing text should mention error"
    );
    assert!(
        result.text.contains("Please try again"),
        "User-facing text should be friendly"
    );

    // Verify no tool calls
    assert!(!result.has_tool_calls);
    assert!(result.tool_calls.is_empty());

    // Verify events were emitted
    let events = event_emitter.events().await;
    assert!(!events.is_empty(), "Events should have been emitted");

    // Check for output.message.completed event (error message for user)
    let has_output_message = events
        .iter()
        .any(|e| e.event_type == "output.message.completed");
    assert!(
        has_output_message,
        "Should emit output.message.completed event for error"
    );

    // Check for reason.completed event with success=false
    let reason_completed = events.iter().find(|e| e.event_type == "reason.completed");
    assert!(
        reason_completed.is_some(),
        "Should emit reason.completed event"
    );
}

#[tokio::test]
async fn test_reason_atom_emits_output_message_completed_on_success() {
    use everruns_core::memory::InMemoryEventEmitter;

    let (
        harness_store,
        agent_store,
        session_store,
        message_retriever,
        provider_store,
        harness_id,
        agent_id,
        session_id,
    ) = setup_test_environment().await;

    // Add a user message
    message_retriever
        .seed(
            session_id.into(),
            vec![Message::user("What is the capital of France?")],
        )
        .await;

    // Create a driver with a fixed response
    let driver_registry =
        create_custom_driver_registry(LlmSimConfig::fixed("The capital of France is Paris."));

    // Use an in-memory event emitter to capture events
    let event_emitter = InMemoryEventEmitter::new();

    let atom = ReasonAtom::new(
        harness_store,
        agent_store,
        session_store,
        message_retriever.clone(),
        provider_store,
        CapabilityRegistry::new(),
        driver_registry,
        event_emitter.clone(),
    );

    let context = create_context(session_id);
    let input = ReasonInput {
        context,
        harness_id,
        agent_id: Some(agent_id.into()),
        org_id: 0,
        mcp_tool_definitions: vec![],
        previous_response_id: None,
        iteration: 1,
    };

    let result = atom
        .execute(input)
        .await
        .expect("ReasonAtom should succeed");

    assert!(result.success);
    assert_eq!(result.text, "The capital of France is Paris.");

    // Verify events were emitted
    let events = event_emitter.events().await;
    assert!(!events.is_empty(), "Events should have been emitted");

    // Check for output.message.started event
    let has_output_started = events
        .iter()
        .any(|e| e.event_type == "output.message.started");
    assert!(
        has_output_started,
        "Should emit output.message.started event"
    );

    // Check for output.message.completed event (the final message)
    let output_completed = events
        .iter()
        .find(|e| e.event_type == "output.message.completed");
    assert!(
        output_completed.is_some(),
        "Should emit output.message.completed event on success"
    );

    // Verify the completed event has the correct message
    if let Some(event) = output_completed {
        if let everruns_core::EventData::OutputMessageCompleted(data) = &event.data {
            assert_eq!(data.message.text(), Some("The capital of France is Paris."));
            assert_eq!(data.message.role, everruns_core::MessageRole::Agent);
        } else {
            panic!("Expected OutputMessageCompleted data");
        }
    }

    // Check for reason.started and reason.completed events
    let has_reason_started = events.iter().any(|e| e.event_type == "reason.started");
    assert!(has_reason_started, "Should emit reason.started event");

    let reason_completed = events.iter().find(|e| e.event_type == "reason.completed");
    assert!(
        reason_completed.is_some(),
        "Should emit reason.completed event"
    );

    // Verify reason.completed shows success
    if let Some(event) = reason_completed
        && let everruns_core::EventData::ReasonCompleted(data) = &event.data
    {
        assert!(data.success, "reason.completed should indicate success");
    }

    // Check for llm.generation event
    let has_llm_generation = events.iter().any(|e| e.event_type == "llm.generation");
    assert!(has_llm_generation, "Should emit llm.generation event");
}

#[tokio::test]
async fn test_reason_atom_does_not_retry_transient_stream_error() {
    // Stream-level errors are NOT retried at the atom level.
    // Transient retries happen at the driver level (HTTP 429/5xx).
    // The atom reports the error as a failure to avoid duplicate user-visible messages.
    use everruns_core::memory::InMemoryEventEmitter;

    let (
        harness_store,
        agent_store,
        session_store,
        message_retriever,
        provider_store,
        harness_id,
        agent_id,
        session_id,
    ) = setup_test_environment().await;

    message_retriever
        .seed(session_id.into(), vec![Message::user("Hello!")])
        .await;

    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_for_registry = Arc::clone(&attempts);
    let mut driver_registry = DriverRegistry::new();
    driver_registry.register(ProviderType::LlmSim, move |_api_key, _base_url| {
        Box::new(FlakyStreamDriver {
            attempts: Arc::clone(&attempts_for_registry),
        })
    });

    let event_emitter = InMemoryEventEmitter::new();

    let atom = ReasonAtom::new(
        harness_store,
        agent_store,
        session_store,
        message_retriever.clone(),
        provider_store,
        CapabilityRegistry::new(),
        driver_registry,
        event_emitter.clone(),
    );

    let context = create_context(session_id);
    let input = ReasonInput {
        context,
        harness_id,
        agent_id: Some(agent_id.into()),
        org_id: 0,
        mcp_tool_definitions: vec![],
        previous_response_id: None,
        iteration: 1,
    };

    let result = atom
        .execute(input)
        .await
        .expect("ReasonAtom should return Ok with failure result");

    assert!(
        !result.success,
        "Stream error should not be retried at atom level"
    );
    // Only one attempt — no retry
    assert_eq!(attempts.load(Ordering::SeqCst), 1);

    let events = event_emitter.events().await;
    let llm_event = events
        .iter()
        .find(|e| e.event_type == "llm.generation")
        .expect("llm.generation event should be emitted");

    if let everruns_core::EventData::LlmGeneration(data) = &llm_event.data {
        assert!(!data.metadata.success, "Should report failure");
    } else {
        panic!("Expected llm.generation event data");
    }
}

#[tokio::test]
async fn test_driver_registry_integration() {
    // Verify that register_driver works with the standard DriverRegistry flow
    let mut registry = DriverRegistry::new();
    register_driver(&mut registry);

    assert!(registry.has_driver(&ProviderType::LlmSim));

    // Create driver via registry
    let config = everruns_core::llm_driver_registry::ProviderConfig::new(ProviderType::LlmSim)
        .with_api_key("test-key");

    let driver = registry
        .create_driver(&config)
        .expect("Should create LlmSim driver");

    // Test the driver
    use everruns_core::llm_driver_registry::{
        LlmCallConfig, LlmDriver, LlmMessage, LlmMessageRole,
    };

    let messages = vec![LlmMessage::text(LlmMessageRole::User, "Hello")];
    let call_config = LlmCallConfig {
        model: "test".to_string(),
        temperature: None,
        max_tokens: None,
        tools: vec![],
        reasoning_effort: None,
        metadata: std::collections::HashMap::new(),
        previous_response_id: None,
        tool_search: None,
    };

    let response = driver
        .chat_completion(messages, &call_config)
        .await
        .expect("Chat completion should succeed");

    // Default driver returns a fixed response
    assert!(!response.text.is_empty());
}

#[tokio::test]
async fn test_reason_atom_handles_model_not_available() {
    use everruns_core::memory::InMemoryEventEmitter;

    let (
        harness_store,
        agent_store,
        session_store,
        message_retriever,
        provider_store,
        harness_id,
        agent_id,
        session_id,
    ) = setup_test_environment().await;

    // Add a user message
    message_retriever
        .seed(session_id.into(), vec![Message::user("Hello!")])
        .await;

    // Create a driver that returns a model-not-available error
    let driver_registry = create_custom_driver_registry(LlmSimConfig::model_not_available());

    let event_emitter = InMemoryEventEmitter::new();

    let atom = ReasonAtom::new(
        harness_store,
        agent_store,
        session_store,
        message_retriever.clone(),
        provider_store,
        CapabilityRegistry::new(),
        driver_registry,
        event_emitter.clone(),
    );

    let context = create_context(session_id);
    let input = ReasonInput {
        context,
        harness_id,
        agent_id: Some(agent_id.into()),
        org_id: 0,
        mcp_tool_definitions: vec![],
        previous_response_id: None,
        iteration: 1,
    };

    let result = atom
        .execute(input)
        .await
        .expect("ReasonAtom should handle model-not-available gracefully");

    // Verify the result indicates failure
    assert!(!result.success, "Result should indicate failure");
    assert!(
        result.error.is_some(),
        "Result should contain error message"
    );
    assert!(
        result
            .error
            .as_ref()
            .unwrap()
            .contains("Model not available"),
        "Error should mention model not available: {}",
        result.error.as_ref().unwrap()
    );

    // Verify user-friendly error message mentions the model and suggests action
    assert!(
        result.text.contains("not available"),
        "User-facing text should mention model not available: {}",
        result.text
    );
    assert!(
        result.text.contains("select a different model"),
        "User-facing text should suggest selecting a different model: {}",
        result.text
    );

    // Verify no tool calls
    assert!(!result.has_tool_calls);
    assert!(result.tool_calls.is_empty());

    // Verify events were emitted
    let events = event_emitter.events().await;
    assert!(!events.is_empty(), "Events should have been emitted");

    // Check for output.message.completed event (error message for user)
    let output_msg = events
        .iter()
        .find(|e| e.event_type == "output.message.completed");
    assert!(
        output_msg.is_some(),
        "Should emit output.message.completed event for error"
    );
    if let Some(event) = output_msg {
        if let everruns_core::EventData::OutputMessageCompleted(data) = &event.data {
            let text = data.message.text().unwrap_or_default();
            assert!(
                text.contains("not available"),
                "Output message should mention model not available: {}",
                text
            );
        } else {
            panic!("Expected OutputMessageCompleted data");
        }
    }

    // Check for reason.completed event with success=false
    let reason_completed = events.iter().find(|e| e.event_type == "reason.completed");
    assert!(
        reason_completed.is_some(),
        "Should emit reason.completed event"
    );
}

// ============================================================================
// response_id / previous_response_id chaining tests
// ============================================================================

#[tokio::test]
async fn test_reason_atom_returns_response_id_from_driver() {
    let (
        harness_store,
        agent_store,
        session_store,
        message_retriever,
        provider_store,
        harness_id,
        agent_id,
        session_id,
    ) = setup_test_environment().await;

    message_retriever
        .seed(session_id.into(), vec![Message::user("Hello")])
        .await;

    // Configure driver to return a response_id
    let config = LlmSimConfig::fixed("Hello from response-id test").with_response_id("resp_abc123");
    let driver_registry = create_custom_driver_registry(config);
    let atom = ReasonAtom::new(
        harness_store,
        agent_store,
        session_store,
        message_retriever.clone(),
        provider_store,
        CapabilityRegistry::new(),
        driver_registry,
        NoopEventEmitter,
    );

    let context = create_context(session_id);
    let input = ReasonInput {
        context,
        harness_id,
        agent_id: Some(agent_id.into()),
        org_id: 0,
        mcp_tool_definitions: vec![],
        previous_response_id: None,
        iteration: 1,
    };

    let result = atom
        .execute(input)
        .await
        .expect("ReasonAtom should succeed");

    assert!(result.success);
    assert_eq!(
        result.response_id.as_deref(),
        Some("resp_abc123"),
        "ReasonResult should carry the driver's response_id"
    );
}

#[tokio::test]
async fn test_reason_atom_response_id_none_when_driver_omits_it() {
    let (
        harness_store,
        agent_store,
        session_store,
        message_retriever,
        provider_store,
        harness_id,
        agent_id,
        session_id,
    ) = setup_test_environment().await;

    message_retriever
        .seed(session_id.into(), vec![Message::user("Hello")])
        .await;

    // Default driver has no response_id
    let config = LlmSimConfig::fixed("No response id");
    let driver_registry = create_custom_driver_registry(config);

    let atom = ReasonAtom::new(
        harness_store,
        agent_store,
        session_store,
        message_retriever.clone(),
        provider_store,
        CapabilityRegistry::new(),
        driver_registry,
        NoopEventEmitter,
    );

    let context = create_context(session_id);
    let input = ReasonInput {
        context,
        harness_id,
        agent_id: Some(agent_id.into()),
        org_id: 0,
        mcp_tool_definitions: vec![],
        previous_response_id: None,
        iteration: 1,
    };

    let result = atom
        .execute(input)
        .await
        .expect("ReasonAtom should succeed");

    assert!(result.success);
    assert_eq!(
        result.response_id, None,
        "ReasonResult.response_id should be None when driver omits it"
    );
}

#[tokio::test]
async fn test_previous_response_id_round_trips_through_serde() {
    // ReasonInput with previous_response_id
    let input = ReasonInput {
        context: AtomContext::new(SessionId::new(), TurnId::new(), MessageId::new()),
        harness_id: HarnessId::new(),
        agent_id: None,
        org_id: 0,
        mcp_tool_definitions: vec![],
        previous_response_id: Some("resp_xyz789".to_string()),
        iteration: 1,
    };
    let json = serde_json::to_value(&input).unwrap();
    assert_eq!(json["previous_response_id"], "resp_xyz789");
    let deserialized: ReasonInput = serde_json::from_value(json).unwrap();
    assert_eq!(
        deserialized.previous_response_id.as_deref(),
        Some("resp_xyz789")
    );

    // ReasonInput without previous_response_id (omitted via skip_serializing_if)
    let input_none = ReasonInput {
        context: AtomContext::new(SessionId::new(), TurnId::new(), MessageId::new()),
        harness_id: HarnessId::new(),
        agent_id: None,
        org_id: 0,
        mcp_tool_definitions: vec![],
        previous_response_id: None,
        iteration: 1,
    };
    let json_none = serde_json::to_value(&input_none).unwrap();
    assert!(
        json_none.get("previous_response_id").is_none(),
        "None should be omitted from serialization"
    );

    // ReasonResult with response_id
    let result = everruns_core::atoms::ReasonResult {
        text: "test".to_string(),
        tool_calls: vec![],
        tool_definitions: vec![],
        has_tool_calls: false,
        success: true,
        max_iterations: 10,
        error: None,
        usage: None,
        locale: None,
        response_id: Some("resp_out_456".to_string()),
    };
    let result_json = serde_json::to_value(&result).unwrap();
    assert_eq!(result_json["response_id"], "resp_out_456");
    let result_rt: everruns_core::atoms::ReasonResult =
        serde_json::from_value(result_json).unwrap();
    assert_eq!(result_rt.response_id.as_deref(), Some("resp_out_456"));
}

#[tokio::test]
async fn test_llm_call_config_previous_response_id() {
    let agent = RuntimeAgent::new("test prompt", "test-model");

    // Builder sets previous_response_id
    let config = LlmCallConfigBuilder::from(&agent)
        .previous_response_id(Some("resp_prev_001".to_string()))
        .build();
    assert_eq!(
        config.previous_response_id.as_deref(),
        Some("resp_prev_001")
    );

    // Builder defaults to None
    let config_default = LlmCallConfigBuilder::from(&agent).build();
    assert_eq!(config_default.previous_response_id, None);
}

// ============================================================================
// Trailing stream error with partial output (OpenAI server_error after tool calls)
// ============================================================================

/// Driver that emits tool calls followed by a trailing error event.
/// Simulates OpenAI Responses API behaviour where a server_error can
/// follow fully-streamed function calls.
#[derive(Clone, Debug)]
struct ToolCallsThenErrorDriver;

#[async_trait]
impl everruns_core::LlmDriver for ToolCallsThenErrorDriver {
    async fn chat_completion_stream(
        &self,
        _messages: Vec<everruns_core::LlmMessage>,
        _config: &everruns_core::LlmCallConfig,
    ) -> everruns_core::Result<everruns_core::LlmResponseStream> {
        Ok(Box::pin(stream::iter(vec![
            // Tool calls arrive first (fully streamed)
            Ok(everruns_core::LlmStreamEvent::ToolCalls(vec![ToolCall {
                id: "call_session_1".to_string(),
                name: "manage_sessions".to_string(),
                arguments: json!({"operation": "create", "agent_id": "agent_123"}),
            }])),
            // Trailing server error after valid output
            Ok(everruns_core::LlmStreamEvent::Error(
                "server_error: An error occurred while processing your request.".to_string(),
            )),
        ])))
    }
}

#[tokio::test]
async fn test_reason_atom_preserves_tool_calls_on_trailing_stream_error() {
    use everruns_core::memory::InMemoryEventEmitter;

    let (
        harness_store,
        agent_store,
        session_store,
        message_retriever,
        provider_store,
        harness_id,
        agent_id,
        session_id,
    ) = setup_test_environment().await;

    message_retriever
        .seed(session_id.into(), vec![Message::user("Run builder agent")])
        .await;

    let mut driver_registry = DriverRegistry::new();
    driver_registry.register(ProviderType::LlmSim, |_api_key, _base_url| {
        Box::new(ToolCallsThenErrorDriver)
    });

    let event_emitter = InMemoryEventEmitter::new();

    let atom = ReasonAtom::new(
        harness_store,
        agent_store,
        session_store,
        message_retriever.clone(),
        provider_store,
        CapabilityRegistry::new(),
        driver_registry,
        event_emitter.clone(),
    );

    let context = create_context(session_id);
    let input = ReasonInput {
        context,
        harness_id,
        agent_id: Some(agent_id.into()),
        org_id: 0,
        mcp_tool_definitions: vec![],
        previous_response_id: None,
        iteration: 1,
    };

    let result = atom
        .execute(input)
        .await
        .expect("ReasonAtom should return Ok for partial success");

    // Tool calls should be preserved despite the trailing error
    assert!(
        result.success,
        "Partial success with tool calls should be treated as success"
    );
    assert!(result.has_tool_calls, "Tool calls should be present");
    assert_eq!(result.tool_calls.len(), 1);
    assert_eq!(result.tool_calls[0].name, "manage_sessions");
    assert_eq!(result.tool_calls[0].id, "call_session_1");

    // No response_id since we never got a Done event
    assert!(result.response_id.is_none());
}

/// Driver that emits text content followed by a trailing error event.
#[derive(Clone, Debug)]
struct TextThenErrorDriver;

#[async_trait]
impl everruns_core::LlmDriver for TextThenErrorDriver {
    async fn chat_completion_stream(
        &self,
        _messages: Vec<everruns_core::LlmMessage>,
        _config: &everruns_core::LlmCallConfig,
    ) -> everruns_core::Result<everruns_core::LlmResponseStream> {
        Ok(Box::pin(stream::iter(vec![
            Ok(everruns_core::LlmStreamEvent::TextDelta(
                "Here are the links:\n\n- Research Agent:".to_string(),
            )),
            Ok(everruns_core::LlmStreamEvent::Error(
                "server_error: internal failure".to_string(),
            )),
        ])))
    }
}

#[tokio::test]
async fn test_reason_atom_preserves_text_on_trailing_stream_error() {
    use everruns_core::memory::InMemoryEventEmitter;

    let (
        harness_store,
        agent_store,
        session_store,
        message_retriever,
        provider_store,
        harness_id,
        agent_id,
        session_id,
    ) = setup_test_environment().await;

    message_retriever
        .seed(session_id.into(), vec![Message::user("Give me links")])
        .await;

    let mut driver_registry = DriverRegistry::new();
    driver_registry.register(ProviderType::LlmSim, |_api_key, _base_url| {
        Box::new(TextThenErrorDriver)
    });

    let event_emitter = InMemoryEventEmitter::new();

    let atom = ReasonAtom::new(
        harness_store,
        agent_store,
        session_store,
        message_retriever.clone(),
        provider_store,
        CapabilityRegistry::new(),
        driver_registry,
        event_emitter.clone(),
    );

    let context = create_context(session_id);
    let input = ReasonInput {
        context,
        harness_id,
        agent_id: Some(agent_id.into()),
        org_id: 0,
        mcp_tool_definitions: vec![],
        previous_response_id: None,
        iteration: 1,
    };

    let result = atom
        .execute(input)
        .await
        .expect("ReasonAtom should return Ok for partial success");

    // Text should be preserved despite the trailing error
    assert!(
        result.success,
        "Partial success with text should be treated as success"
    );
    assert!(
        result.text.contains("Research Agent"),
        "Partial text should be preserved: got '{}'",
        result.text
    );
    assert!(!result.has_tool_calls);
}

/// Driver that emits an error without any prior output (pure failure).
/// This should still fail as before — no partial output to recover.
#[derive(Clone, Debug)]
struct PureErrorDriver;

#[async_trait]
impl everruns_core::LlmDriver for PureErrorDriver {
    async fn chat_completion_stream(
        &self,
        _messages: Vec<everruns_core::LlmMessage>,
        _config: &everruns_core::LlmCallConfig,
    ) -> everruns_core::Result<everruns_core::LlmResponseStream> {
        Ok(Box::pin(stream::iter(vec![Ok(
            everruns_core::LlmStreamEvent::Error("server_error: total failure".to_string()),
        )])))
    }
}

#[tokio::test]
async fn test_reason_atom_still_fails_on_pure_stream_error() {
    use everruns_core::memory::InMemoryEventEmitter;

    let (
        harness_store,
        agent_store,
        session_store,
        message_retriever,
        provider_store,
        harness_id,
        agent_id,
        session_id,
    ) = setup_test_environment().await;

    message_retriever
        .seed(session_id.into(), vec![Message::user("Hello!")])
        .await;

    let mut driver_registry = DriverRegistry::new();
    driver_registry.register(ProviderType::LlmSim, |_api_key, _base_url| {
        Box::new(PureErrorDriver)
    });

    let event_emitter = InMemoryEventEmitter::new();

    let atom = ReasonAtom::new(
        harness_store,
        agent_store,
        session_store,
        message_retriever.clone(),
        provider_store,
        CapabilityRegistry::new(),
        driver_registry,
        event_emitter.clone(),
    );

    let context = create_context(session_id);
    let input = ReasonInput {
        context,
        harness_id,
        agent_id: Some(agent_id.into()),
        org_id: 0,
        mcp_tool_definitions: vec![],
        previous_response_id: None,
        iteration: 1,
    };

    let result = atom
        .execute(input)
        .await
        .expect("ReasonAtom should handle pure error gracefully");

    // Pure error (no partial output) should still fail
    assert!(
        !result.success,
        "Pure stream error should still be a failure"
    );
    assert!(result.error.is_some());
    assert!(!result.has_tool_calls);
}

// ============================================================================
// Error placeholder message stripping
// ============================================================================

#[tokio::test]
async fn test_reason_atom_strips_error_placeholder_messages() {
    use everruns_core::memory::InMemoryEventEmitter;

    let (
        harness_store,
        agent_store,
        session_store,
        message_retriever,
        provider_store,
        harness_id,
        agent_id,
        session_id,
    ) = setup_test_environment().await;

    // Seed with a conversation that includes error placeholder messages
    // (simulates accumulated DLQ errors from prior failed turns)
    message_retriever
        .seed(
            session_id.into(),
            vec![
                Message::user("Create agents for me"),
                Message::assistant(
                    "I encountered an error while processing your request. Please try again later.",
                ),
                Message::assistant(
                    "I encountered an error while processing your request. Please try again later.",
                ),
                Message::assistant(
                    "I encountered an error while processing your request. Please try again later.",
                ),
                Message::user("Try again"),
            ],
        )
        .await;

    // Use echo driver to see what the LLM receives
    let driver_registry = create_custom_driver_registry(LlmSimConfig::echo());

    let event_emitter = InMemoryEventEmitter::new();

    let atom = ReasonAtom::new(
        harness_store,
        agent_store,
        session_store,
        message_retriever.clone(),
        provider_store,
        CapabilityRegistry::new(),
        driver_registry,
        event_emitter.clone(),
    );

    let context = create_context(session_id);
    let input = ReasonInput {
        context,
        harness_id,
        agent_id: Some(agent_id.into()),
        org_id: 0,
        mcp_tool_definitions: vec![],
        previous_response_id: None,
        iteration: 1,
    };

    let result = atom
        .execute(input)
        .await
        .expect("ReasonAtom should succeed");

    assert!(result.success);

    // The echo driver echoes back the user messages it received.
    // The error placeholder messages should NOT appear in the echo output
    // because they were stripped before sending to the LLM.
    assert!(
        !result.text.contains("I encountered an error"),
        "Error placeholder messages should be stripped from LLM input, got: '{}'",
        result.text
    );
}
