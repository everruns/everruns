// Integration tests for Everruns API (M2)
// Run with: cargo test --test integration_test

use everruns_contracts::{Event, Harness, LlmModel, LlmProvider, Session};
use serde_json::json;

const API_BASE_URL: &str = "http://localhost:9000";

#[tokio::test]
#[ignore] // Run with: cargo test --test integration_test -- --ignored
async fn test_full_harness_session_workflow() {
    let client = reqwest::Client::new();

    println!("Testing full harness/session workflow...");

    // Step 1: Create a harness
    println!("\nStep 1: Creating harness...");
    let create_harness_response = client
        .post(format!("{}/v1/harnesses", API_BASE_URL))
        .json(&json!({
            "slug": "test-harness",
            "display_name": "Test Harness",
            "description": "A harness for testing",
            "system_prompt": "You are a helpful assistant"
        }))
        .send()
        .await
        .expect("Failed to create harness");

    assert_eq!(
        create_harness_response.status(),
        201,
        "Expected 201 Created, got {}",
        create_harness_response.status()
    );

    let harness: Harness = create_harness_response
        .json()
        .await
        .expect("Failed to parse harness response");

    println!("Created harness: {}", harness.id);
    assert_eq!(harness.display_name, "Test Harness");
    assert_eq!(harness.status.to_string(), "active");

    // Step 2: List harnesses
    println!("\nStep 2: Listing harnesses...");
    let list_response = client
        .get(format!("{}/v1/harnesses", API_BASE_URL))
        .send()
        .await
        .expect("Failed to list harnesses");

    assert_eq!(list_response.status(), 200);

    let response: serde_json::Value = list_response.json().await.expect("Failed to parse");
    let harnesses: Vec<Harness> =
        serde_json::from_value(response["data"].clone()).expect("Failed to parse harnesses");
    println!("Found {} harness(es)", harnesses.len());
    assert!(!harnesses.is_empty());

    // Step 3: Get harness by ID
    println!("\nStep 3: Getting harness by ID...");
    let get_response = client
        .get(format!("{}/v1/harnesses/{}", API_BASE_URL, harness.id))
        .send()
        .await
        .expect("Failed to get harness");

    assert_eq!(get_response.status(), 200);
    let fetched_harness: Harness = get_response.json().await.expect("Failed to parse harness");
    println!("Fetched harness: {}", fetched_harness.display_name);
    assert_eq!(fetched_harness.id, harness.id);

    // Step 4: Update harness
    println!("\nStep 4: Updating harness...");
    let update_response = client
        .patch(format!("{}/v1/harnesses/{}", API_BASE_URL, harness.id))
        .json(&json!({
            "display_name": "Updated Test Harness",
            "description": "Updated description"
        }))
        .send()
        .await
        .expect("Failed to update harness");

    assert_eq!(update_response.status(), 200);
    let updated_harness: Harness = update_response
        .json()
        .await
        .expect("Failed to parse harness");
    println!("Updated harness: {}", updated_harness.display_name);
    assert_eq!(updated_harness.display_name, "Updated Test Harness");

    // Step 5: Create a session
    println!("\nStep 5: Creating session...");
    let session_response = client
        .post(format!(
            "{}/v1/harnesses/{}/sessions",
            API_BASE_URL, harness.id
        ))
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
    assert_eq!(session.harness_id, harness.id);

    // Step 6: Add event (user message)
    println!("\nStep 6: Adding user message event...");
    let event_response = client
        .post(format!(
            "{}/v1/harnesses/{}/sessions/{}/events",
            API_BASE_URL, harness.id, session.id
        ))
        .json(&json!({
            "event_type": "message.user",
            "data": {
                "message": {
                    "role": "user",
                    "content": [{"type": "text", "text": "Hello!"}]
                }
            }
        }))
        .send()
        .await
        .expect("Failed to create event");

    assert_eq!(event_response.status(), 201);
    let event: Event = event_response.json().await.expect("Failed to parse event");
    println!("Created event: {}", event.id);
    assert_eq!(event.event_type, "message.user");

    // Step 7: List messages
    println!("\nStep 7: Listing messages...");
    let messages_response = client
        .get(format!(
            "{}/v1/harnesses/{}/sessions/{}/messages",
            API_BASE_URL, harness.id, session.id
        ))
        .send()
        .await
        .expect("Failed to list messages");

    assert_eq!(messages_response.status(), 200);
    let response: serde_json::Value = messages_response.json().await.expect("Failed to parse");
    let messages: Vec<Event> =
        serde_json::from_value(response["data"].clone()).expect("Failed to parse messages");
    println!("Found {} message(s)", messages.len());
    assert_eq!(messages.len(), 1);

    // Step 8: Get session
    println!("\nStep 8: Getting session...");
    let get_session_response = client
        .get(format!(
            "{}/v1/harnesses/{}/sessions/{}",
            API_BASE_URL, harness.id, session.id
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
            "model_id": "gpt-4o",
            "display_name": "GPT-4o",
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
    assert_eq!(model.model_id, "gpt-4o");

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
