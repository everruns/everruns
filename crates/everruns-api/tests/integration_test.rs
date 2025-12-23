// Integration tests for Everruns API (M2)
// Run with: cargo test --test integration_test

use everruns_contracts::{Agent, Event, LlmModel, LlmProvider, Message, Session};
use serde_json::json;
use uuid::Uuid;

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
    let message: Message = message_response
        .json()
        .await
        .expect("Failed to parse message");
    println!("Created message: {}", message.id);
    assert_eq!(message.role.to_string(), "user");

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
    let response: serde_json::Value = messages_response.json().await.expect("Failed to parse");
    let messages: Vec<Message> =
        serde_json::from_value(response["data"].clone()).expect("Failed to parse messages");
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
async fn test_idempotent_agent_creation() {
    let client = reqwest::Client::new();

    println!("Testing idempotent agent creation (PUT /v1/agents)...");

    let unique_name = format!("Idempotent Test Agent {}", Uuid::new_v4());

    // Step 1: Create agent using PUT (should return 201)
    println!("\nStep 1: First PUT request (should create)...");
    let first_response = client
        .put(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": unique_name,
            "system_prompt": "You are a test assistant.",
            "tags": ["test", "idempotent"]
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(
        first_response.status(),
        201,
        "Expected 201 Created for first PUT, got {}",
        first_response.status()
    );

    let first_agent: Agent = first_response
        .json()
        .await
        .expect("Failed to parse first agent response");

    println!(
        "Created agent: {} (id: {})",
        first_agent.name, first_agent.id
    );
    assert_eq!(first_agent.name, unique_name);

    // Step 2: Second PUT with same name (should return 200 with same agent)
    println!("\nStep 2: Second PUT request (should return existing)...");
    let second_response = client
        .put(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": unique_name,
            "system_prompt": "Different prompt - should be ignored.",
            "tags": ["different", "tags"]
        }))
        .send()
        .await
        .expect("Failed to get existing agent");

    assert_eq!(
        second_response.status(),
        200,
        "Expected 200 OK for second PUT (existing agent), got {}",
        second_response.status()
    );

    let second_agent: Agent = second_response
        .json()
        .await
        .expect("Failed to parse second agent response");

    println!(
        "Returned agent: {} (id: {})",
        second_agent.name, second_agent.id
    );
    assert_eq!(
        second_agent.id, first_agent.id,
        "Expected same agent ID for idempotent PUT"
    );

    // Step 3: Cleanup - delete the test agent
    println!("\nStep 3: Cleaning up...");
    let delete_response = client
        .delete(format!("{}/v1/agents/{}", API_BASE_URL, first_agent.id))
        .send()
        .await
        .expect("Failed to delete agent");

    assert_eq!(
        delete_response.status(),
        204,
        "Expected 204 No Content for delete"
    );

    println!("\nIdempotent agent creation test passed!");
}
