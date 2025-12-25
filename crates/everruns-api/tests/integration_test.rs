// Integration tests for Everruns API (M2)
// Run with: cargo test --test integration_test

use everruns_core::llm_entities::LlmProvider;
use everruns_core::{Agent, Event, LlmModel, Session};
use serde_json::{json, Value};

const API_BASE_URL: &str = "http://localhost:9000";

#[tokio::test]
#[ignore] // Run with: cargo test --test integration_test -- --ignored
async fn test_full_agent_session_workflow() {
    let client = reqwest::Client::new();

    println!("Testing full agent/session workflow...");

    // Step 1: Create an agent
    println!("\nStep 1: Creating agent...");
    let create_agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "Test Agent",
            "description": "An agent for testing",
            "system_prompt": "You are a helpful assistant"
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(
        create_agent_response.status(),
        201,
        "Expected 201 Created, got {}",
        create_agent_response.status()
    );

    let agent: Agent = create_agent_response
        .json()
        .await
        .expect("Failed to parse agent response");

    println!("Created agent: {}", agent.id);
    assert_eq!(agent.name, "Test Agent");
    assert_eq!(agent.status.to_string(), "active");

    // Step 2: List agents
    println!("\nStep 2: Listing agents...");
    let list_response = client
        .get(format!("{}/v1/agents", API_BASE_URL))
        .send()
        .await
        .expect("Failed to list agents");

    assert_eq!(list_response.status(), 200);

    let response: serde_json::Value = list_response.json().await.expect("Failed to parse");
    let agents: Vec<Agent> =
        serde_json::from_value(response["data"].clone()).expect("Failed to parse agents");
    println!("Found {} agent(s)", agents.len());
    assert!(!agents.is_empty());

    // Step 3: Get agent by ID
    println!("\nStep 3: Getting agent by ID...");
    let get_response = client
        .get(format!("{}/v1/agents/{}", API_BASE_URL, agent.id))
        .send()
        .await
        .expect("Failed to get agent");

    assert_eq!(get_response.status(), 200);
    let fetched_agent: Agent = get_response.json().await.expect("Failed to parse agent");
    println!("Fetched agent: {}", fetched_agent.name);
    assert_eq!(fetched_agent.id, agent.id);

    // Step 4: Update agent
    println!("\nStep 4: Updating agent...");
    let update_response = client
        .patch(format!("{}/v1/agents/{}", API_BASE_URL, agent.id))
        .json(&json!({
            "name": "Updated Test Agent",
            "description": "Updated description"
        }))
        .send()
        .await
        .expect("Failed to update agent");

    assert_eq!(update_response.status(), 200);
    let updated_agent: Agent = update_response.json().await.expect("Failed to parse agent");
    println!("Updated agent: {}", updated_agent.name);
    assert_eq!(updated_agent.name, "Updated Test Agent");

    // Step 5: Create a session
    println!("\nStep 5: Creating session...");
    let session_response = client
        .post(format!("{}/v1/agents/{}/sessions", API_BASE_URL, agent.id))
        .json(&json!({
            "title": "Test Session"
        }))
        .send()
        .await
        .expect("Failed to create session");

    assert_eq!(session_response.status(), 201);
    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");
    println!("Created session: {}", session.id);
    assert_eq!(session.agent_id, agent.id);

    // Step 6: Add message (user message)
    println!("\nStep 6: Adding user message...");
    let message_response = client
        .post(format!(
            "{}/v1/agents/{}/sessions/{}/messages",
            API_BASE_URL, agent.id, session.id
        ))
        .json(&json!({
            "role": "user",
            "content": {"text": "Hello!"}
        }))
        .send()
        .await
        .expect("Failed to create message");

    assert_eq!(message_response.status(), 201);
    let message: Value = message_response
        .json()
        .await
        .expect("Failed to parse message");
    println!("Created message: {}", message["id"]);
    assert_eq!(message["role"], "user");

    // Step 7: List messages
    println!("\nStep 7: Listing messages...");
    let messages_response = client
        .get(format!(
            "{}/v1/agents/{}/sessions/{}/messages",
            API_BASE_URL, agent.id, session.id
        ))
        .send()
        .await
        .expect("Failed to list messages");

    assert_eq!(messages_response.status(), 200);
    let response: Value = messages_response.json().await.expect("Failed to parse");
    let messages = response["data"]
        .as_array()
        .expect("Expected array of messages");
    println!("Found {} message(s)", messages.len());
    assert_eq!(messages.len(), 1);

    // Step 8: Get session
    println!("\nStep 8: Getting session...");
    let get_session_response = client
        .get(format!(
            "{}/v1/agents/{}/sessions/{}",
            API_BASE_URL, agent.id, session.id
        ))
        .send()
        .await
        .expect("Failed to get session");

    assert_eq!(get_session_response.status(), 200);
    let fetched_session: Session = get_session_response
        .json()
        .await
        .expect("Failed to parse session");
    println!("Fetched session: {}", fetched_session.id);
    assert_eq!(fetched_session.id, session.id);

    // Step 9: Create event (for SSE notifications)
    println!("\nStep 9: Creating event...");
    let event_response = client
        .post(format!(
            "{}/v1/agents/{}/sessions/{}/events",
            API_BASE_URL, agent.id, session.id
        ))
        .json(&json!({
            "event_type": "status.update",
            "data": {"status": "processing"}
        }))
        .send()
        .await
        .expect("Failed to create event");

    assert_eq!(event_response.status(), 201);
    let event: Event = event_response.json().await.expect("Failed to parse event");
    println!("Created event: {}", event.id);
    assert_eq!(event.event_type, "status.update");

    println!("\nAll tests passed!");
}

#[tokio::test]
#[ignore]
async fn test_health_endpoint() {
    let client = reqwest::Client::new();

    println!("Testing health endpoint...");
    let response = client
        .get(format!("{}/health", API_BASE_URL))
        .send()
        .await
        .expect("Failed to call health endpoint");

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.expect("Failed to parse response");
    println!("Health check: {:?}", body);
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
#[ignore]
async fn test_openapi_spec() {
    let client = reqwest::Client::new();

    println!("Testing OpenAPI spec endpoint...");
    let response = client
        .get(format!("{}/api-doc/openapi.json", API_BASE_URL))
        .send()
        .await
        .expect("Failed to get OpenAPI spec");

    assert_eq!(response.status(), 200);
    let spec: serde_json::Value = response.json().await.expect("Failed to parse spec");
    println!("OpenAPI spec title: {}", spec["info"]["title"]);
    assert_eq!(spec["info"]["title"], "Everruns API");
}

#[tokio::test]
#[ignore]
async fn test_llm_provider_and_model_workflow() {
    let client = reqwest::Client::new();

    println!("Testing LLM Provider and Model workflow...");

    // Step 1: Create an LLM provider
    println!("\nStep 1: Creating LLM provider...");
    let create_provider_response = client
        .post(format!("{}/v1/llm-providers", API_BASE_URL))
        .json(&json!({
            "name": "Test OpenAI Provider",
            "provider_type": "openai",
            "base_url": "https://api.openai.com/v1",
            "is_default": true
        }))
        .send()
        .await
        .expect("Failed to create LLM provider");

    let response_text = create_provider_response
        .text()
        .await
        .expect("Failed to get response text");

    let provider: LlmProvider =
        serde_json::from_str(&response_text).expect("Failed to parse provider response");

    println!("Created provider: {} ({})", provider.name, provider.id);
    assert_eq!(provider.name, "Test OpenAI Provider");

    // Step 2: Create a model for the provider
    println!("\nStep 2: Creating model for provider...");
    let create_model_response = client
        .post(format!(
            "{}/v1/llm-providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": "gpt-5.2",
            "display_name": "GPT-5.2",
            "capabilities": ["chat", "vision"],
            "context_window": 128000,
            "is_default": true
        }))
        .send()
        .await
        .expect("Failed to create model");

    let model_response_text = create_model_response
        .text()
        .await
        .expect("Failed to get model response text");

    let model: LlmModel =
        serde_json::from_str(&model_response_text).expect("Failed to parse model response");

    println!("Created model: {} ({})", model.display_name, model.id);
    assert_eq!(model.model_id, "gpt-5.2");

    // Cleanup
    println!("\nCleaning up...");

    client
        .delete(format!("{}/v1/llm-models/{}", API_BASE_URL, model.id))
        .send()
        .await
        .expect("Failed to delete model");

    client
        .delete(format!("{}/v1/llm-providers/{}", API_BASE_URL, provider.id))
        .send()
        .await
        .expect("Failed to delete provider");

    println!("All LLM provider and model tests passed!");
}

#[tokio::test]
#[ignore]
async fn test_llm_model_profile() {
    let client = reqwest::Client::new();

    println!("Testing LLM Model Profile...");

    // Step 1: Create an LLM provider
    println!("\nStep 1: Creating OpenAI provider...");
    let create_provider_response = client
        .post(format!("{}/v1/llm-providers", API_BASE_URL))
        .json(&json!({
            "name": "Test Profile Provider",
            "provider_type": "openai",
            "is_default": false
        }))
        .send()
        .await
        .expect("Failed to create LLM provider");

    let provider: LlmProvider = create_provider_response
        .json()
        .await
        .expect("Failed to parse provider response");

    println!("Created provider: {} ({})", provider.name, provider.id);

    // Step 2: Create a known model (gpt-4o) that has a profile
    println!("\nStep 2: Creating gpt-4o model...");
    let create_model_response = client
        .post(format!(
            "{}/v1/llm-providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": "gpt-4o",
            "display_name": "GPT-4o",
            "capabilities": ["chat", "vision"],
            "is_default": false
        }))
        .send()
        .await
        .expect("Failed to create model");

    let model_json: Value = create_model_response
        .json()
        .await
        .expect("Failed to parse model response");

    println!("Created model: {}", model_json["display_name"]);

    // Step 3: Get the model and verify profile is included
    println!("\nStep 3: Getting model with profile...");
    let get_model_response = client
        .get(format!(
            "{}/v1/llm-models/{}",
            API_BASE_URL,
            model_json["id"].as_str().unwrap()
        ))
        .send()
        .await
        .expect("Failed to get model");

    let model_with_profile: Value = get_model_response
        .json()
        .await
        .expect("Failed to parse model response");

    // Verify profile exists and has expected fields
    let profile = &model_with_profile["profile"];
    assert!(!profile.is_null(), "Profile should not be null for gpt-4o");
    assert_eq!(profile["name"], "GPT-4o", "Profile name should be GPT-4o");
    assert_eq!(
        profile["family"], "gpt-4o",
        "Profile family should be gpt-4o"
    );
    assert!(
        profile["tool_call"].as_bool().unwrap_or(false),
        "GPT-4o should support tool calls"
    );
    assert!(
        profile["cost"].is_object(),
        "Profile should have cost information"
    );
    assert!(
        profile["limits"].is_object(),
        "Profile should have limits information"
    );

    println!("Profile verified: {:?}", profile);

    // Step 4: List all models and verify profile data
    println!("\nStep 4: Listing all models...");
    let list_models_response = client
        .get(format!("{}/v1/llm-models", API_BASE_URL))
        .send()
        .await
        .expect("Failed to list models");

    let models_response: Value = list_models_response
        .json()
        .await
        .expect("Failed to parse models response");

    let models = models_response["data"]
        .as_array()
        .expect("Expected array of models");
    let gpt4o_model = models.iter().find(|m| m["model_id"] == "gpt-4o");
    assert!(gpt4o_model.is_some(), "Should find gpt-4o in model list");
    assert!(
        !gpt4o_model.unwrap()["profile"].is_null(),
        "Profile should be included in list response"
    );

    // Cleanup
    println!("\nCleaning up...");
    client
        .delete(format!(
            "{}/v1/llm-models/{}",
            API_BASE_URL,
            model_json["id"].as_str().unwrap()
        ))
        .send()
        .await
        .expect("Failed to delete model");

    client
        .delete(format!("{}/v1/llm-providers/{}", API_BASE_URL, provider.id))
        .send()
        .await
        .expect("Failed to delete provider");

    println!("LLM Model Profile tests passed!");
}
