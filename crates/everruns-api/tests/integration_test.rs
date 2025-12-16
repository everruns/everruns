// Integration tests for Everruns API
// Run with: cargo test --test integration_test

use everruns_contracts::{Agent, AgentStatus, LlmModel, LlmProvider, Message, Run, Thread};
use serde_json::json;

const API_BASE_URL: &str = "http://localhost:9000";

#[tokio::test]
#[ignore] // Run with: cargo test --test integration_test -- --ignored
async fn test_full_agent_workflow() {
    let client = reqwest::Client::new();

    println!("🧪 Testing full agent workflow...");

    // Step 1: Create an agent
    println!("\n📝 Step 1: Creating agent...");
    let create_agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "Test Assistant",
            "description": "An AI assistant for testing",
            "default_model_id": "gpt-5.1",
            "definition": {
                "system_prompt": "You are a helpful assistant",
                "temperature": 0.7
            }
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

    println!("✅ Created agent: {}", agent.id);
    assert_eq!(agent.name, "Test Assistant");
    assert_eq!(agent.status, AgentStatus::Active);

    // Step 2: List agents
    println!("\n📋 Step 2: Listing agents...");
    let list_response = client
        .get(format!("{}/v1/agents", API_BASE_URL))
        .send()
        .await
        .expect("Failed to list agents");

    assert_eq!(list_response.status(), 200);

    let agents: Vec<Agent> = list_response.json().await.expect("Failed to parse agents");
    println!("✅ Found {} agent(s)", agents.len());
    assert!(!agents.is_empty());

    // Step 3: Get agent by ID
    println!("\n🔍 Step 3: Getting agent by ID...");
    let get_response = client
        .get(format!("{}/v1/agents/{}", API_BASE_URL, agent.id))
        .send()
        .await
        .expect("Failed to get agent");

    assert_eq!(get_response.status(), 200);
    let fetched_agent: Agent = get_response.json().await.expect("Failed to parse agent");
    println!("✅ Fetched agent: {}", fetched_agent.name);
    assert_eq!(fetched_agent.id, agent.id);

    // Step 4: Update agent
    println!("\n✏️  Step 4: Updating agent...");
    let update_response = client
        .patch(format!("{}/v1/agents/{}", API_BASE_URL, agent.id))
        .json(&json!({
            "name": "Updated Test Assistant",
            "description": "Updated description"
        }))
        .send()
        .await
        .expect("Failed to update agent");

    assert_eq!(update_response.status(), 200);
    let updated_agent: Agent = update_response.json().await.expect("Failed to parse agent");
    println!("✅ Updated agent: {}", updated_agent.name);
    assert_eq!(updated_agent.name, "Updated Test Assistant");
    assert_eq!(
        updated_agent.description,
        Some("Updated description".to_string())
    );

    // Step 5: Create a thread
    println!("\n🧵 Step 5: Creating thread...");
    let thread_response = client
        .post(format!("{}/v1/threads", API_BASE_URL))
        .json(&json!({}))
        .send()
        .await
        .expect("Failed to create thread");

    assert_eq!(thread_response.status(), 201);
    let thread: Thread = thread_response
        .json()
        .await
        .expect("Failed to parse thread");
    println!("✅ Created thread: {}", thread.id);

    // Step 6: Add messages to thread
    println!("\n💬 Step 6: Adding messages to thread...");
    let message_response = client
        .post(format!(
            "{}/v1/threads/{}/messages",
            API_BASE_URL, thread.id
        ))
        .json(&json!({
            "role": "user",
            "content": "Hello, can you help me?"
        }))
        .send()
        .await
        .expect("Failed to create message");

    assert_eq!(message_response.status(), 201);
    let message: Message = message_response
        .json()
        .await
        .expect("Failed to parse message");
    println!("✅ Created message: {}", message.id);
    assert_eq!(message.role, "user");

    // Step 7: List messages
    println!("\n📨 Step 7: Listing messages...");
    let messages_response = client
        .get(format!(
            "{}/v1/threads/{}/messages",
            API_BASE_URL, thread.id
        ))
        .send()
        .await
        .expect("Failed to list messages");

    assert_eq!(messages_response.status(), 200);
    let messages: Vec<Message> = messages_response
        .json()
        .await
        .expect("Failed to parse messages");
    println!("✅ Found {} message(s)", messages.len());
    assert_eq!(messages.len(), 1);

    // Step 8: Create a run
    println!("\n🏃 Step 8: Creating run...");
    let run_response = client
        .post(format!("{}/v1/runs", API_BASE_URL))
        .json(&json!({
            "agent_id": agent.id,
            "thread_id": thread.id
        }))
        .send()
        .await
        .expect("Failed to create run");

    assert_eq!(run_response.status(), 201);
    let run: Run = run_response.json().await.expect("Failed to parse run");
    println!("✅ Created run: {}", run.id);
    assert_eq!(run.agent_id, agent.id);
    assert_eq!(run.thread_id, thread.id);

    // Step 9: Get run by ID
    println!("\n🔎 Step 9: Getting run by ID...");
    let get_run_response = client
        .get(format!("{}/v1/runs/{}", API_BASE_URL, run.id))
        .send()
        .await
        .expect("Failed to get run");

    assert_eq!(get_run_response.status(), 200);
    let fetched_run: Run = get_run_response.json().await.expect("Failed to parse run");
    println!("✅ Fetched run: {}", fetched_run.id);
    assert_eq!(fetched_run.id, run.id);

    println!("\n🎉 All tests passed!");
}

#[tokio::test]
#[ignore]
async fn test_health_endpoint() {
    let client = reqwest::Client::new();

    println!("🏥 Testing health endpoint...");
    let response = client
        .get(format!("{}/health", API_BASE_URL))
        .send()
        .await
        .expect("Failed to call health endpoint");

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.expect("Failed to parse response");
    println!("✅ Health check: {:?}", body);
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
#[ignore]
async fn test_openapi_spec() {
    let client = reqwest::Client::new();

    println!("📖 Testing OpenAPI spec endpoint...");
    let response = client
        .get(format!("{}/api-doc/openapi.json", API_BASE_URL))
        .send()
        .await
        .expect("Failed to get OpenAPI spec");

    assert_eq!(response.status(), 200);
    let spec: serde_json::Value = response.json().await.expect("Failed to parse spec");
    println!("✅ OpenAPI spec title: {}", spec["info"]["title"]);
    assert_eq!(spec["info"]["title"], "Everruns API");
}

#[tokio::test]
#[ignore]
async fn test_llm_provider_and_model_workflow() {
    let client = reqwest::Client::new();

    println!("🧪 Testing LLM Provider and Model workflow...");

    // Step 1: Create an LLM provider
    println!("\n📝 Step 1: Creating LLM provider...");
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

    println!(
        "Create provider response status: {}",
        create_provider_response.status()
    );
    let response_text = create_provider_response
        .text()
        .await
        .expect("Failed to get response text");
    println!("Create provider response body: {}", response_text);

    let provider: LlmProvider =
        serde_json::from_str(&response_text).expect("Failed to parse provider response");

    println!("✅ Created provider: {} ({})", provider.name, provider.id);
    assert_eq!(provider.name, "Test OpenAI Provider");

    // Step 2: List providers
    println!("\n📋 Step 2: Listing providers...");
    let list_response = client
        .get(format!("{}/v1/llm-providers", API_BASE_URL))
        .send()
        .await
        .expect("Failed to list providers");

    assert_eq!(list_response.status(), 200);
    let providers: Vec<LlmProvider> = list_response
        .json()
        .await
        .expect("Failed to parse providers");
    println!("✅ Found {} provider(s)", providers.len());
    assert!(!providers.is_empty());

    // Step 3: Get provider by ID
    println!("\n🔍 Step 3: Getting provider by ID...");
    let get_response = client
        .get(format!("{}/v1/llm-providers/{}", API_BASE_URL, provider.id))
        .send()
        .await
        .expect("Failed to get provider");

    assert_eq!(get_response.status(), 200);
    let fetched_provider: LlmProvider =
        get_response.json().await.expect("Failed to parse provider");
    println!("✅ Fetched provider: {}", fetched_provider.name);
    assert_eq!(fetched_provider.id, provider.id);

    // Step 4: Create a model for the provider
    println!("\n🤖 Step 4: Creating model for provider...");
    let model_url = format!("{}/v1/llm-providers/{}/models", API_BASE_URL, provider.id);
    println!("POST {}", model_url);

    let create_model_response = client
        .post(&model_url)
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

    let model_status = create_model_response.status();
    println!("Create model response status: {}", model_status);

    let model_response_text = create_model_response
        .text()
        .await
        .expect("Failed to get model response text");
    println!("Create model response body: {}", model_response_text);

    assert_eq!(
        model_status, 201,
        "Expected 201 Created for model creation, got {}. Response: {}",
        model_status, model_response_text
    );

    let model: LlmModel =
        serde_json::from_str(&model_response_text).expect("Failed to parse model response");

    println!("✅ Created model: {} ({})", model.display_name, model.id);
    assert_eq!(model.model_id, "gpt-4o");
    assert_eq!(model.display_name, "GPT-4o");

    // Step 5: List models for provider
    println!("\n📋 Step 5: Listing models for provider...");
    let list_models_response = client
        .get(format!(
            "{}/v1/llm-providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .send()
        .await
        .expect("Failed to list models for provider");

    assert_eq!(list_models_response.status(), 200);
    let provider_models: Vec<LlmModel> = list_models_response
        .json()
        .await
        .expect("Failed to parse provider models");
    println!("✅ Found {} model(s) for provider", provider_models.len());
    assert!(!provider_models.is_empty());

    // Step 6: List all models
    println!("\n📋 Step 6: Listing all models...");
    let all_models_response = client
        .get(format!("{}/v1/llm-models", API_BASE_URL))
        .send()
        .await
        .expect("Failed to list all models");

    assert_eq!(all_models_response.status(), 200);

    // Step 7: Get model by ID
    println!("\n🔍 Step 7: Getting model by ID...");
    let get_model_response = client
        .get(format!("{}/v1/llm-models/{}", API_BASE_URL, model.id))
        .send()
        .await
        .expect("Failed to get model");

    assert_eq!(get_model_response.status(), 200);
    let fetched_model: LlmModel = get_model_response
        .json()
        .await
        .expect("Failed to parse model");
    println!("✅ Fetched model: {}", fetched_model.display_name);
    assert_eq!(fetched_model.id, model.id);

    // Step 8: Update model
    println!("\n✏️  Step 8: Updating model...");
    let update_model_response = client
        .patch(format!("{}/v1/llm-models/{}", API_BASE_URL, model.id))
        .json(&json!({
            "display_name": "GPT-4o (Updated)"
        }))
        .send()
        .await
        .expect("Failed to update model");

    assert_eq!(update_model_response.status(), 200);
    let updated_model: LlmModel = update_model_response
        .json()
        .await
        .expect("Failed to parse updated model");
    println!("✅ Updated model: {}", updated_model.display_name);
    assert_eq!(updated_model.display_name, "GPT-4o (Updated)");

    // Step 9: Delete model
    println!("\n🗑️  Step 9: Deleting model...");
    let delete_model_response = client
        .delete(format!("{}/v1/llm-models/{}", API_BASE_URL, model.id))
        .send()
        .await
        .expect("Failed to delete model");

    assert_eq!(delete_model_response.status(), 204);
    println!("✅ Deleted model");

    // Step 10: Delete provider
    println!("\n🗑️  Step 10: Deleting provider...");
    let delete_provider_response = client
        .delete(format!("{}/v1/llm-providers/{}", API_BASE_URL, provider.id))
        .send()
        .await
        .expect("Failed to delete provider");

    assert_eq!(delete_provider_response.status(), 204);
    println!("✅ Deleted provider");

    println!("\n🎉 All LLM provider and model tests passed!");
}
