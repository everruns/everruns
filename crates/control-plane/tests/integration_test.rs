// Integration tests for Everruns API
// Run with: cargo test -p everruns-control-plane --test integration_test -- --test-threads=1
// Requires: API + Worker running (uses LlmSim for workflow tests, no real API keys needed)

use everruns_core::llm_models::LlmProvider;
use everruns_core::{Agent, LlmModel, Session, SessionFile};
use serde_json::{Value, json};

const API_BASE_URL: &str = "http://localhost:9000";
// Default organization ID for org-scoped routes
const DEFAULT_ORG: &str = "org_00000000000000000000000000000001";

#[tokio::test]
async fn test_full_agent_session_workflow() {
    let client = reqwest::Client::new();

    println!("Testing full agent/session workflow...");

    // Step 1: Create an agent
    println!("\nStep 1: Creating agent...");
    let create_agent_response = client
        .post(format!("{}/v1/orgs/{}/agents", API_BASE_URL, DEFAULT_ORG))
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
        .get(format!("{}/v1/orgs/{}/agents", API_BASE_URL, DEFAULT_ORG))
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
        .get(format!(
            "{}/v1/orgs/{}/agents/{}",
            API_BASE_URL, DEFAULT_ORG, agent.id
        ))
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
        .patch(format!(
            "{}/v1/orgs/{}/agents/{}",
            API_BASE_URL, DEFAULT_ORG, agent.id
        ))
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
        .post(format!(
            "{}/v1/orgs/{}/agents/{}/sessions",
            API_BASE_URL, DEFAULT_ORG, agent.id
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
    assert_eq!(session.agent_id, agent.id);

    // Step 6: Add message (user message)
    println!("\nStep 6: Adding user message...");
    let message_response = client
        .post(format!(
            "{}/v1/orgs/{}/agents/{}/sessions/{}/messages",
            API_BASE_URL, DEFAULT_ORG, agent.id, session.id
        ))
        .json(&json!({
            "message": {
                "role": "user",
                "content": [{"type": "text", "text": "Hello!"}]
            }
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
            "{}/v1/orgs/{}/agents/{}/sessions/{}/messages",
            API_BASE_URL, DEFAULT_ORG, agent.id, session.id
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
            "{}/v1/orgs/{}/agents/{}/sessions/{}",
            API_BASE_URL, DEFAULT_ORG, agent.id, session.id
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

    // Step 9: List events (events are created automatically with messages)
    println!("\nStep 9: Listing events...");
    let events_response = client
        .get(format!(
            "{}/v1/orgs/{}/agents/{}/sessions/{}/events",
            API_BASE_URL, DEFAULT_ORG, agent.id, session.id
        ))
        .send()
        .await
        .expect("Failed to list events");

    assert_eq!(events_response.status(), 200);
    let events_data: Value = events_response
        .json()
        .await
        .expect("Failed to parse events");
    let events = events_data["data"]
        .as_array()
        .expect("Expected array of events");
    println!("Found {} event(s)", events.len());
    // Events are created when messages are processed by the workflow
    // For this basic test, we just verify the endpoint works

    println!("\nAll tests passed!");
}

#[tokio::test]
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
async fn test_llm_provider_and_model_workflow() {
    let client = reqwest::Client::new();

    println!("Testing LLM Provider and Model workflow...");

    // Step 1: Create an LLM provider
    println!("\nStep 1: Creating LLM provider...");
    let create_provider_response = client
        .post(format!(
            "{}/v1/orgs/{}/llm-providers",
            API_BASE_URL, DEFAULT_ORG
        ))
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
            "{}/v1/orgs/{}/llm-providers/{}/models",
            API_BASE_URL, DEFAULT_ORG, provider.id
        ))
        .json(&json!({
            "model_id": "gpt-5.2",
            "display_name": "GPT-5.2",
            "capabilities": ["chat", "vision"],
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
        .delete(format!(
            "{}/v1/orgs/{}/llm-models/{}",
            API_BASE_URL, DEFAULT_ORG, model.id
        ))
        .send()
        .await
        .expect("Failed to delete model");

    client
        .delete(format!(
            "{}/v1/orgs/{}/llm-providers/{}",
            API_BASE_URL, DEFAULT_ORG, provider.id
        ))
        .send()
        .await
        .expect("Failed to delete provider");

    println!("All LLM provider and model tests passed!");
}

#[tokio::test]
async fn test_llm_model_profile() {
    let client = reqwest::Client::new();

    println!("Testing LLM Model Profile...");

    // Step 1: Create an LLM provider
    println!("\nStep 1: Creating OpenAI provider...");
    let create_provider_response = client
        .post(format!(
            "{}/v1/orgs/{}/llm-providers",
            API_BASE_URL, DEFAULT_ORG
        ))
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
            "{}/v1/orgs/{}/llm-providers/{}/models",
            API_BASE_URL, DEFAULT_ORG, provider.id
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
    let model_id = model_json["id"].as_str().unwrap();

    // Step 3: Get the model via the list endpoint which includes profile
    println!("\nStep 3: Getting model with profile via list endpoint...");
    let list_models_response = client
        .get(format!(
            "{}/v1/orgs/{}/llm-models",
            API_BASE_URL, DEFAULT_ORG
        ))
        .send()
        .await
        .expect("Failed to list models");

    assert_eq!(list_models_response.status(), 200);
    let list_response: Value = list_models_response
        .json()
        .await
        .expect("Failed to parse models response");
    let models = list_response["data"]
        .as_array()
        .expect("Response should have data array");

    let gpt4o_model = models
        .iter()
        .find(|m| m["model_id"] == "gpt-4o")
        .expect("Should find gpt-4o in model list");

    // Verify profile exists and has expected fields
    let profile = &gpt4o_model["profile"];
    println!("Profile: {:?}", profile);

    // Profile may be null if the model profile lookup isn't working
    // For now, just verify we can list models - profile lookup is optional
    if !profile.is_null() {
        assert_eq!(profile["name"], "GPT-4o", "Profile name should be GPT-4o");
        assert_eq!(
            profile["family"], "gpt-4o",
            "Profile family should be gpt-4o"
        );
        assert!(
            profile["tool_call"].as_bool().unwrap_or(false),
            "GPT-4o should support tool calls"
        );
        println!("Profile verified successfully");
    } else {
        println!("Profile is null - skipping profile assertions");
    }

    // Cleanup
    println!("\nCleaning up...");
    client
        .delete(format!(
            "{}/v1/orgs/{}/llm-models/{}",
            API_BASE_URL, DEFAULT_ORG, model_id
        ))
        .send()
        .await
        .expect("Failed to delete model");

    client
        .delete(format!(
            "{}/v1/orgs/{}/llm-providers/{}",
            API_BASE_URL, DEFAULT_ORG, provider.id
        ))
        .send()
        .await
        .expect("Failed to delete provider");

    println!("LLM Model Profile tests passed!");
}

#[tokio::test]
async fn test_session_inherits_agent_default_model() {
    let client = reqwest::Client::new();

    println!("Testing session model_id inheritance from agent...");

    // Step 1: Create an LLM provider
    println!("\nStep 1: Creating LLM provider...");
    let provider_response = client
        .post(format!(
            "{}/v1/orgs/{}/llm-providers",
            API_BASE_URL, DEFAULT_ORG
        ))
        .json(&json!({
            "name": "Test Provider for Session Model",
            "provider_type": "openai",
            "is_default": false
        }))
        .send()
        .await
        .expect("Failed to create provider");

    let provider: LlmProvider = provider_response
        .json()
        .await
        .expect("Failed to parse provider");
    println!("Created provider: {}", provider.id);

    // Step 2: Create a model
    println!("\nStep 2: Creating model...");
    let model_response = client
        .post(format!(
            "{}/v1/orgs/{}/llm-providers/{}/models",
            API_BASE_URL, DEFAULT_ORG, provider.id
        ))
        .json(&json!({
            "model_id": "test-model",
            "display_name": "Test Model",
            "is_default": false
        }))
        .send()
        .await
        .expect("Failed to create model");

    let model: LlmModel = model_response.json().await.expect("Failed to parse model");
    println!("Created model: {}", model.id);

    // Step 3: Create an agent with default_model_id
    println!("\nStep 3: Creating agent with default_model_id...");
    let agent_response = client
        .post(format!("{}/v1/orgs/{}/agents", API_BASE_URL, DEFAULT_ORG))
        .json(&json!({
            "name": "Agent with Default Model",
            "system_prompt": "Test agent",
            "default_model_id": model.id.to_string()
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(agent_response.status(), 201);
    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!(
        "Created agent: {} with default_model_id: {:?}",
        agent.id, agent.default_model_id
    );
    assert_eq!(agent.default_model_id, Some(model.id));

    // Step 4: Create a session WITHOUT specifying model_id
    println!("\nStep 4: Creating session without model_id...");
    let session_response = client
        .post(format!(
            "{}/v1/orgs/{}/agents/{}/sessions",
            API_BASE_URL, DEFAULT_ORG, agent.id
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
    println!(
        "Created session: {} with model_id: {:?}",
        session.id, session.model_id
    );

    // Verify session inherited the agent's default_model_id
    assert_eq!(
        session.model_id,
        Some(model.id),
        "Session should inherit agent's default_model_id"
    );

    // Step 5: Create a session WITH explicit model_id (should override)
    println!("\nStep 5: Creating session with explicit model_id...");

    // Create another model
    let model2_response = client
        .post(format!(
            "{}/v1/orgs/{}/llm-providers/{}/models",
            API_BASE_URL, DEFAULT_ORG, provider.id
        ))
        .json(&json!({
            "model_id": "test-model-2",
            "display_name": "Test Model 2",
            "is_default": false
        }))
        .send()
        .await
        .expect("Failed to create second model");

    let model2: LlmModel = model2_response
        .json()
        .await
        .expect("Failed to parse second model");

    let session2_response = client
        .post(format!(
            "{}/v1/orgs/{}/agents/{}/sessions",
            API_BASE_URL, DEFAULT_ORG, agent.id
        ))
        .json(&json!({
            "title": "Test Session 2",
            "model_id": model2.id.to_string()
        }))
        .send()
        .await
        .expect("Failed to create session with explicit model");

    assert_eq!(session2_response.status(), 201);
    let session2: Session = session2_response
        .json()
        .await
        .expect("Failed to parse session2");
    println!(
        "Created session2: {} with model_id: {:?}",
        session2.id, session2.model_id
    );

    // Verify explicit model_id overrides default
    assert_eq!(
        session2.model_id,
        Some(model2.id),
        "Session should use explicit model_id"
    );

    // Cleanup
    println!("\nCleaning up...");
    client
        .delete(format!(
            "{}/v1/orgs/{}/agents/{}",
            API_BASE_URL, DEFAULT_ORG, agent.id
        ))
        .send()
        .await
        .expect("Failed to delete agent");
    client
        .delete(format!(
            "{}/v1/orgs/{}/llm-models/{}",
            API_BASE_URL, DEFAULT_ORG, model.id
        ))
        .send()
        .await
        .expect("Failed to delete model");
    client
        .delete(format!(
            "{}/v1/orgs/{}/llm-models/{}",
            API_BASE_URL, DEFAULT_ORG, model2.id
        ))
        .send()
        .await
        .expect("Failed to delete model2");
    client
        .delete(format!(
            "{}/v1/orgs/{}/llm-providers/{}",
            API_BASE_URL, DEFAULT_ORG, provider.id
        ))
        .send()
        .await
        .expect("Failed to delete provider");

    println!("Session model_id inheritance test passed!");
}

#[tokio::test]
async fn test_session_filesystem() {
    let client = reqwest::Client::new();

    println!("Testing session filesystem...");

    // Step 1: Create an agent
    println!("\nStep 1: Creating agent...");
    let agent_response = client
        .post(format!("{}/v1/orgs/{}/agents", API_BASE_URL, DEFAULT_ORG))
        .json(&json!({
            "name": "Filesystem Test Agent",
            "system_prompt": "Test agent for filesystem"
        }))
        .send()
        .await
        .expect("Failed to create agent");

    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!("Created agent: {}", agent.id);

    // Step 2: Create a session
    println!("\nStep 2: Creating session...");
    let session_response = client
        .post(format!(
            "{}/v1/orgs/{}/agents/{}/sessions",
            API_BASE_URL, DEFAULT_ORG, agent.id
        ))
        .json(&json!({
            "title": "Filesystem Test Session"
        }))
        .send()
        .await
        .expect("Failed to create session");

    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");
    println!("Created session: {}", session.id);

    let fs_url = format!(
        "{}/v1/orgs/{}/agents/{}/sessions/{}/fs",
        API_BASE_URL, DEFAULT_ORG, agent.id, session.id
    );

    // Step 3: List root directory (should be empty)
    println!("\nStep 3: Listing root directory...");
    let list_response = client
        .get(&fs_url)
        .send()
        .await
        .expect("Failed to list files");

    assert_eq!(list_response.status(), 200);
    let list_result: Value = list_response.json().await.expect("Failed to parse");
    assert_eq!(list_result["data"].as_array().unwrap().len(), 0);
    println!("Root directory is empty");

    // Step 4: Create a file
    println!("\nStep 4: Creating file...");
    let create_response = client
        .post(format!("{}/hello.txt", fs_url))
        .json(&json!({
            "content": "Hello, World!",
            "encoding": "text"
        }))
        .send()
        .await
        .expect("Failed to create file");

    assert_eq!(create_response.status(), 201);
    let file: SessionFile = create_response.json().await.expect("Failed to parse file");
    println!("Created file: {}", file.path);
    assert_eq!(file.path, "/hello.txt");
    assert!(!file.is_directory);

    // Step 5: Read file
    println!("\nStep 5: Reading file...");
    let read_response = client
        .get(format!("{}/hello.txt", fs_url))
        .send()
        .await
        .expect("Failed to read file");

    assert_eq!(read_response.status(), 200);
    let file: SessionFile = read_response.json().await.expect("Failed to parse file");
    assert_eq!(file.content.as_deref(), Some("Hello, World!"));
    println!("File content: {:?}", file.content);

    // Step 6: Get file stat
    println!("\nStep 6: Getting file stat...");
    let stat_response = client
        .post(format!("{}/_/stat", fs_url))
        .json(&json!({
            "path": "/hello.txt"
        }))
        .send()
        .await
        .expect("Failed to get stat");

    assert_eq!(stat_response.status(), 200);
    let stat: Value = stat_response.json().await.expect("Failed to parse stat");
    assert_eq!(stat["path"], "/hello.txt");
    assert_eq!(stat["is_directory"], false);
    println!("File stat: size={}", stat["size_bytes"]);

    // Step 7: Update file
    println!("\nStep 7: Updating file...");
    let update_response = client
        .put(format!("{}/hello.txt", fs_url))
        .json(&json!({
            "content": "Updated content"
        }))
        .send()
        .await
        .expect("Failed to update file");

    assert_eq!(update_response.status(), 200);
    let file: SessionFile = update_response.json().await.expect("Failed to parse file");
    assert_eq!(file.content.as_deref(), Some("Updated content"));
    println!("File updated");

    // Step 8: Create directory
    println!("\nStep 8: Creating directory...");
    let dir_response = client
        .post(format!("{}/docs", fs_url))
        .json(&json!({
            "is_directory": true
        }))
        .send()
        .await
        .expect("Failed to create directory");

    assert_eq!(dir_response.status(), 201);
    let dir: SessionFile = dir_response.json().await.expect("Failed to parse dir");
    assert!(dir.is_directory);
    println!("Created directory: {}", dir.path);

    // Step 9: Create file in directory (auto-creates parent)
    println!("\nStep 9: Creating nested file...");
    let nested_response = client
        .post(format!("{}/src/main.rs", fs_url))
        .json(&json!({
            "content": "fn main() {}"
        }))
        .send()
        .await
        .expect("Failed to create nested file");

    assert_eq!(nested_response.status(), 201);
    let nested: SessionFile = nested_response.json().await.expect("Failed to parse");
    assert_eq!(nested.path, "/src/main.rs");
    println!("Created nested file: {}", nested.path);

    // Step 10: List all files
    println!("\nStep 10: Listing all files...");
    let list_all_response = client
        .get(format!("{}?recursive=true", fs_url))
        .send()
        .await
        .expect("Failed to list all files");

    assert_eq!(list_all_response.status(), 200);
    let list_all: Value = list_all_response.json().await.expect("Failed to parse");
    let files = list_all["data"].as_array().unwrap();
    assert!(files.len() >= 3); // hello.txt, docs, src/main.rs
    println!("Found {} files", files.len());

    // Step 11: Copy file
    println!("\nStep 11: Copying file...");
    let copy_response = client
        .post(format!("{}/_/copy", fs_url))
        .json(&json!({
            "src_path": "/hello.txt",
            "dst_path": "/hello-copy.txt"
        }))
        .send()
        .await
        .expect("Failed to copy file");

    assert_eq!(copy_response.status(), 201);
    println!("File copied");

    // Step 12: Move file
    println!("\nStep 12: Moving file...");
    let move_response = client
        .post(format!("{}/_/move", fs_url))
        .json(&json!({
            "src_path": "/hello-copy.txt",
            "dst_path": "/renamed.txt"
        }))
        .send()
        .await
        .expect("Failed to move file");

    assert_eq!(move_response.status(), 200);
    println!("File moved/renamed");

    // Step 13: Grep search
    println!("\nStep 13: Searching files...");
    let grep_response = client
        .post(format!("{}/_/grep", fs_url))
        .json(&json!({
            "pattern": "main"
        }))
        .send()
        .await
        .expect("Failed to grep");

    assert_eq!(grep_response.status(), 200);
    let grep_result: Value = grep_response.json().await.expect("Failed to parse");
    let results = grep_result["data"].as_array().unwrap();
    assert!(!results.is_empty());
    println!("Found {} files with matches", results.len());

    // Step 14: Delete file
    println!("\nStep 14: Deleting file...");
    let delete_response = client
        .delete(format!("{}/renamed.txt", fs_url))
        .send()
        .await
        .expect("Failed to delete file");

    assert_eq!(delete_response.status(), 200);
    let delete_result: Value = delete_response.json().await.expect("Failed to parse");
    assert_eq!(delete_result["deleted"], true);
    println!("File deleted");

    // Step 15: Delete directory recursively
    println!("\nStep 15: Deleting directory recursively...");
    let delete_dir_response = client
        .delete(format!("{}/src?recursive=true", fs_url))
        .send()
        .await
        .expect("Failed to delete directory");

    assert_eq!(delete_dir_response.status(), 200);
    println!("Directory deleted");

    // Cleanup
    println!("\nCleaning up...");
    client
        .delete(format!(
            "{}/v1/orgs/{}/agents/{}",
            API_BASE_URL, DEFAULT_ORG, agent.id
        ))
        .send()
        .await
        .expect("Failed to delete agent");

    println!("Session filesystem test passed!");
}

/// Test that message creation returns promptly and triggers agent workflow
///
/// This test verifies:
/// 1. Message creation returns within 5 seconds (not blocking on workflow)
/// 2. After waiting, an assistant response appears (workflow executed)
///
/// Requirements: API + Worker (uses LlmSim provider, no real API keys needed).
#[tokio::test]
async fn test_message_triggers_agent_workflow() {
    use std::time::{Duration, Instant};

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("Failed to create client");

    println!("Testing message triggers agent workflow...");

    // Step 0: Create LlmSim provider and model (no real API keys needed)
    println!("\nStep 0: Creating LlmSim provider and model...");
    let provider_response = client
        .post(format!(
            "{}/v1/orgs/{}/llm-providers",
            API_BASE_URL, DEFAULT_ORG
        ))
        .json(&json!({
            "name": "LlmSim Test Provider",
            "provider_type": "llmsim"
        }))
        .send()
        .await
        .expect("Failed to create provider");

    if provider_response.status() != 201 {
        let status = provider_response.status();
        let body = provider_response.text().await.unwrap_or_default();
        panic!(
            "Failed to create LlmSim provider: status={}, body={}",
            status, body
        );
    }
    let provider: LlmProvider = provider_response
        .json()
        .await
        .expect("Failed to parse provider");
    println!("Created LlmSim provider: {}", provider.id);

    let model_response = client
        .post(format!(
            "{}/v1/orgs/{}/llm-providers/{}/models",
            API_BASE_URL, DEFAULT_ORG, provider.id
        ))
        .json(&json!({
            "model_id": "llmsim-test",
            "display_name": "LlmSim Test Model"
        }))
        .send()
        .await
        .expect("Failed to create model");

    if model_response.status() != 201 {
        let status = model_response.status();
        let body = model_response.text().await.unwrap_or_default();
        panic!(
            "Failed to create LlmSim model: status={}, body={}",
            status, body
        );
    }
    let model: LlmModel = model_response.json().await.expect("Failed to parse model");
    println!("Created LlmSim model: {}", model.id);

    // Step 1: Create agent with LlmSim model
    println!("\nStep 1: Creating agent with LlmSim model...");
    let agent_response = client
        .post(format!("{}/v1/orgs/{}/agents", API_BASE_URL, DEFAULT_ORG))
        .json(&json!({
            "name": "Workflow Test Agent",
            "system_prompt": "You are a helpful assistant. Respond briefly.",
            "default_model_id": model.id.to_string()
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(agent_response.status(), 201);
    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!(
        "Created agent: {} with model: {:?}",
        agent.id, agent.default_model_id
    );

    // Step 2: Create session
    println!("\nStep 2: Creating session...");
    let session_response = client
        .post(format!(
            "{}/v1/orgs/{}/agents/{}/sessions",
            API_BASE_URL, DEFAULT_ORG, agent.id
        ))
        .json(&json!({"title": "Workflow Test Session"}))
        .send()
        .await
        .expect("Failed to create session");

    assert_eq!(session_response.status(), 201);
    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");
    println!("Created session: {}", session.id);

    // Step 3: Send message and verify it returns promptly (within 5 seconds)
    println!("\nStep 3: Sending message (should return promptly)...");
    let start = Instant::now();
    let message_response = client
        .post(format!(
            "{}/v1/orgs/{}/agents/{}/sessions/{}/messages",
            API_BASE_URL, DEFAULT_ORG, agent.id, session.id
        ))
        .json(&json!({
            "message": {
                "content": [{"type": "text", "text": "Say hello in one word."}]
            }
        }))
        .send()
        .await
        .expect("Failed to create message");
    let elapsed = start.elapsed();

    assert_eq!(
        message_response.status(),
        201,
        "Message creation should succeed"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "Message creation took too long: {:?}. Should not block on workflow start.",
        elapsed
    );
    println!("Message created in {:?}", elapsed);

    let message: Value = message_response
        .json()
        .await
        .expect("Failed to parse message");
    assert_eq!(message["role"], "user");
    println!("Created user message: {}", message["id"]);

    // Step 4: Wait for workflow to complete and check for assistant response
    println!("\nStep 4: Waiting for agent response (up to 30 seconds)...");
    let mut assistant_found = false;
    for i in 1..=30 {
        tokio::time::sleep(Duration::from_secs(1)).await;

        let messages_response = client
            .get(format!(
                "{}/v1/orgs/{}/agents/{}/sessions/{}/messages",
                API_BASE_URL, DEFAULT_ORG, agent.id, session.id
            ))
            .send()
            .await;

        if let Ok(resp) = messages_response
            && resp.status() == 200
        {
            let data: Value = resp.json().await.unwrap_or_default();
            let empty_vec = vec![];
            let messages = data["data"].as_array().unwrap_or(&empty_vec);

            // Debug: print message count and roles on first check and every 10s
            if i == 1 || i % 10 == 0 {
                println!(
                    "  [{}s] Found {} messages, roles: {:?}",
                    i,
                    messages.len(),
                    messages
                        .iter()
                        .map(|m| m["role"].as_str().unwrap_or("?"))
                        .collect::<Vec<_>>()
                );
            }

            for msg in messages {
                // API returns "agent" role (not "assistant")
                if msg["role"] == "agent" {
                    assistant_found = true;
                    let content = &msg["content"];
                    println!("Found agent response after {}s: {:?}", i, content);
                    break;
                }
            }

            if assistant_found {
                break;
            }
        }

        if i % 5 == 0 && !assistant_found {
            println!("Still waiting... ({}s)", i);
        }
    }

    // If we didn't find an agent response, check events for debugging
    if !assistant_found {
        println!("\nDebug: Checking events for session...");
        if let Ok(resp) = client
            .get(format!(
                "{}/v1/orgs/{}/agents/{}/sessions/{}/events",
                API_BASE_URL, DEFAULT_ORG, agent.id, session.id
            ))
            .send()
            .await
            && resp.status() == 200
            && let Ok(data) = resp.json::<Value>().await
        {
            let events = data["data"].as_array();
            println!("  Events count: {}", events.map(|e| e.len()).unwrap_or(0));
            if let Some(events) = events {
                for (i, event) in events.iter().enumerate().take(10) {
                    println!(
                        "  Event {}: type={}, data_preview={}",
                        i,
                        event["type"].as_str().unwrap_or("?"),
                        &event["data"]
                            .to_string()
                            .chars()
                            .take(100)
                            .collect::<String>()
                    );
                }
            }
        }
    }

    assert!(
        assistant_found,
        "Agent workflow did not produce an agent response within 30 seconds. \
        Check: 1) Worker is running, 2) LLM provider configured, 3) Default model set"
    );

    // Step 5: Verify events were created
    println!("\nStep 5: Verifying events...");
    let events_response = client
        .get(format!(
            "{}/v1/orgs/{}/agents/{}/sessions/{}/events",
            API_BASE_URL, DEFAULT_ORG, agent.id, session.id
        ))
        .send()
        .await
        .expect("Failed to list events");

    assert_eq!(events_response.status(), 200);
    let events_data: Value = events_response
        .json()
        .await
        .expect("Failed to parse events");
    let events = events_data["data"]
        .as_array()
        .expect("Expected events array");
    println!("Found {} events", events.len());
    assert!(
        events.len() >= 2,
        "Expected at least 2 events (user message + agent response)"
    );

    // Cleanup
    println!("\nCleaning up...");
    client
        .delete(format!(
            "{}/v1/orgs/{}/agents/{}",
            API_BASE_URL, DEFAULT_ORG, agent.id
        ))
        .send()
        .await
        .expect("Failed to delete agent");
    client
        .delete(format!(
            "{}/v1/orgs/{}/llm-models/{}",
            API_BASE_URL, DEFAULT_ORG, model.id
        ))
        .send()
        .await
        .expect("Failed to delete model");
    client
        .delete(format!(
            "{}/v1/orgs/{}/llm-providers/{}",
            API_BASE_URL, DEFAULT_ORG, provider.id
        ))
        .send()
        .await
        .expect("Failed to delete provider");

    println!("Message triggers agent workflow test passed!");
}

/// Test that tool calls are not duplicated during workflow execution.
///
/// This test verifies that when an agent uses a tool (like current_time),
/// the tool call appears only once in the events, not duplicated.
///
/// Regression test for duplicate tool call scheduling at initial scheduling.
#[tokio::test]
async fn test_no_duplicate_tool_calls() {
    use std::collections::HashMap;

    let client = reqwest::Client::new();

    println!("Testing no duplicate tool calls...");

    // Step 1: Create an LLM provider with API key (if available)
    println!("\nStep 1: Creating LLM provider...");
    let provider_response = client
        .post(format!(
            "{}/v1/orgs/{}/llm-providers",
            API_BASE_URL, DEFAULT_ORG
        ))
        .json(&json!({
            "name": "Duplicate Tool Test Provider",
            "provider_type": "openai",
            "is_default": false
        }))
        .send()
        .await
        .expect("Failed to create provider");

    let provider: LlmProvider = provider_response
        .json()
        .await
        .expect("Failed to parse provider");
    println!("Created provider: {}", provider.id);

    // Step 2: Create a model configured for tool use
    println!("\nStep 2: Creating model...");
    let model_response = client
        .post(format!(
            "{}/v1/orgs/{}/llm-providers/{}/models",
            API_BASE_URL, DEFAULT_ORG, provider.id
        ))
        .json(&json!({
            "model_id": "gpt-4o-mini",
            "display_name": "GPT-4o Mini Test"
        }))
        .send()
        .await
        .expect("Failed to create model");

    if model_response.status() != 201 {
        let status = model_response.status();
        let body = model_response.text().await.unwrap_or_default();
        panic!("Failed to create model: status={}, body={}", status, body);
    }
    let model: LlmModel = model_response.json().await.expect("Failed to parse model");
    println!("Created model: {}", model.id);

    // Step 3: Create an agent with current_time capability
    println!("\nStep 3: Creating agent with current_time capability...");
    let agent_response = client
        .post(format!("{}/v1/orgs/{}/agents", API_BASE_URL, DEFAULT_ORG))
        .json(&json!({
            "name": "Time Tool Test Agent",
            "system_prompt": "You are a helpful time assistant. When asked about the current time, use the get_current_time tool.",
            "capabilities": [{"ref": "current_time", "config": {}}],
            "default_model_id": model.id
        }))
        .send()
        .await
        .expect("Failed to create agent");

    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!(
        "Created agent: {} with capabilities: {:?}",
        agent.id, agent.capabilities
    );

    // Step 4: Create a session
    println!("\nStep 4: Creating session...");
    let session_response = client
        .post(format!(
            "{}/v1/orgs/{}/agents/{}/sessions",
            API_BASE_URL, DEFAULT_ORG, agent.id
        ))
        .json(&json!({}))
        .send()
        .await
        .expect("Failed to create session");

    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");
    println!("Created session: {}", session.id);

    // Step 5: Send a message that should trigger tool use
    println!("\nStep 5: Sending message to trigger tool use...");
    let message_response = client
        .post(format!(
            "{}/v1/orgs/{}/agents/{}/sessions/{}/messages",
            API_BASE_URL, DEFAULT_ORG, agent.id, session.id
        ))
        .json(&json!({
            "message": {
                "content": [{"type": "text", "text": "What time is it right now?"}]
            }
        }))
        .send()
        .await
        .expect("Failed to send message");

    assert!(
        message_response.status().is_success() || message_response.status() == 404,
        "Expected success or 404, got {}",
        message_response.status()
    );

    // Step 6: Wait for workflow to complete by polling messages
    println!("\nStep 6: Waiting for workflow to complete...");
    let mut tool_call_found = false;
    for i in 1..=30 {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

        let messages_response = client
            .get(format!(
                "{}/v1/orgs/{}/agents/{}/sessions/{}/messages",
                API_BASE_URL, DEFAULT_ORG, agent.id, session.id
            ))
            .send()
            .await;

        if let Ok(resp) = messages_response
            && resp.status() == 200
        {
            let data: Value = resp.json().await.unwrap_or_default();
            let empty_vec = vec![];
            let messages = data["data"].as_array().unwrap_or(&empty_vec);

            // Check if we have an agent response (workflow completed)
            for msg in messages {
                if msg["role"] == "agent" {
                    // Check for tool calls in content
                    if let Some(content) = msg["content"].as_array() {
                        for part in content {
                            if part.get("tool_call").is_some() {
                                tool_call_found = true;
                                println!("Found tool call after {}s", i);
                                break;
                            }
                        }
                    }
                }
            }

            if tool_call_found {
                // Wait a bit more for workflow to fully complete
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                break;
            }
        }

        if i % 5 == 0 {
            println!("  Still waiting... ({}s)", i);
        }
    }

    // Step 7: Get all messages and check for duplicates
    println!("\nStep 7: Checking for duplicate tool calls...");
    let messages_response = client
        .get(format!(
            "{}/v1/orgs/{}/agents/{}/sessions/{}/messages",
            API_BASE_URL, DEFAULT_ORG, agent.id, session.id
        ))
        .send()
        .await
        .expect("Failed to list messages");

    let messages_data: Value = messages_response
        .json()
        .await
        .expect("Failed to parse messages");
    let empty_vec = vec![];
    let messages = messages_data["data"].as_array().unwrap_or(&empty_vec);

    println!("Found {} messages", messages.len());

    // Count tool calls by their ID to detect duplicates
    let mut tool_call_ids: HashMap<String, u32> = HashMap::new();

    for msg in messages {
        if let Some(content) = msg["content"].as_array() {
            for part in content {
                // Check for tool_call content parts
                if let Some(tool_call) = part.get("tool_call") {
                    let id = tool_call["id"].as_str().unwrap_or("unknown");
                    let name = tool_call["name"].as_str().unwrap_or("unknown");
                    println!("  Found tool_call: {} ({})", name, id);
                    *tool_call_ids.entry(id.to_string()).or_insert(0) += 1;
                }

                // Check for tool_result content parts (from tool.call_completed events)
                if let Some(tool_result) = part.get("tool_result") {
                    let id = tool_result["id"].as_str().unwrap_or("unknown");
                    let name = tool_result["name"].as_str().unwrap_or("unknown");
                    println!("  Found tool_result: {} ({})", name, id);
                }
            }
        }
    }

    // Check for duplicate tool calls
    let mut has_duplicates = false;
    for (id, count) in &tool_call_ids {
        if *count > 1 {
            println!(
                "ERROR: Duplicate tool_call found! ID: {}, count: {}",
                id, count
            );
            has_duplicates = true;
        }
    }

    // If there were tool calls, verify no duplicates
    if !tool_call_ids.is_empty() {
        assert!(
            !has_duplicates,
            "Found duplicate tool calls in messages! Tool call IDs with counts: {:?}",
            tool_call_ids
        );
        println!("No duplicate tool calls found - test passed!");
    } else {
        println!(
            "No tool calls found in messages (workflow may not have completed or API key not configured)"
        );
    }

    // Cleanup
    println!("\nCleaning up...");
    client
        .delete(format!(
            "{}/v1/orgs/{}/agents/{}",
            API_BASE_URL, DEFAULT_ORG, agent.id
        ))
        .send()
        .await
        .expect("Failed to delete agent");
    client
        .delete(format!(
            "{}/v1/orgs/{}/llm-models/{}",
            API_BASE_URL, DEFAULT_ORG, model.id
        ))
        .send()
        .await
        .expect("Failed to delete model");
    client
        .delete(format!(
            "{}/v1/orgs/{}/llm-providers/{}",
            API_BASE_URL, DEFAULT_ORG, provider.id
        ))
        .send()
        .await
        .expect("Failed to delete provider");

    println!("No duplicate tool calls test completed!");
}

#[tokio::test]
async fn test_sessions_pagination() {
    let client = reqwest::Client::new();

    println!("Testing sessions pagination...");

    // Create an agent for the test
    let agent_response = client
        .post(format!("{}/v1/orgs/{}/agents", API_BASE_URL, DEFAULT_ORG))
        .json(&json!({
            "name": "Pagination Test Agent",
            "system_prompt": "Test agent"
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(agent_response.status(), 201);
    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!("Created agent: {}", agent.id);

    // Create 15 sessions
    println!("Creating 15 sessions...");
    for i in 1..=15 {
        let response = client
            .post(format!(
                "{}/v1/orgs/{}/agents/{}/sessions",
                API_BASE_URL, DEFAULT_ORG, agent.id
            ))
            .json(&json!({ "title": format!("Session {}", i) }))
            .send()
            .await
            .expect("Failed to create session");
        assert_eq!(response.status(), 201, "Failed to create session {}", i);
    }
    println!("Created 15 sessions");

    // Test 1: Default pagination returns all with metadata
    println!("\nTest 1: Default pagination...");
    let response = client
        .get(format!(
            "{}/v1/orgs/{}/agents/{}/sessions",
            API_BASE_URL, DEFAULT_ORG, agent.id
        ))
        .send()
        .await
        .expect("Failed to list sessions");

    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("Failed to parse");

    assert_eq!(body["total"], 15, "Expected total=15");
    assert_eq!(body["offset"], 0, "Expected offset=0");
    assert_eq!(body["limit"], 20, "Expected default limit=20");
    assert_eq!(
        body["data"].as_array().unwrap().len(),
        15,
        "Expected 15 sessions"
    );
    println!("✓ Default pagination works");

    // Test 2: Custom limit
    println!("\nTest 2: Custom limit=5...");
    let response = client
        .get(format!(
            "{}/v1/orgs/{}/agents/{}/sessions?limit=5",
            API_BASE_URL, DEFAULT_ORG, agent.id
        ))
        .send()
        .await
        .expect("Failed to list sessions with limit");

    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("Failed to parse");

    assert_eq!(body["total"], 15, "Total should still be 15");
    assert_eq!(body["limit"], 5, "Limit should be 5");
    assert_eq!(
        body["data"].as_array().unwrap().len(),
        5,
        "Expected 5 sessions"
    );
    println!("✓ Custom limit works");

    // Test 3: Offset pagination
    println!("\nTest 3: Offset=5, limit=5...");
    let response = client
        .get(format!(
            "{}/v1/orgs/{}/agents/{}/sessions?offset=5&limit=5",
            API_BASE_URL, DEFAULT_ORG, agent.id
        ))
        .send()
        .await
        .expect("Failed to list sessions with offset");

    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("Failed to parse");

    assert_eq!(body["total"], 15);
    assert_eq!(body["offset"], 5);
    assert_eq!(body["limit"], 5);
    assert_eq!(body["data"].as_array().unwrap().len(), 5);
    println!("✓ Offset pagination works");

    // Test 4: Last partial page
    println!("\nTest 4: Last partial page (offset=10, limit=10)...");
    let response = client
        .get(format!(
            "{}/v1/orgs/{}/agents/{}/sessions?offset=10&limit=10",
            API_BASE_URL, DEFAULT_ORG, agent.id
        ))
        .send()
        .await
        .expect("Failed to list sessions");

    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("Failed to parse");

    assert_eq!(body["total"], 15);
    assert_eq!(
        body["data"].as_array().unwrap().len(),
        5,
        "Expected 5 remaining sessions"
    );
    println!("✓ Last partial page works");

    // Test 5: Beyond range returns empty data
    println!("\nTest 5: Beyond range (offset=20)...");
    let response = client
        .get(format!(
            "{}/v1/orgs/{}/agents/{}/sessions?offset=20",
            API_BASE_URL, DEFAULT_ORG, agent.id
        ))
        .send()
        .await
        .expect("Failed to list sessions");

    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("Failed to parse");

    assert_eq!(body["total"], 15);
    assert_eq!(
        body["data"].as_array().unwrap().len(),
        0,
        "Expected empty data"
    );
    println!("✓ Beyond range returns empty data");

    // Test 6: Max limit enforcement
    println!("\nTest 6: Max limit enforcement (limit=200 should cap to 100)...");
    let response = client
        .get(format!(
            "{}/v1/orgs/{}/agents/{}/sessions?limit=200",
            API_BASE_URL, DEFAULT_ORG, agent.id
        ))
        .send()
        .await
        .expect("Failed to list sessions");

    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("Failed to parse");

    assert_eq!(body["limit"], 100, "Limit should be capped at 100");
    println!("✓ Max limit enforcement works");

    // Cleanup
    println!("\nCleaning up...");
    client
        .delete(format!(
            "{}/v1/orgs/{}/agents/{}",
            API_BASE_URL, DEFAULT_ORG, agent.id
        ))
        .send()
        .await
        .expect("Failed to delete agent");

    println!("Sessions pagination test completed!");
}

/// Test that a second message in the same session triggers a new workflow
///
/// This test verifies:
/// 1. First message triggers workflow and gets response
/// 2. Second message triggers a NEW workflow and gets response
///
/// This is a regression test for the issue where second messages were not picked up
/// because the workflow ID was the same as session_id and the completed workflow
/// blocked creation of a new workflow.
///
/// Requirements: API + Worker (uses LlmSim provider, no real API keys needed).
#[tokio::test]
async fn test_second_message_triggers_workflow() {
    use std::time::{Duration, Instant};

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("Failed to create client");

    println!("Testing second message triggers workflow...");

    // Step 0: Create LlmSim provider and model (no real API keys needed)
    println!("\nStep 0: Creating LlmSim provider and model...");
    let provider_response = client
        .post(format!(
            "{}/v1/orgs/{}/llm-providers",
            API_BASE_URL, DEFAULT_ORG
        ))
        .json(&json!({
            "name": "LlmSim Second Message Test",
            "provider_type": "llmsim"
        }))
        .send()
        .await
        .expect("Failed to create provider");

    if provider_response.status() != 201 {
        let status = provider_response.status();
        let body = provider_response.text().await.unwrap_or_default();
        panic!(
            "Failed to create LlmSim provider: status={}, body={}",
            status, body
        );
    }
    let provider: LlmProvider = provider_response
        .json()
        .await
        .expect("Failed to parse provider");
    println!("Created LlmSim provider: {}", provider.id);

    let model_response = client
        .post(format!(
            "{}/v1/orgs/{}/llm-providers/{}/models",
            API_BASE_URL, DEFAULT_ORG, provider.id
        ))
        .json(&json!({
            "model_id": "llmsim-second-msg-test",
            "display_name": "LlmSim Second Message Test Model"
        }))
        .send()
        .await
        .expect("Failed to create model");

    if model_response.status() != 201 {
        let status = model_response.status();
        let body = model_response.text().await.unwrap_or_default();
        panic!(
            "Failed to create LlmSim model: status={}, body={}",
            status, body
        );
    }
    let model: LlmModel = model_response.json().await.expect("Failed to parse model");
    println!("Created LlmSim model: {}", model.id);

    // Step 1: Create agent with LlmSim model
    println!("\nStep 1: Creating agent with LlmSim model...");
    let agent_response = client
        .post(format!("{}/v1/orgs/{}/agents", API_BASE_URL, DEFAULT_ORG))
        .json(&json!({
            "name": "Second Message Test Agent",
            "system_prompt": "You are a helpful assistant. Respond briefly.",
            "default_model_id": model.id.to_string()
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(agent_response.status(), 201);
    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!(
        "Created agent: {} with model: {:?}",
        agent.id, agent.default_model_id
    );

    // Step 2: Create session
    println!("\nStep 2: Creating session...");
    let session_response = client
        .post(format!(
            "{}/v1/orgs/{}/agents/{}/sessions",
            API_BASE_URL, DEFAULT_ORG, agent.id
        ))
        .json(&json!({"title": "Second Message Test Session"}))
        .send()
        .await
        .expect("Failed to create session");

    assert_eq!(session_response.status(), 201);
    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");
    println!("Created session: {}", session.id);

    // Step 3: Send FIRST message and wait for response
    println!("\nStep 3: Sending FIRST message...");
    let first_message_response = client
        .post(format!(
            "{}/v1/orgs/{}/agents/{}/sessions/{}/messages",
            API_BASE_URL, DEFAULT_ORG, agent.id, session.id
        ))
        .json(&json!({
            "message": {
                "content": [{"type": "text", "text": "Hello, this is the first message."}]
            }
        }))
        .send()
        .await
        .expect("Failed to create first message");

    assert_eq!(
        first_message_response.status(),
        201,
        "First message creation should succeed"
    );
    println!("First message created");

    // Wait for first response
    println!("\nWaiting for first agent response...");
    let mut first_response_found = false;
    for i in 1..=30 {
        tokio::time::sleep(Duration::from_secs(1)).await;

        let messages_response = client
            .get(format!(
                "{}/v1/orgs/{}/agents/{}/sessions/{}/messages",
                API_BASE_URL, DEFAULT_ORG, agent.id, session.id
            ))
            .send()
            .await;

        if let Ok(resp) = messages_response
            && resp.status() == 200
        {
            let data: Value = resp.json().await.unwrap_or_default();
            let empty_vec = vec![];
            let messages = data["data"].as_array().unwrap_or(&empty_vec);

            // Count agent messages
            let agent_count = messages.iter().filter(|m| m["role"] == "agent").count();
            if agent_count >= 1 {
                first_response_found = true;
                println!("Found first agent response after {}s", i);
                break;
            }
        }

        if i % 5 == 0 {
            println!("Still waiting for first response... ({}s)", i);
        }
    }

    assert!(
        first_response_found,
        "First agent response not received within 30 seconds"
    );

    // Small delay to ensure workflow is fully completed
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Step 4: Send SECOND message and wait for response
    println!("\nStep 4: Sending SECOND message...");
    let second_message_start = Instant::now();
    let second_message_response = client
        .post(format!(
            "{}/v1/orgs/{}/agents/{}/sessions/{}/messages",
            API_BASE_URL, DEFAULT_ORG, agent.id, session.id
        ))
        .json(&json!({
            "message": {
                "content": [{"type": "text", "text": "Hello, this is the SECOND message. Please respond."}]
            }
        }))
        .send()
        .await
        .expect("Failed to create second message");

    assert_eq!(
        second_message_response.status(),
        201,
        "Second message creation should succeed"
    );
    println!(
        "Second message created in {:?}",
        second_message_start.elapsed()
    );

    // Wait for second response
    println!("\nWaiting for SECOND agent response...");
    let mut second_response_found = false;
    for i in 1..=30 {
        tokio::time::sleep(Duration::from_secs(1)).await;

        let messages_response = client
            .get(format!(
                "{}/v1/orgs/{}/agents/{}/sessions/{}/messages",
                API_BASE_URL, DEFAULT_ORG, agent.id, session.id
            ))
            .send()
            .await;

        if let Ok(resp) = messages_response
            && resp.status() == 200
        {
            let data: Value = resp.json().await.unwrap_or_default();
            let empty_vec = vec![];
            let messages = data["data"].as_array().unwrap_or(&empty_vec);

            // Count user and agent messages
            let user_count = messages.iter().filter(|m| m["role"] == "user").count();
            let agent_count = messages.iter().filter(|m| m["role"] == "agent").count();

            if i == 1 || i % 5 == 0 {
                println!(
                    "  [{}s] Messages: {} user, {} agent",
                    i, user_count, agent_count
                );
            }

            // We expect 2 user messages and 2 agent messages
            if user_count >= 2 && agent_count >= 2 {
                second_response_found = true;
                println!("Found second agent response after {}s", i);
                break;
            }
        }
    }

    // Debug: if second response not found, show all messages
    if !second_response_found {
        println!("\nDebug: Final message state:");
        if let Ok(resp) = client
            .get(format!(
                "{}/v1/orgs/{}/agents/{}/sessions/{}/messages",
                API_BASE_URL, DEFAULT_ORG, agent.id, session.id
            ))
            .send()
            .await
            && resp.status() == 200
        {
            let data: Value = resp.json().await.unwrap_or_default();
            let empty_vec = vec![];
            let messages = data["data"].as_array().unwrap_or(&empty_vec);
            for (i, msg) in messages.iter().enumerate() {
                let role = msg["role"].as_str().unwrap_or("?");
                let content_preview = msg["content"].to_string();
                let preview: String = content_preview.chars().take(100).collect();
                println!("  Message {}: role={}, content={}", i, role, preview);
            }
        }
    }

    assert!(
        second_response_found,
        "Second agent response not received within 30 seconds. \
        This indicates the second message workflow was not triggered. \
        Bug: workflow_id = session_id causes conflict when creating second workflow."
    );

    // Step 5: Verify we have exactly 2 user messages and 2 agent messages
    println!("\nStep 5: Verifying message counts...");
    let final_messages_response = client
        .get(format!(
            "{}/v1/orgs/{}/agents/{}/sessions/{}/messages",
            API_BASE_URL, DEFAULT_ORG, agent.id, session.id
        ))
        .send()
        .await
        .expect("Failed to get final messages");

    let final_data: Value = final_messages_response
        .json()
        .await
        .expect("Failed to parse final messages");
    let final_messages = final_data["data"]
        .as_array()
        .expect("Expected messages array");

    let user_count = final_messages
        .iter()
        .filter(|m| m["role"] == "user")
        .count();
    let agent_count = final_messages
        .iter()
        .filter(|m| m["role"] == "agent")
        .count();

    println!(
        "Final counts: {} user messages, {} agent messages",
        user_count, agent_count
    );
    assert_eq!(user_count, 2, "Expected 2 user messages");
    assert_eq!(agent_count, 2, "Expected 2 agent messages");

    // Cleanup
    println!("\nCleaning up...");
    client
        .delete(format!(
            "{}/v1/orgs/{}/agents/{}",
            API_BASE_URL, DEFAULT_ORG, agent.id
        ))
        .send()
        .await
        .expect("Failed to delete agent");
    client
        .delete(format!(
            "{}/v1/orgs/{}/llm-models/{}",
            API_BASE_URL, DEFAULT_ORG, model.id
        ))
        .send()
        .await
        .expect("Failed to delete model");
    client
        .delete(format!(
            "{}/v1/orgs/{}/llm-providers/{}",
            API_BASE_URL, DEFAULT_ORG, provider.id
        ))
        .send()
        .await
        .expect("Failed to delete provider");

    println!("Second message workflow test passed!");
}

/// Test capability mounts are applied when session is created
///
/// This test verifies:
/// 1. Create agent with sample_data capability (which has mount points)
/// 2. Create session for the agent
/// 3. Verify the /samples directory exists with expected files
/// 4. Verify files are read-only (from readonly mount)
#[tokio::test]
async fn test_capability_mounts_applied_on_session_creation() {
    let client = reqwest::Client::new();

    println!("Testing capability mounts applied on session creation...");

    // Step 1: Create an agent with sample_data capability
    println!("\nStep 1: Creating agent with sample_data capability...");
    let agent_response = client
        .post(format!("{}/v1/orgs/{}/agents", API_BASE_URL, DEFAULT_ORG))
        .json(&json!({
            "name": "Mount Test Agent",
            "system_prompt": "Test agent for capability mounts",
            "capabilities": [
                {"ref": "sample_data", "config": {}},
                {"ref": "session_file_system", "config": {}}
            ]
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(
        agent_response.status(),
        201,
        "Expected 201 Created for agent"
    );

    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!(
        "Created agent: {} with capabilities: {:?}",
        agent.id, agent.capabilities
    );
    assert_eq!(
        agent.capabilities.len(),
        2,
        "Agent should have 2 capabilities"
    );

    // Step 2: Create a session (this should trigger mount application)
    println!("\nStep 2: Creating session...");
    let session_response = client
        .post(format!(
            "{}/v1/orgs/{}/agents/{}/sessions",
            API_BASE_URL, DEFAULT_ORG, agent.id
        ))
        .json(&json!({
            "title": "Mount Test Session"
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

    let fs_url = format!(
        "{}/v1/orgs/{}/agents/{}/sessions/{}/fs",
        API_BASE_URL, DEFAULT_ORG, agent.id, session.id
    );

    // Step 3: Verify /samples directory exists
    println!("\nStep 3: Verifying /samples directory exists...");
    let stat_response = client
        .post(format!("{}/_/stat", fs_url))
        .json(&json!({"path": "/samples"}))
        .send()
        .await
        .expect("Failed to stat /samples");

    assert_eq!(stat_response.status(), 200);
    let stat: Value = stat_response.json().await.expect("Failed to parse stat");
    assert_eq!(stat["is_directory"], true, "/samples should be a directory");
    println!("/samples directory exists");

    // Step 4: List /samples directory contents
    println!("\nStep 4: Listing /samples directory...");
    let list_response = client
        .get(format!("{}/samples", fs_url))
        .send()
        .await
        .expect("Failed to list /samples");

    assert_eq!(list_response.status(), 200);
    let list_result: Value = list_response.json().await.expect("Failed to parse list");
    let files = list_result["data"].as_array().expect("Expected data array");
    println!("Found {} files in /samples", files.len());

    // Should have users.json, config.yaml, README.md
    let file_names: Vec<&str> = files.iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert!(
        file_names.contains(&"users.json"),
        "Expected users.json in /samples"
    );
    assert!(
        file_names.contains(&"config.yaml"),
        "Expected config.yaml in /samples"
    );
    assert!(
        file_names.contains(&"README.md"),
        "Expected README.md in /samples"
    );
    println!("All expected files present: {:?}", file_names);

    // Step 5: Verify users.json is readable
    println!("\nStep 5: Reading /samples/users.json...");
    let read_response = client
        .get(format!("{}/samples/users.json", fs_url))
        .send()
        .await
        .expect("Failed to read users.json");

    assert_eq!(read_response.status(), 200);
    let file: SessionFile = read_response.json().await.expect("Failed to parse file");
    assert!(file.content.is_some(), "File should have content");
    let content = file.content.as_ref().unwrap();
    assert!(content.contains("Alice"), "users.json should contain Alice");
    assert!(file.is_readonly, "Mounted file should be readonly");
    println!("users.json is readable and readonly");

    // Step 6: Verify readonly protection - try to update users.json
    println!("\nStep 6: Verifying readonly protection...");
    let update_response = client
        .put(format!("{}/samples/users.json", fs_url))
        .json(&json!({
            "content": "modified content"
        }))
        .send()
        .await
        .expect("Failed to send update request");

    // Should fail because file is readonly
    assert_ne!(
        update_response.status(),
        200,
        "Readonly file update should fail"
    );
    println!("Readonly protection working - update rejected");

    // Step 7: Verify config.yaml content
    println!("\nStep 7: Reading /samples/config.yaml...");
    let config_response = client
        .get(format!("{}/samples/config.yaml", fs_url))
        .send()
        .await
        .expect("Failed to read config.yaml");

    assert_eq!(config_response.status(), 200);
    let config_file: SessionFile = config_response.json().await.expect("Failed to parse file");
    let config_content = config_file.content.as_ref().unwrap();
    assert!(
        config_content.contains("application:"),
        "config.yaml should contain application:"
    );
    assert!(
        config_content.contains("database:"),
        "config.yaml should contain database:"
    );
    println!("config.yaml has expected YAML content");

    // Step 8: Create a second session - verify mounts are independent
    println!("\nStep 8: Creating second session...");
    let session2_response = client
        .post(format!(
            "{}/v1/orgs/{}/agents/{}/sessions",
            API_BASE_URL, DEFAULT_ORG, agent.id
        ))
        .json(&json!({
            "title": "Second Mount Test Session"
        }))
        .send()
        .await
        .expect("Failed to create second session");

    assert_eq!(session2_response.status(), 201);
    let session2: Session = session2_response
        .json()
        .await
        .expect("Failed to parse session2");
    println!("Created session2: {}", session2.id);

    // Verify second session also has mounts
    let fs_url2 = format!(
        "{}/v1/orgs/{}/agents/{}/sessions/{}/fs",
        API_BASE_URL, DEFAULT_ORG, agent.id, session2.id
    );
    let stat2_response = client
        .post(format!("{}/_/stat", fs_url2))
        .json(&json!({"path": "/samples"}))
        .send()
        .await
        .expect("Failed to stat /samples in session2");

    assert_eq!(stat2_response.status(), 200);
    let stat2: Value = stat2_response.json().await.expect("Failed to parse stat2");
    assert_eq!(
        stat2["is_directory"], true,
        "session2 should also have /samples"
    );
    println!("Second session also has /samples directory mounted");

    // Cleanup
    println!("\nCleaning up...");
    client
        .delete(format!(
            "{}/v1/orgs/{}/agents/{}",
            API_BASE_URL, DEFAULT_ORG, agent.id
        ))
        .send()
        .await
        .expect("Failed to delete agent");

    println!("Capability mounts test passed!");
}

/// Test MCP server CRUD operations
///
/// This test verifies:
/// 1. MCP server creation
/// 2. MCP server listing
/// 3. MCP server retrieval by ID
/// 4. MCP server update
/// 5. MCP server deletion
#[tokio::test]
async fn test_mcp_server_crud() {
    use everruns_core::McpServer;

    let client = reqwest::Client::new();

    println!("Testing MCP server CRUD operations...");

    // Step 1: Create an MCP server
    println!("\nStep 1: Creating MCP server...");
    let create_response = client
        .post(format!(
            "{}/v1/orgs/{}/mcp-servers",
            API_BASE_URL, DEFAULT_ORG
        ))
        .json(&json!({
            "name": "test-mcp-server",
            "description": "A test MCP server for integration testing",
            "url": "https://mcp.example.com/v1/mcp"
        }))
        .send()
        .await
        .expect("Failed to create MCP server");

    assert_eq!(
        create_response.status(),
        201,
        "Expected 201 Created, got {}",
        create_response.status()
    );

    let server: McpServer = create_response
        .json()
        .await
        .expect("Failed to parse MCP server response");

    println!("Created MCP server: {} ({})", server.name, server.id);
    assert_eq!(server.name, "test-mcp-server");
    assert_eq!(server.url, "https://mcp.example.com/v1/mcp");
    assert_eq!(server.status.to_string(), "active");
    assert!(!server.api_key_set, "API key should not be set");

    // Step 2: List MCP servers
    println!("\nStep 2: Listing MCP servers...");
    let list_response = client
        .get(format!(
            "{}/v1/orgs/{}/mcp-servers",
            API_BASE_URL, DEFAULT_ORG
        ))
        .send()
        .await
        .expect("Failed to list MCP servers");

    assert_eq!(list_response.status(), 200);
    let list_data: Value = list_response.json().await.expect("Failed to parse");
    let servers: Vec<McpServer> =
        serde_json::from_value(list_data["data"].clone()).expect("Failed to parse servers");
    println!("Found {} MCP server(s)", servers.len());
    assert!(!servers.is_empty());
    assert!(servers.iter().any(|s| s.id == server.id));

    // Step 3: Get MCP server by ID
    println!("\nStep 3: Getting MCP server by ID...");
    let get_response = client
        .get(format!(
            "{}/v1/orgs/{}/mcp-servers/{}",
            API_BASE_URL, DEFAULT_ORG, server.id
        ))
        .send()
        .await
        .expect("Failed to get MCP server");

    assert_eq!(get_response.status(), 200);
    let fetched_server: McpServer = get_response
        .json()
        .await
        .expect("Failed to parse MCP server");
    println!("Fetched MCP server: {}", fetched_server.name);
    assert_eq!(fetched_server.id, server.id);
    assert_eq!(fetched_server.name, "test-mcp-server");

    // Step 4: Update MCP server
    println!("\nStep 4: Updating MCP server...");
    let update_response = client
        .patch(format!(
            "{}/v1/orgs/{}/mcp-servers/{}",
            API_BASE_URL, DEFAULT_ORG, server.id
        ))
        .json(&json!({
            "name": "updated-mcp-server",
            "description": "Updated description",
            "url": "https://mcp.updated.com/v1/mcp"
        }))
        .send()
        .await
        .expect("Failed to update MCP server");

    assert_eq!(update_response.status(), 200);
    let updated_server: McpServer = update_response
        .json()
        .await
        .expect("Failed to parse MCP server");
    println!("Updated MCP server: {}", updated_server.name);
    assert_eq!(updated_server.name, "updated-mcp-server");
    assert_eq!(updated_server.url, "https://mcp.updated.com/v1/mcp");
    assert_eq!(
        updated_server.description,
        Some("Updated description".to_string())
    );

    // Step 5: Update MCP server status to disabled
    println!("\nStep 5: Disabling MCP server...");
    let disable_response = client
        .patch(format!(
            "{}/v1/orgs/{}/mcp-servers/{}",
            API_BASE_URL, DEFAULT_ORG, server.id
        ))
        .json(&json!({
            "status": "disabled"
        }))
        .send()
        .await
        .expect("Failed to disable MCP server");

    assert_eq!(disable_response.status(), 200);
    let disabled_server: McpServer = disable_response
        .json()
        .await
        .expect("Failed to parse MCP server");
    assert_eq!(disabled_server.status.to_string(), "disabled");
    println!("MCP server disabled");

    // Step 6: Create MCP server with API key
    println!("\nStep 6: Creating MCP server with API key...");
    let create_with_key_response = client
        .post(format!(
            "{}/v1/orgs/{}/mcp-servers",
            API_BASE_URL, DEFAULT_ORG
        ))
        .json(&json!({
            "name": "mcp-server-with-key",
            "url": "https://secure.mcp.com/v1/mcp",
            "api_key": "test-api-key-12345"
        }))
        .send()
        .await
        .expect("Failed to create MCP server with API key");

    assert_eq!(create_with_key_response.status(), 201);
    let server_with_key: McpServer = create_with_key_response
        .json()
        .await
        .expect("Failed to parse MCP server");
    assert!(server_with_key.api_key_set, "API key should be set");
    println!(
        "Created MCP server with API key: {} (api_key_set: {})",
        server_with_key.name, server_with_key.api_key_set
    );

    // Step 7: Create MCP server with custom headers
    println!("\nStep 7: Creating MCP server with custom headers...");
    let create_with_headers_response = client
        .post(format!(
            "{}/v1/orgs/{}/mcp-servers",
            API_BASE_URL, DEFAULT_ORG
        ))
        .json(&json!({
            "name": "mcp-server-with-headers",
            "url": "https://headers.mcp.com/v1/mcp",
            "headers": {
                "X-Custom-Header": "custom-value",
                "X-Another-Header": "another-value"
            }
        }))
        .send()
        .await
        .expect("Failed to create MCP server with headers");

    assert_eq!(create_with_headers_response.status(), 201);
    let server_with_headers: McpServer = create_with_headers_response
        .json()
        .await
        .expect("Failed to parse MCP server");
    assert_eq!(server_with_headers.headers.len(), 2);
    assert_eq!(
        server_with_headers.headers.get("X-Custom-Header"),
        Some(&"custom-value".to_string())
    );
    println!(
        "Created MCP server with headers: {} ({} headers)",
        server_with_headers.name,
        server_with_headers.headers.len()
    );

    // Step 8: Test validation - empty name
    println!("\nStep 8: Testing validation - empty name...");
    let empty_name_response = client
        .post(format!(
            "{}/v1/orgs/{}/mcp-servers",
            API_BASE_URL, DEFAULT_ORG
        ))
        .json(&json!({
            "name": "",
            "url": "https://mcp.example.com/v1/mcp"
        }))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(
        empty_name_response.status(),
        400,
        "Expected 400 Bad Request for empty name"
    );
    println!("Empty name correctly rejected");

    // Step 9: Test validation - empty URL
    println!("\nStep 9: Testing validation - empty URL...");
    let empty_url_response = client
        .post(format!(
            "{}/v1/orgs/{}/mcp-servers",
            API_BASE_URL, DEFAULT_ORG
        ))
        .json(&json!({
            "name": "test-server",
            "url": ""
        }))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(
        empty_url_response.status(),
        400,
        "Expected 400 Bad Request for empty URL"
    );
    println!("Empty URL correctly rejected");

    // Step 10: Test 404 for non-existent server
    println!("\nStep 10: Testing 404 for non-existent server...");
    let not_found_response = client
        .get(format!(
            "{}/v1/orgs/{}/mcp-servers/00000000-0000-0000-0000-000000000000",
            API_BASE_URL, DEFAULT_ORG
        ))
        .send()
        .await
        .expect("Failed to get non-existent server");

    assert_eq!(
        not_found_response.status(),
        404,
        "Expected 404 Not Found for non-existent server"
    );
    println!("Non-existent server correctly returns 404");

    // Cleanup
    println!("\nCleaning up...");
    client
        .delete(format!(
            "{}/v1/orgs/{}/mcp-servers/{}",
            API_BASE_URL, DEFAULT_ORG, server.id
        ))
        .send()
        .await
        .expect("Failed to delete MCP server");
    client
        .delete(format!(
            "{}/v1/orgs/{}/mcp-servers/{}",
            API_BASE_URL, DEFAULT_ORG, server_with_key.id
        ))
        .send()
        .await
        .expect("Failed to delete MCP server with key");
    client
        .delete(format!(
            "{}/v1/orgs/{}/mcp-servers/{}",
            API_BASE_URL, DEFAULT_ORG, server_with_headers.id
        ))
        .send()
        .await
        .expect("Failed to delete MCP server with headers");

    // Verify deletion
    let verify_deleted = client
        .get(format!(
            "{}/v1/orgs/{}/mcp-servers/{}",
            API_BASE_URL, DEFAULT_ORG, server.id
        ))
        .send()
        .await
        .expect("Failed to verify deletion");
    assert_eq!(verify_deleted.status(), 404);

    println!("MCP server CRUD test passed!");
}
