use crate::support::*;
use everruns_platform::Agent;
use everruns_platform::Session;
use everruns_provider::model::Model;
use everruns_provider::provider::Provider;
use serde_json::{Value, json};

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
        .post(format!("{}/v1/providers", API_BASE_URL))
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
    let provider: Provider = provider_response
        .json()
        .await
        .expect("Failed to parse provider");
    println!("Created LlmSim provider: {}", provider.id);

    let model_response = client
        .post(format!(
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": "llmsim-test",
            "display_name": "LlmSim Test Model",
            "enabled": true
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
    let model: Model = model_response.json().await.expect("Failed to parse model");
    println!("Created LlmSim model: {}", model.id);

    // Step 1: Create agent with LlmSim model
    println!("\nStep 1: Creating agent with LlmSim model...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "workflow-test-agent",
            "display_name": "Workflow Test Agent",
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
        agent.public_id, agent.default_model_id
    );

    // Step 2: Create session
    println!("\nStep 2: Creating session...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({"harness_name": SEED_HARNESS_NAME, "agent_id": agent.public_id, "title": "Workflow Test Session"}))
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
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
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
                "{}/v1/sessions/{}/messages",
                API_BASE_URL, session.id
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
                "{}/v1/sessions/{}/events",
                API_BASE_URL, session.id
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
        .expect("Expected events array");
    println!("Found {} events", events.len());
    assert!(
        events.len() >= 2,
        "Expected at least 2 events (user message + agent response)"
    );

    // Cleanup
    println!("\nCleaning up...");
    // `sessions.model_id` references the model below, so the session has to go
    // first or the model delete is refused (EVE-955).
    cleanup_delete(
        &client,
        format!("{}/v1/sessions/{}", API_BASE_URL, session.id),
        "session",
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
        format!("{}/v1/providers/{}", API_BASE_URL, provider.id),
        "provider",
    )
    .await;

    println!("Message triggers agent workflow test passed!");
}

// ============================================================================
// POST /v1/sessions/:id/messages?wait=true (workflow test)
// ============================================================================

/// Tests POST /messages?wait=true blocks until the worker completes the turn (workflow test):
/// with server+worker running, the request must return 200 with a completed status and the
/// assistant output produced by the LlmSim scripted model.
#[tokio::test]
async fn test_post_message_wait_returns_completed_turn() {
    use std::time::Duration;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(70))
        .build()
        .expect("Failed to create client");

    // Step 1: Register LlmSim provider + model and create agent.
    let provider_name = "wait-test-llmsim";
    let provider_response = client
        .post(format!("{}/v1/providers", API_BASE_URL))
        .json(&json!({
            "name": provider_name,
            "provider_type": "llmsim",
            "config": {"base_url": "http://localhost:1", "api_key": "test-key"}
        }))
        .send()
        .await
        .expect("Failed to create LlmSim provider");
    assert_eq!(provider_response.status(), 201);
    let provider: Provider = provider_response
        .json()
        .await
        .expect("Failed to parse provider");

    let model_id = "llmsim-wait-test";
    let model_create = client
        .post(format!(
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": model_id,
            "display_name": "LlmSim Wait Test Model"
        }))
        .send()
        .await
        .expect("Failed to create LlmSim model");
    assert_eq!(model_create.status(), 201);
    // Keep the platform model id: `model_id` above is the provider-side model
    // string, which the delete endpoint does not accept.
    let model: Model = model_create
        .json()
        .await
        .expect("Failed to parse LlmSim model");

    let agent_name = "wait-test-agent";
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "provider_id": provider.id,
            "model_id": model_id,
            "name": agent_name,
            "system_prompt": "You are a helpful test assistant."
        }))
        .send()
        .await
        .expect("Failed to create agent");
    assert_eq!(agent_response.status(), 201);
    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");

    // Step 2: Create a session for the agent.
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({
            "harness_name": SEED_HARNESS_NAME,
            "agent_id": agent.public_id,
            "title": "Wait test"
        }))
        .send()
        .await
        .expect("Failed to create session");
    assert_eq!(session_response.status(), 201);
    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");

    // Step 3: POST with wait=true; the worker completes the turn, expect 200 completed.
    let message_response = client
        .post(format!(
            "{}/v1/sessions/{}/messages?wait=true&timeout_ms=60000",
            API_BASE_URL, session.id
        ))
        .json(&json!({
            "message": {
                "role": "user",
                "content": [{ "type": "text", "text": "Reply to this workflow test message." }]
            }
        }))
        .send()
        .await
        .expect("Failed to post message");
    assert_eq!(message_response.status(), 200);
    let body: Value = message_response
        .json()
        .await
        .expect("Failed to parse wait response");
    // A turn the provider *account* blocked (no credits, usage limit) says
    // nothing about whether `wait=true` returns a completed turn, so it skips
    // and reports — the rule the other live tests in this file already follow
    // via `skip_on_provider_account_block!`.
    //
    // Not that macro, though: it returns early, and this test must still tear
    // down the provider, model, agent and session it created or the next run
    // fails on a 409 for a name it already took (EVE-955). So the block is
    // checked inline and cleanup below runs either way.
    let account_block = session_provider_account_block(&client, &session.id).await;
    if let Some(ref code) = account_block {
        report_provider_account_skip(code);
    } else {
        assert_eq!(body["status"], "completed");
        let messages = body["messages"].as_array().expect("messages array");
        assert!(!messages.is_empty(), "waited turn returns assistant output");
        println!("✓ POST /messages?wait=true returns completed turn");
    }

    // Cleanup: this test created a provider, a model, an agent and a session,
    // and cleaned up none of them — so a second run against the same database
    // failed on a 409 for the agent name it had already taken (EVE-955).
    cleanup_delete(
        &client,
        format!("{}/v1/sessions/{}", API_BASE_URL, session.id),
        "session",
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
        format!("{}/v1/providers/{}", API_BASE_URL, provider.id),
        "provider",
    )
    .await;
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
        .post(format!("{}/v1/providers", API_BASE_URL))
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
    let provider: Provider = provider_response
        .json()
        .await
        .expect("Failed to parse provider");
    println!("Created LlmSim provider: {}", provider.id);

    let model_response = client
        .post(format!(
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": "llmsim-second-msg-test",
            "display_name": "LlmSim Second Message Test Model",
            "enabled": true
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
    let model: Model = model_response.json().await.expect("Failed to parse model");
    println!("Created LlmSim model: {}", model.id);

    // Step 1: Create agent with LlmSim model
    println!("\nStep 1: Creating agent with LlmSim model...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "second-message-test-agent",
            "display_name": "Second Message Test Agent",
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
        agent.public_id, agent.default_model_id
    );

    // Step 2: Create session
    println!("\nStep 2: Creating session...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({"harness_name": SEED_HARNESS_NAME, "agent_id": agent.public_id, "title": "Second Message Test Session"}))
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
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
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
                "{}/v1/sessions/{}/messages",
                API_BASE_URL, session.id
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
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
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
                "{}/v1/sessions/{}/messages",
                API_BASE_URL, session.id
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
                "{}/v1/sessions/{}/messages",
                API_BASE_URL, session.id
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
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
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
    // `sessions.model_id` references the model below, so the session has to go
    // first or the model delete is refused (EVE-955).
    cleanup_delete(
        &client,
        format!("{}/v1/sessions/{}", API_BASE_URL, session.id),
        "session",
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
        format!("{}/v1/providers/{}", API_BASE_URL, provider.id),
        "provider",
    )
    .await;

    println!("Second message workflow test passed!");
}
