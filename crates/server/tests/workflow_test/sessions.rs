use crate::support::*;
use everruns_platform::Agent;
use everruns_platform::Session;
use everruns_provider::model::Model;
use everruns_provider::provider::Provider;
use serde_json::{Value, json};

#[tokio::test]
async fn test_full_agent_session_workflow() {
    let client = reqwest::Client::new();

    println!("Testing full agent/session workflow...");

    // Step 1: Create an agent
    println!("\nStep 1: Creating agent...");
    let create_agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "test-agent",
            "display_name": "Test Agent",
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

    println!("Created agent: {}", agent.public_id);
    assert_eq!(agent.name, "test-agent");
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
        .get(format!("{}/v1/agents/{}", API_BASE_URL, agent.public_id))
        .send()
        .await
        .expect("Failed to get agent");

    assert_eq!(get_response.status(), 200);
    let fetched_agent: Agent = get_response.json().await.expect("Failed to parse agent");
    println!("Fetched agent: {}", fetched_agent.name);
    assert_eq!(fetched_agent.public_id, agent.public_id);

    // Step 4: Update agent
    println!("\nStep 4: Updating agent...");
    let update_response = client
        .patch(format!("{}/v1/agents/{}", API_BASE_URL, agent.public_id))
        .json(&json!({
            "name": "updated-test-agent",
            "display_name": "Updated Test Agent",
            "description": "Updated description"
        }))
        .send()
        .await
        .expect("Failed to update agent");

    assert_eq!(update_response.status(), 200);
    let updated_agent: Agent = update_response.json().await.expect("Failed to parse agent");
    println!("Updated agent: {}", updated_agent.name);
    assert_eq!(updated_agent.name, "updated-test-agent");

    // Step 5: Create a session
    println!("\nStep 5: Creating session...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({
            "harness_name": SEED_HARNESS_NAME,
            "agent_id": agent.public_id,
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
    assert_eq!(session.agent_id, Some(agent.public_id));

    // Step 6: Add message (user message)
    println!("\nStep 6: Adding user message...");
    let message_response = client
        .post(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
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
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
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
        .get(format!("{}/v1/sessions/{}", API_BASE_URL, session.id))
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
            "{}/v1/sessions/{}/events",
            API_BASE_URL, session.id
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

    // Cleanup
    //
    // This test used to create an agent and a session and delete neither, so a
    // second run against the same database failed on a 409 for the `test-agent`
    // name it had already taken (EVE-955).
    println!("\nCleaning up...");
    cleanup_delete(
        &client,
        format!("{}/v1/sessions/{}", API_BASE_URL, session.id),
        "session",
    )
    .await;
    cleanup_agent(&client, &agent.public_id).await;

    println!("\nAll tests passed!");
}

#[tokio::test]
async fn test_session_inherits_agent_default_model() {
    let client = reqwest::Client::new();

    println!("Testing session model_id inheritance from agent...");

    // Step 1: Create an LLM provider
    println!("\nStep 1: Creating LLM provider...");
    let provider_response = client
        .post(format!("{}/v1/providers", API_BASE_URL))
        .json(&json!({
            "name": "Test Provider for Session Model",
            "provider_type": "openai",
            "enabled": false
        }))
        .send()
        .await
        .expect("Failed to create provider");

    let provider: Provider = provider_response
        .json()
        .await
        .expect("Failed to parse provider");
    println!("Created provider: {}", provider.id);

    // Step 2: Create a model
    println!("\nStep 2: Creating model...");
    let model_response = client
        .post(format!(
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": "test-model",
            "display_name": "Test Model",
            "enabled": true
        }))
        .send()
        .await
        .expect("Failed to create model");

    let model: Model = model_response.json().await.expect("Failed to parse model");
    println!("Created model: {}", model.id);

    // Step 3: Create an agent with default_model_id
    println!("\nStep 3: Creating agent with default_model_id...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "agent-with-default-model",
            "display_name": "Agent with Default Model",
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
        agent.public_id, agent.default_model_id
    );
    assert_eq!(agent.default_model_id, Some(model.id));

    // Step 4: Create a session WITHOUT specifying model_id
    println!("\nStep 4: Creating session without model_id...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({
            "harness_name": SEED_HARNESS_NAME,
            "agent_id": agent.public_id,
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
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": "test-model-2",
            "display_name": "Test Model 2",
            "enabled": true
        }))
        .send()
        .await
        .expect("Failed to create second model");

    let model2: Model = model2_response
        .json()
        .await
        .expect("Failed to parse second model");

    let session2_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({
            "harness_name": SEED_HARNESS_NAME,
            "agent_id": agent.public_id,
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
    //
    // Order matters: `sessions.model_id` and the agent's `default_model_id`
    // both reference these models, so the sessions and the agent have to go
    // first or the model deletes are refused (EVE-955).
    println!("\nCleaning up...");
    cleanup_delete(
        &client,
        format!("{}/v1/sessions/{}", API_BASE_URL, session.id),
        "session",
    )
    .await;
    cleanup_delete(
        &client,
        format!("{}/v1/sessions/{}", API_BASE_URL, session2.id),
        "session2",
    )
    .await;
    cleanup_agent(&client, &agent.public_id).await;
    cleanup_delete(
        &client,
        format!("{}/v1/models/{}", API_BASE_URL, model.id),
        "model",
    )
    .await;
    cleanup_delete(
        &client,
        format!("{}/v1/models/{}", API_BASE_URL, model2.id),
        "model",
    )
    .await;
    cleanup_delete(
        &client,
        format!("{}/v1/providers/{}", API_BASE_URL, provider.id),
        "provider",
    )
    .await;

    println!("Session model_id inheritance test passed!");
}

#[tokio::test]
async fn test_sessions_pagination() {
    let client = reqwest::Client::new();

    println!("Testing sessions pagination...");

    // Create an agent for the test
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "pagination-test-agent",
            "display_name": "Pagination Test Agent",
            "system_prompt": "Test agent"
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(agent_response.status(), 201);
    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!("Created agent: {}", agent.public_id);

    // Create 15 sessions
    println!("Creating 15 sessions...");
    let mut session_ids = Vec::with_capacity(15);
    for i in 1..=15 {
        let response = client
            .post(format!("{}/v1/sessions", API_BASE_URL))
            .json(&json!({ "harness_name": SEED_HARNESS_NAME, "agent_id": agent.public_id, "title": format!("Session {}", i) }))
            .send()
            .await
            .expect("Failed to create session");
        assert_eq!(response.status(), 201, "Failed to create session {}", i);
        let session: Session = response.json().await.expect("Failed to parse session");
        session_ids.push(session.id);
    }
    println!("Created 15 sessions");

    // Test 1: Default pagination returns all with metadata
    println!("\nTest 1: Default pagination...");
    let response = client
        .get(format!(
            "{}/v1/sessions?agent_id={}",
            API_BASE_URL, agent.public_id
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
            "{}/v1/sessions?agent_id={}&limit=5",
            API_BASE_URL, agent.public_id
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
            "{}/v1/sessions?agent_id={}&offset=5&limit=5",
            API_BASE_URL, agent.public_id
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
            "{}/v1/sessions?agent_id={}&offset=10&limit=10",
            API_BASE_URL, agent.public_id
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
            "{}/v1/sessions?agent_id={}&offset=20",
            API_BASE_URL, agent.public_id
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
            "{}/v1/sessions?agent_id={}&limit=200",
            API_BASE_URL, agent.public_id
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
    for session_id in session_ids {
        cleanup_delete(
            &client,
            format!("{}/v1/sessions/{}", API_BASE_URL, session_id),
            "session",
        )
        .await;
    }
    cleanup_agent(&client, &agent.public_id).await;

    println!("Sessions pagination test completed!");
}
