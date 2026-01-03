// Integration tests for ReasonAtom with LlmSimDriver
//
// These tests verify the full ReasonAtom workflow using the simulated LLM driver,
// enabling deterministic testing without real LLM API calls.
//
// Run with: cargo test -p everruns-core --test reason_atom_test

use everruns_core::agent::{Agent, AgentStatus};
use everruns_core::atoms::{Atom, AtomContext, ReasonAtom, ReasonInput};
use everruns_core::capabilities::CapabilityRegistry;
use everruns_core::llm_driver_registry::{DriverRegistry, ProviderType};
use everruns_core::llm_models::LlmProviderType;
use everruns_core::llmsim_driver::{register_driver, LlmSimConfig, LlmSimDriver};
use everruns_core::memory::{
    InMemoryAgentStore, InMemoryLlmProviderStore, InMemoryMessageStore, InMemorySessionStore,
};
use everruns_core::session::{Session, SessionStatus};
use everruns_core::traits::{MessageStore, ModelWithProvider, NoopEventEmitter};
use everruns_core::{Message, ToolCall};
use serde_json::json;
use uuid::Uuid;

/// Create a basic test setup with in-memory stores
async fn setup_test_environment() -> (
    InMemoryAgentStore,
    InMemorySessionStore,
    InMemoryMessageStore,
    InMemoryLlmProviderStore,
    Uuid, // agent_id
    Uuid, // session_id
) {
    let agent_store = InMemoryAgentStore::new();
    let session_store = InMemorySessionStore::new();
    let message_store = InMemoryMessageStore::new();
    let provider_store = InMemoryLlmProviderStore::new();

    // Create a test agent
    let agent_id = Uuid::now_v7();
    let agent = Agent {
        id: agent_id,
        name: "Test Agent".to_string(),
        description: None,
        system_prompt: "You are a helpful assistant.".to_string(),
        capabilities: vec![],
        default_model_id: None,
        tags: vec![],
        status: AgentStatus::Active,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    agent_store.add_agent(agent).await;

    // Create a test session
    let session_id = Uuid::now_v7();
    let session = Session {
        id: session_id,
        agent_id,
        title: Some("Test Session".to_string()),
        tags: vec![],
        status: SessionStatus::Pending,
        model_id: None,
        created_at: chrono::Utc::now(),
        started_at: None,
        finished_at: None,
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
        agent_store,
        session_store,
        message_store,
        provider_store,
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
    let turn_id = Uuid::now_v7();
    let input_message_id = Uuid::now_v7();
    AtomContext::new(session_id, turn_id, input_message_id)
}

#[tokio::test]
async fn test_reason_atom_with_fixed_response() {
    let (agent_store, session_store, message_store, provider_store, agent_id, session_id) =
        setup_test_environment().await;

    // Add a user message
    message_store
        .seed(
            session_id,
            vec![Message::user("What is the capital of France?")],
        )
        .await;

    // Create a driver with a fixed response
    let driver_registry =
        create_custom_driver_registry(LlmSimConfig::fixed("The capital of France is Paris."));

    let atom = ReasonAtom::new(
        agent_store,
        session_store,
        message_store.clone(),
        provider_store,
        CapabilityRegistry::new(),
        driver_registry,
        NoopEventEmitter,
    );

    let context = create_context(session_id);
    let input = ReasonInput { context, agent_id };

    let result = atom
        .execute(input)
        .await
        .expect("ReasonAtom should succeed");

    assert!(result.success);
    assert_eq!(result.text, "The capital of France is Paris.");
    assert!(!result.has_tool_calls);
    assert!(result.tool_calls.is_empty());

    // Verify the assistant message was stored
    let messages = message_store.load(session_id).await.unwrap();
    assert_eq!(messages.len(), 2); // user + assistant
    assert_eq!(messages[1].text(), Some("The capital of France is Paris."));
}

#[tokio::test]
async fn test_reason_atom_with_tool_calls() {
    let (agent_store, session_store, message_store, provider_store, agent_id, session_id) =
        setup_test_environment().await;

    // Add a user message
    message_store
        .seed(
            session_id,
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
        agent_store,
        session_store,
        message_store.clone(),
        provider_store,
        CapabilityRegistry::new(),
        driver_registry,
        NoopEventEmitter,
    );

    let context = create_context(session_id);
    let input = ReasonInput { context, agent_id };

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
    let (agent_store, session_store, message_store, provider_store, agent_id, session_id) =
        setup_test_environment().await;

    // Add a user message
    message_store
        .seed(session_id, vec![Message::user("Hello, how are you?")])
        .await;

    // Create a driver that echoes the user input
    let driver_registry = create_custom_driver_registry(LlmSimConfig::echo());

    let atom = ReasonAtom::new(
        agent_store,
        session_store,
        message_store.clone(),
        provider_store,
        CapabilityRegistry::new(),
        driver_registry,
        NoopEventEmitter,
    );

    let context = create_context(session_id);
    let input = ReasonInput { context, agent_id };

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

    let (agent_store, session_store, message_store, provider_store, agent_id, session_id) =
        setup_test_environment().await;

    // First test with one configuration
    message_store
        .seed(session_id, vec![Message::user("Question 1")])
        .await;

    let driver_registry1 = create_custom_driver_registry(LlmSimConfig::fixed("Response A"));

    let atom1 = ReasonAtom::new(
        agent_store.clone(),
        session_store.clone(),
        message_store.clone(),
        provider_store.clone(),
        CapabilityRegistry::new(),
        driver_registry1,
        NoopEventEmitter,
    );

    let context1 = create_context(session_id);
    let result1 = atom1
        .execute(ReasonInput {
            context: context1,
            agent_id,
        })
        .await
        .expect("First call should succeed");

    assert_eq!(result1.text, "Response A");

    // Second test with a different configuration
    let session_id2 = Uuid::now_v7();
    let session2 = Session {
        id: session_id2,
        agent_id,
        title: Some("Test Session 2".to_string()),
        tags: vec![],
        status: SessionStatus::Pending,
        model_id: None,
        created_at: chrono::Utc::now(),
        started_at: None,
        finished_at: None,
    };
    session_store.add_session(session2).await;
    message_store
        .seed(session_id2, vec![Message::user("Question 2")])
        .await;

    let driver_registry2 = create_custom_driver_registry(LlmSimConfig::fixed("Response B"));

    let atom2 = ReasonAtom::new(
        agent_store.clone(),
        session_store.clone(),
        message_store.clone(),
        provider_store.clone(),
        CapabilityRegistry::new(),
        driver_registry2,
        NoopEventEmitter,
    );

    let context2 = create_context(session_id2);
    let result2 = atom2
        .execute(ReasonInput {
            context: context2,
            agent_id,
        })
        .await
        .expect("Second call should succeed");

    assert_eq!(result2.text, "Response B");
}

#[tokio::test]
async fn test_reason_atom_with_multi_turn_conversation() {
    let (agent_store, session_store, message_store, provider_store, agent_id, session_id) =
        setup_test_environment().await;

    // Seed a multi-turn conversation
    message_store
        .seed(
            session_id,
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

    let atom = ReasonAtom::new(
        agent_store,
        session_store,
        message_store.clone(),
        provider_store,
        CapabilityRegistry::new(),
        driver_registry,
        NoopEventEmitter,
    );

    let context = create_context(session_id);
    let input = ReasonInput { context, agent_id };

    let result = atom
        .execute(input)
        .await
        .expect("ReasonAtom should succeed");

    assert!(result.success);
    assert!(result.text.contains("Bob"));

    // Verify all messages are preserved
    let messages = message_store.load(session_id).await.unwrap();
    assert_eq!(messages.len(), 4); // 3 original + 1 new assistant response
}

#[tokio::test]
async fn test_reason_atom_with_tool_result_continuation() {
    let (agent_store, session_store, message_store, provider_store, agent_id, session_id) =
        setup_test_environment().await;

    // Simulate a conversation where tool was called and result is available
    let tool_call = ToolCall {
        id: "call_123".to_string(),
        name: "get_weather".to_string(),
        arguments: json!({"city": "Tokyo"}),
    };

    message_store
        .seed(
            session_id,
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
        create_custom_driver_registry(LlmSimConfig::fixed("It's 22°C and sunny in Tokyo!"));

    let atom = ReasonAtom::new(
        agent_store,
        session_store,
        message_store.clone(),
        provider_store,
        CapabilityRegistry::new(),
        driver_registry,
        NoopEventEmitter,
    );

    let context = create_context(session_id);
    let input = ReasonInput { context, agent_id };

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
    let (agent_store, session_store, message_store, provider_store, agent_id, session_id) =
        setup_test_environment().await;

    message_store
        .seed(session_id, vec![Message::user("Tell me a long story")])
        .await;

    // Use lorem ipsum generator
    let driver_registry = create_custom_driver_registry(LlmSimConfig::lorem(100));

    let atom = ReasonAtom::new(
        agent_store,
        session_store,
        message_store.clone(),
        provider_store,
        CapabilityRegistry::new(),
        driver_registry,
        NoopEventEmitter,
    );

    let context = create_context(session_id);
    let input = ReasonInput { context, agent_id };

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
    };

    let response = driver
        .chat_completion(messages, &call_config)
        .await
        .expect("Chat completion should succeed");

    // Default driver returns a fixed response
    assert!(!response.text.is_empty());
}
