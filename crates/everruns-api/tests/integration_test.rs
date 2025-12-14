// Integration tests for Everruns API
// Run with: cargo test --test integration_test

use everruns_contracts::{Agent, AgentStatus, Message, Run, Thread};
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
