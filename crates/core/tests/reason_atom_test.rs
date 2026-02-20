// Integration tests for ReasonAtom with LlmSimDriver
//
// These tests verify the full ReasonAtom workflow using the simulated LLM driver,
// enabling deterministic testing without real LLM API calls.
//
// Run with: cargo test -p everruns-core --test reason_atom_test

use everruns_core::AgentId;
use everruns_core::MessageRetriever;
use everruns_core::agent::{Agent, AgentStatus};
use everruns_core::atoms::{Atom, AtomContext, ReasonAtom, ReasonInput};
use everruns_core::capabilities::CapabilityRegistry;
use everruns_core::harness::{Harness, HarnessStatus};
use everruns_core::llm_driver_registry::{DriverRegistry, ProviderType};
use everruns_core::llm_models::LlmProviderType;
use everruns_core::llmsim_driver::{LlmSimConfig, LlmSimDriver, register_driver};
use everruns_core::memory::{
    InMemoryAgentStore, InMemoryHarnessStore, InMemoryLlmProviderStore, InMemoryMessageRetriever,
    InMemorySessionStore,
};
use everruns_core::session::{Session, SessionStatus};
use everruns_core::traits::{ModelWithProvider, NoopEventEmitter};
use everruns_core::typed_id::{HarnessId, MessageId, SessionId, TurnId};
use everruns_core::{Message, ToolCall};
use serde_json::json;
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
        default_model_id: None,
        tags: vec![],
        capabilities: vec![],
        status: HarnessStatus::Active,
        created_at: now,
        updated_at: now,
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
        tools: vec![],
        default_model_id: None,
        tags: vec![],
        status: AgentStatus::Active,
        created_at: now,
        updated_at: now,
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
        title: Some("Test Session".to_string()),
        preview: None,
        output_preview: None,
        tags: vec![],
        status: SessionStatus::Started,
        model_id: None,
        capabilities: vec![],
        tools: vec![],
        created_at: now,
        updated_at: now,
        started_at: None,
        finished_at: None,
        usage: None,
        is_pinned: None,
        active_schedule_count: None,
        features: vec![],
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
        title: Some("Test Session 2".to_string()),
        preview: None,
        output_preview: None,
        tags: vec![],
        status: SessionStatus::Started,
        model_id: None,
        capabilities: vec![],
        tools: vec![],
        created_at: now2,
        updated_at: now2,
        started_at: None,
        finished_at: None,
        usage: None,
        is_pinned: None,
        active_schedule_count: None,
        features: vec![],
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
