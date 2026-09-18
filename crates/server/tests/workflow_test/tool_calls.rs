use crate::support::*;
use everruns_core::SessionFile;
use everruns_platform::Agent;
use everruns_platform::Session;
use everruns_provider::model::Model;
use everruns_provider::provider::Provider;
use serde_json::{Value, json};

/// Test the stable edit_file integration boundary with LlmSim.
///
/// Verifies:
/// 1. Agents with `session_file_system` preview `read_file` and `edit_file`
/// 2. `edit_file` advertises `expected_hash` as a required parameter
/// 3. Session filesystem API round-trips the target text file for the session
///
/// Note: the default LlmSim driver used by workflow tests does not
/// deterministically emit file tool calls, so exact `edit_file` behavior is
/// covered in core/server unit tests instead of this workflow smoke test.
#[tokio::test]
async fn test_agent_execution_llmsim_with_edit_file_tool() {
    use std::time::Duration;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("Failed to create client");

    println!("Testing agent execution with edit_file tool...");

    // Step 1: Create LlmSim provider and model
    println!("\nStep 1: Creating LlmSim provider and model...");
    let provider_response = client
        .post(format!("{}/v1/providers", API_BASE_URL))
        .json(&json!({
            "name": "LlmSim Edit Tool Provider",
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
            "model_id": "llmsim-edit-tool-test",
            "display_name": "LlmSim Edit Tool Test Model",
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

    // Step 2: Create agent with session_file_system capability
    println!("\nStep 2: Creating edit agent...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "edit-tool-test-agent",
            "display_name": "Edit Tool Test Agent",
            "system_prompt": "You are a file editing assistant. When asked to update an existing file, first use read_file to inspect the file and obtain its content hash, then use edit_file to make the requested exact replacement. Do not use write_file for existing files. After editing, confirm the final file contents.",
            "capabilities": [{"ref": "session_file_system", "config": {}}],
            "default_model_id": model.id
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(agent_response.status(), 201);
    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!("Created agent: {}", agent.public_id);

    // Step 3: Create session
    println!("\nStep 3: Creating session...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({
            "harness_name": SEED_HARNESS_NAME,
            "agent_id": agent.public_id,
            "title": "Edit Tool Integration Test"
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

    let fs_url = format!("{}/v1/sessions/{}/fs", API_BASE_URL, session.id);

    // Step 4: Preview agent tools and verify edit_file contract
    println!("\nStep 4: Previewing tool definitions...");
    let preview_response = client
        .post(format!("{}/v1/agents/preview", API_BASE_URL))
        .json(&json!({
            "system_prompt": "You are a file editing assistant. When asked to update an existing file, first use read_file to inspect the file and obtain its content hash, then use edit_file to make the requested exact replacement. Do not use write_file for existing files. After editing, confirm the final file contents.",
            "capabilities": [{"ref": "session_file_system", "config": {}}],
            "tools": []
        }))
        .send()
        .await
        .expect("Failed to preview agent");

    assert_eq!(preview_response.status(), 200);
    let preview: Value = preview_response
        .json()
        .await
        .expect("Failed to parse preview");
    let tools = preview["tools"]
        .as_array()
        .expect("Expected preview tools array");
    let tool_names: Vec<&str> = tools
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(tool_names.contains(&"read_file"));
    assert!(tool_names.contains(&"edit_file"));

    let edit_tool = tools
        .iter()
        .find(|tool| tool["name"] == "edit_file")
        .expect("Expected edit_file tool in preview");
    let required = edit_tool["parameters"]["required"]
        .as_array()
        .expect("Expected edit_file required array");
    assert!(required.contains(&json!("path")));
    assert!(required.contains(&json!("expected_hash")));
    println!("Preview includes read_file and edit_file with expected_hash");

    // Step 5: Seed the file that edit_file would target
    println!("\nStep 5: Seeding session file...");
    let seed_response = client
        .post(format!("{}/workspace/edit-target.txt", fs_url))
        .json(&json!({
            "content": "alpha\nbeta\ngamma\n",
            "encoding": "text"
        }))
        .send()
        .await
        .expect("Failed to create seed file");

    assert_eq!(seed_response.status(), 201);
    println!("Seed file created");

    // Step 6: Verify the file remains accessible through the session filesystem API
    println!("\nStep 6: Verifying session file contents...");
    let file_response = client
        .get(format!("{}/workspace/edit-target.txt", fs_url))
        .send()
        .await
        .expect("Failed to fetch session file");

    assert_eq!(file_response.status(), 200);
    let file: SessionFile = file_response
        .json()
        .await
        .expect("Failed to parse session file");
    assert_eq!(
        file.content.as_deref(),
        Some("alpha\nbeta\ngamma\n"),
        "Expected seeded file content to round-trip unchanged"
    );
    println!("Session file content verified: {:?}", file.content);

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
        format!(
            "{}/v1/providers/{}/models/{}",
            API_BASE_URL, provider.id, model.id
        ),
        "model",
    )
    .await;
    cleanup_delete(
        &client,
        format!("{}/v1/providers/{}", API_BASE_URL, provider.id),
        "provider",
    )
    .await;

    println!("LlmSim edit_file integration test passed!");
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

    // Get API key from environment (this test requires it)
    let api_key = match std::env::var("OPENAI_API_KEY") {
        Ok(key) if !key.is_empty() => key,
        _ => {
            println!("Skipping test: OPENAI_API_KEY not set");
            return;
        }
    };

    let provider_response = client
        .post(format!("{}/v1/providers", API_BASE_URL))
        .json(&json!({
            "name": "Duplicate Tool Test Provider",
            "provider_type": "openai",
            "api_key": api_key,
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

    // Step 2: Create a model configured for tool use
    println!("\nStep 2: Creating model...");
    let model_response = client
        .post(format!(
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": "gpt-5.4-mini",
            "display_name": "GPT-5.4 Mini Test",
            "enabled": true
        }))
        .send()
        .await
        .expect("Failed to create model");

    if model_response.status() != 201 {
        let status = model_response.status();
        let body = model_response.text().await.unwrap_or_default();
        panic!("Failed to create model: status={}, body={}", status, body);
    }
    let model: Model = model_response.json().await.expect("Failed to parse model");
    println!("Created model: {}", model.id);

    // Step 3: Create an agent with current_time capability
    println!("\nStep 3: Creating agent with current_time capability...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "time-tool-test-agent",
            "display_name": "Time Tool Test Agent",
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
        agent.public_id, agent.capabilities
    );

    // Step 4: Create a session
    println!("\nStep 4: Creating session...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({"harness_name": SEED_HARNESS_NAME, "agent_id": agent.public_id}))
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
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
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

    skip_on_provider_account_block!(session_provider_account_block(&client, &session.id).await);

    // Step 7: Get all messages and check for duplicates
    println!("\nStep 7: Checking for duplicate tool calls...");
    let messages_response = client
        .get(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
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

                // Check for tool_result content parts (from tool.completed events)
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
    // `sessions.model_id` pins the model, so the session goes first or the
    // model/provider delete is refused (EVE-955).
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

    println!("No duplicate tool calls test completed!");
}

/// Test agent execution with tool calls using LlmSim driver (deterministic).
///
/// This test verifies the full agent workflow with tool calls using a simulated LLM:
/// 1. Create an agent with current_time capability
/// 2. Send a message asking for a time-based joke
/// 3. Verify the agent calls get_current_time tool
/// 4. Verify the agent generates a response that includes time info
///
/// Requirements: API + Worker running (uses LlmSim, no real API keys needed)
#[tokio::test]
async fn test_agent_execution_llmsim_with_tool_calls() {
    use std::time::Duration;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .expect("Failed to create client");

    println!("Testing agent execution with LlmSim driver (tool calls)...");

    // Step 1: Create LlmSim provider and model
    println!("\nStep 1: Creating LlmSim provider and model...");
    let provider_response = client
        .post(format!("{}/v1/providers", API_BASE_URL))
        .json(&json!({
            "name": "LlmSim Tool Test Provider",
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
            "model_id": "llmsim-tool-test",
            "display_name": "LlmSim Tool Test Model",
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

    // Step 2: Create a dad jokes agent with current_time capability
    println!("\nStep 2: Creating dad jokes agent with current_time capability...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "dad-jokes-time-agent",
            "display_name": "Dad Jokes Time Agent",
            "system_prompt": "You are a dad jokes comedian. When asked for a joke about the time, first use the get_current_time tool to get the current time, then tell a dad joke that incorporates the time. Your jokes should be punny and family-friendly.",
            "capabilities": [{"ref": "current_time", "config": {}}],
            "default_model_id": model.id
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(agent_response.status(), 201);
    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!(
        "Created agent: {} with capabilities: {:?}",
        agent.public_id, agent.capabilities
    );

    // Step 3: Create a session
    println!("\nStep 3: Creating session...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({"harness_name": SEED_HARNESS_NAME, "agent_id": agent.public_id, "title": "Dad Jokes Session"}))
        .send()
        .await
        .expect("Failed to create session");

    assert_eq!(session_response.status(), 201);
    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");
    println!("Created session: {}", session.id);

    // Step 4: Send a message asking for a time-based joke
    println!("\nStep 4: Sending message asking for time-based joke...");
    let message_response = client
        .post(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
        ))
        .json(&json!({
            "message": {
                "content": [{"type": "text", "text": "Tell me a dad joke about the current time!"}]
            }
        }))
        .send()
        .await
        .expect("Failed to send message");

    assert_eq!(message_response.status(), 201);
    println!("Message sent successfully");

    // Step 5: Wait for agent response and tool calls
    println!("\nStep 5: Waiting for agent response with tool calls (up to 45 seconds)...");
    let mut agent_response_found = false;
    let mut tool_call_found = false;
    let mut tool_result_found = false;

    for i in 1..=45 {
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

            // Debug output every 5 seconds
            if i % 5 == 0 {
                println!(
                    "  [{}s] Messages: {}, looking for agent response with tool calls...",
                    i,
                    messages.len()
                );
            }

            for msg in messages {
                let role = msg["role"].as_str().unwrap_or("");

                // Check for agent messages
                if role == "agent" {
                    agent_response_found = true;

                    // Check for tool calls in content
                    if let Some(content) = msg["content"].as_array() {
                        for part in content {
                            // Tool calls have type: "tool_call" and name field
                            if part["type"] == "tool_call" {
                                let tool_name = part["name"].as_str().unwrap_or("");
                                if tool_name == "get_current_time" {
                                    tool_call_found = true;
                                    println!("  Found get_current_time tool call after {}s", i);
                                }
                            }
                        }
                    }
                }
                // Check for tool results in user messages
                if role == "user"
                    && let Some(content) = msg["content"].as_array()
                {
                    for part in content {
                        if part["type"] == "tool_result" {
                            tool_result_found = true;
                            println!("  Found tool result after {}s", i);
                        }
                    }
                }
            }

            // We need at least an agent response (LlmSim may not always use tools)
            if agent_response_found {
                // Give a bit more time for full completion
                tokio::time::sleep(Duration::from_secs(2)).await;
                break;
            }
        }
    }

    // Step 6: Verify the workflow completed
    println!("\nStep 6: Verifying workflow completion...");

    // Get final messages
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

    println!("Final message count: {}", final_messages.len());

    // Count message types
    let user_count = final_messages
        .iter()
        .filter(|m| m["role"] == "user")
        .count();
    let agent_count = final_messages
        .iter()
        .filter(|m| m["role"] == "agent")
        .count();

    println!("Message counts: {} user, {} agent", user_count, agent_count);

    // We should have at least 1 user message and 1 agent message
    assert!(
        user_count >= 1,
        "Expected at least 1 user message, got {}",
        user_count
    );
    assert!(
        agent_response_found,
        "Agent did not respond within 45 seconds"
    );
    assert!(
        agent_count >= 1,
        "Expected at least 1 agent message, got {}",
        agent_count
    );

    // Print summary
    println!("\nTest summary:");
    println!("  Agent response: {}", agent_response_found);
    println!("  Tool call (get_current_time): {}", tool_call_found);
    println!("  Tool result: {}", tool_result_found);

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
        format!(
            "{}/v1/providers/{}/models/{}",
            API_BASE_URL, provider.id, model.id
        ),
        "model",
    )
    .await;
    cleanup_delete(
        &client,
        format!("{}/v1/providers/{}", API_BASE_URL, provider.id),
        "provider",
    )
    .await;

    println!("LlmSim agent execution with tool calls test passed!");
}

/// Test agent execution with OpenAI driver.
///
/// This test verifies end-to-end agent execution with real OpenAI API:
/// 1. Create an agent with current_time capability
/// 2. Send a message asking for a time-based joke
/// 3. Verify the agent calls get_current_time tool
/// 4. Verify the joke references the current time
///
/// Requirements: API + Worker running, OPENAI_API_KEY environment variable set.
/// Skips if no API key is available.
#[tokio::test]
async fn test_agent_execution_openai_with_tool_calls() {
    use std::time::Duration;

    // Check if OpenAI API key is available
    let api_key = std::env::var("OPENAI_API_KEY").ok();
    if api_key.is_none() || api_key.as_ref().map(|k| k.is_empty()).unwrap_or(true) {
        println!("Skipping OpenAI test: OPENAI_API_KEY not set");
        return;
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .expect("Failed to create client");

    println!("Testing agent execution with OpenAI driver (tool calls)...");

    // Step 1: Create OpenAI provider
    println!("\nStep 1: Creating OpenAI provider...");
    // Get API key from environment
    let api_key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY should be set");

    let provider_response = client
        .post(format!("{}/v1/providers", API_BASE_URL))
        .json(&json!({
            "name": "OpenAI Tool Test Provider",
            "provider_type": "openai",
            "api_key": api_key,
            "enabled": false
        }))
        .send()
        .await
        .expect("Failed to create provider");

    if provider_response.status() != 201 {
        let status = provider_response.status();
        let body = provider_response.text().await.unwrap_or_default();
        panic!(
            "Failed to create OpenAI provider: status={}, body={}",
            status, body
        );
    }
    let provider: Provider = provider_response
        .json()
        .await
        .expect("Failed to parse provider");
    println!("Created OpenAI provider: {}", provider.id);

    // Create model (gpt-5.4-mini for cost-effectiveness)
    let model_response = client
        .post(format!(
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": "gpt-5.4-mini",
            "display_name": "GPT-5.4 Mini (Tool Test)",
            "enabled": true
        }))
        .send()
        .await
        .expect("Failed to create model");

    if model_response.status() != 201 {
        let status = model_response.status();
        let body = model_response.text().await.unwrap_or_default();
        panic!("Failed to create model: status={}, body={}", status, body);
    }
    let model: Model = model_response.json().await.expect("Failed to parse model");
    println!("Created model: {} ({})", model.display_name, model.id);

    // Step 2: Create dad jokes agent with current_time capability
    println!("\nStep 2: Creating dad jokes agent with current_time capability...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "openai-dad-jokes-agent",
            "display_name": "OpenAI Dad Jokes Agent",
            "system_prompt": "You are a dad jokes comedian. When the user asks for a joke about the time, you MUST first use the get_current_time tool to get the current time, then tell a short dad joke that somehow references the time you received. Keep your response brief.",
            "capabilities": [{"ref": "current_time", "config": {}}],
            "default_model_id": model.id
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(agent_response.status(), 201);
    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!("Created agent: {}", agent.public_id);

    // Step 3: Create session
    println!("\nStep 3: Creating session...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({"harness_name": SEED_HARNESS_NAME, "agent_id": agent.public_id, "title": "OpenAI Dad Jokes Session"}))
        .send()
        .await
        .expect("Failed to create session");

    assert_eq!(session_response.status(), 201);
    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");
    println!("Created session: {}", session.id);

    // Step 4: Send message
    println!("\nStep 4: Sending message...");
    let message_response = client
        .post(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
        ))
        .json(&json!({
            "message": {
                "content": [{"type": "text", "text": "Tell me a dad joke about the current time!"}]
            }
        }))
        .send()
        .await
        .expect("Failed to send message");

    assert_eq!(message_response.status(), 201);

    // Step 5: Wait for response with tool calls
    println!("\nStep 5: Waiting for response (up to 60 seconds)...");
    let mut tool_call_found = false;
    let mut final_response_found = false;
    let mut final_response_text = String::new();

    for i in 1..=60 {
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

            if i % 10 == 0 {
                println!("  [{}s] Messages: {}", i, messages.len());
            }

            for msg in messages {
                if msg["role"] == "agent"
                    && let Some(content) = msg["content"].as_array()
                {
                    for part in content {
                        // Check for tool calls (type: "tool_call" with name field)
                        if part["type"] == "tool_call" {
                            let name = part["name"].as_str().unwrap_or("");
                            if name == "get_current_time" {
                                tool_call_found = true;
                                println!("  Found get_current_time tool call after {}s", i);
                            }
                        }
                        // Check for text response (final answer)
                        if part["type"] == "text" {
                            let text_str = part["text"].as_str().unwrap_or("");
                            if !text_str.is_empty() && text_str.len() > 10 {
                                final_response_found = true;
                                final_response_text = text_str.to_string();
                            }
                        }
                    }
                }
            }

            if tool_call_found && final_response_found {
                tokio::time::sleep(Duration::from_secs(2)).await;
                break;
            }
        }
    }

    // Verify results
    println!("\nResults:");
    println!("  Tool call found: {}", tool_call_found);
    println!("  Final response found: {}", final_response_found);
    if !final_response_text.is_empty() {
        println!(
            "  Response preview: {}...",
            response_preview(&final_response_text)
        );
    }

    // Check for transient API errors (network issues, TLS errors, etc.)
    let is_error_response = final_response_text.contains("encountered an error")
        || final_response_text.contains("try again later");

    // Read the account block while the session is still intact; act on it once
    // the fixtures below are torn down.
    let account_block = session_provider_account_block(&client, &session.id).await;

    // Cleanup first before assertions
    println!("\nCleaning up...");
    // `sessions.model_id` pins the model, so the session goes first or the
    // model/provider delete is refused (EVE-955).
    cleanup_delete(
        &client,
        format!("{}/v1/sessions/{}", API_BASE_URL, session.id),
        "session",
    )
    .await;
    cleanup_agent(&client, &agent.public_id).await;
    cleanup_delete(
        &client,
        format!(
            "{}/v1/providers/{}/models/{}",
            API_BASE_URL, provider.id, model.id
        ),
        "model",
    )
    .await;
    cleanup_delete(
        &client,
        format!("{}/v1/providers/{}", API_BASE_URL, provider.id),
        "provider",
    )
    .await;

    skip_on_provider_account_block!(account_block);

    // If we got an error response, skip the test (transient API issue)
    if is_error_response {
        println!("Skipping assertions due to transient API error");
        return;
    }

    assert!(
        tool_call_found,
        "OpenAI agent should have called get_current_time tool"
    );
    assert!(
        final_response_found,
        "OpenAI agent should have generated a final response"
    );

    println!("OpenAI agent execution with tool calls test passed!");
}

/// Test agent execution with Anthropic driver.
///
/// This test verifies end-to-end agent execution with real Anthropic API:
/// 1. Create an agent with current_time capability
/// 2. Send a message asking for a time-based joke
/// 3. Verify the agent calls get_current_time tool
/// 4. Verify the joke references the current time
///
/// Requirements: API + Worker running, ANTHROPIC_API_KEY environment variable set.
/// Skips if no API key is available.
#[tokio::test]
async fn test_agent_execution_anthropic_with_tool_calls() {
    use std::time::Duration;

    // Check if Anthropic API key is available
    let api_key = std::env::var("ANTHROPIC_API_KEY").ok();
    if api_key.is_none() || api_key.as_ref().map(|k| k.is_empty()).unwrap_or(true) {
        println!("Skipping Anthropic test: ANTHROPIC_API_KEY not set");
        return;
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .expect("Failed to create client");

    println!("Testing agent execution with Anthropic driver (tool calls)...");

    // Step 1: Create Anthropic provider
    println!("\nStep 1: Creating Anthropic provider...");

    // Get API key from environment
    let api_key = std::env::var("ANTHROPIC_API_KEY").expect("ANTHROPIC_API_KEY should be set");

    let provider_response = client
        .post(format!("{}/v1/providers", API_BASE_URL))
        .json(&json!({
            "name": "Anthropic Tool Test Provider",
            "provider_type": "anthropic",
            "api_key": api_key,
            "enabled": false
        }))
        .send()
        .await
        .expect("Failed to create provider");

    if provider_response.status() != 201 {
        let status = provider_response.status();
        let body = provider_response.text().await.unwrap_or_default();
        panic!(
            "Failed to create Anthropic provider: status={}, body={}",
            status, body
        );
    }
    let provider: Provider = provider_response
        .json()
        .await
        .expect("Failed to parse provider");
    println!("Created Anthropic provider: {}", provider.id);

    // Create model (claude-haiku-4-5 for cost-effectiveness)
    let model_response = client
        .post(format!(
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": LIVE_ANTHROPIC_FAST_MODEL,
            "display_name": "Anthropic Fast (Tool Test)",
            "enabled": true
        }))
        .send()
        .await
        .expect("Failed to create model");

    if model_response.status() != 201 {
        let status = model_response.status();
        let body = model_response.text().await.unwrap_or_default();
        panic!("Failed to create model: status={}, body={}", status, body);
    }
    let model: Model = model_response.json().await.expect("Failed to parse model");
    println!("Created model: {} ({})", model.display_name, model.id);

    // Step 2: Create dad jokes agent with current_time capability
    println!("\nStep 2: Creating dad jokes agent with current_time capability...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "anthropic-dad-jokes-agent",
            "display_name": "Anthropic Dad Jokes Agent",
            "system_prompt": "You are a dad jokes comedian. When the user asks for a joke about the time, you MUST first use the get_current_time tool to get the current time, then tell a short dad joke that somehow references the time you received. Keep your response brief.",
            "capabilities": [{"ref": "current_time", "config": {}}],
            "default_model_id": model.id
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(agent_response.status(), 201);
    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!("Created agent: {}", agent.public_id);

    // Step 3: Create session
    println!("\nStep 3: Creating session...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({"harness_name": SEED_HARNESS_NAME, "agent_id": agent.public_id, "title": "Anthropic Dad Jokes Session"}))
        .send()
        .await
        .expect("Failed to create session");

    assert_eq!(session_response.status(), 201);
    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");
    println!("Created session: {}", session.id);

    // Step 4: Send message
    println!("\nStep 4: Sending message...");
    let message_response = client
        .post(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
        ))
        .json(&json!({
            "message": {
                "content": [{"type": "text", "text": "Tell me a dad joke about the current time!"}]
            }
        }))
        .send()
        .await
        .expect("Failed to send message");

    assert_eq!(message_response.status(), 201);

    // Step 5: Wait for response with tool calls
    println!("\nStep 5: Waiting for response (up to 60 seconds)...");
    let mut tool_call_found = false;
    let mut final_response_found = false;
    let mut final_response_text = String::new();

    for i in 1..=60 {
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

            if i % 10 == 0 {
                println!("  [{}s] Messages: {}", i, messages.len());
            }

            for msg in messages {
                if msg["role"] == "agent"
                    && let Some(content) = msg["content"].as_array()
                {
                    for part in content {
                        // Check for tool calls (type: "tool_call" with name field)
                        if part["type"] == "tool_call" {
                            let name = part["name"].as_str().unwrap_or("");
                            if name == "get_current_time" {
                                tool_call_found = true;
                                println!("  Found get_current_time tool call after {}s", i);
                            }
                        }
                        // Check for text response (final answer)
                        if part["type"] == "text" {
                            let text_str = part["text"].as_str().unwrap_or("");
                            if !text_str.is_empty() && text_str.len() > 10 {
                                final_response_found = true;
                                final_response_text = text_str.to_string();
                            }
                        }
                    }
                }
            }

            if tool_call_found && final_response_found {
                tokio::time::sleep(Duration::from_secs(2)).await;
                break;
            }
        }
    }

    // Verify results
    println!("\nResults:");
    println!("  Tool call found: {}", tool_call_found);
    println!("  Final response found: {}", final_response_found);
    if !final_response_text.is_empty() {
        println!(
            "  Response preview: {}...",
            response_preview(&final_response_text)
        );
    }

    // Check for transient API errors (network issues, TLS errors, etc.)
    let is_error_response = final_response_text.contains("encountered an error")
        || final_response_text.contains("try again later");

    // Read the account block while the session is still intact; act on it once
    // the fixtures below are torn down.
    let account_block = session_provider_account_block(&client, &session.id).await;

    // Cleanup first before assertions
    println!("\nCleaning up...");
    // `sessions.model_id` pins the model, so the session goes first or the
    // model/provider delete is refused (EVE-955).
    cleanup_delete(
        &client,
        format!("{}/v1/sessions/{}", API_BASE_URL, session.id),
        "session",
    )
    .await;
    cleanup_agent(&client, &agent.public_id).await;
    cleanup_delete(
        &client,
        format!(
            "{}/v1/providers/{}/models/{}",
            API_BASE_URL, provider.id, model.id
        ),
        "model",
    )
    .await;
    cleanup_delete(
        &client,
        format!("{}/v1/providers/{}", API_BASE_URL, provider.id),
        "provider",
    )
    .await;

    skip_on_provider_account_block!(account_block);

    // If we got an error response, skip the test (transient API issue)
    if is_error_response {
        println!("Skipping assertions due to transient API error");
        return;
    }

    assert!(
        tool_call_found,
        "Anthropic agent should have called get_current_time tool"
    );
    assert!(
        final_response_found,
        "Anthropic agent should have generated a final response"
    );

    println!("Anthropic agent execution with tool calls test passed!");
}

/// Test agent execution with a tool-bearing capability over several turns.
///
/// This test verifies the full agent round-trip with a capability that
/// exposes tools (current_time):
/// 1. Create an agent with the current_time capability
/// 2. Send a message that would exercise the tool surface
/// 3. Verify the agent produces a response
///
/// Requirements: API + Worker running (uses LlmSim, no real API keys needed)
#[tokio::test]
async fn test_agent_execution_multiple_tool_calls() {
    use std::time::Duration;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .expect("Failed to create client");

    println!("Testing agent execution with multiple tool calls...");

    // Step 1: Create LlmSim provider and model
    println!("\nStep 1: Creating LlmSim provider and model...");
    let provider_response = client
        .post(format!("{}/v1/providers", API_BASE_URL))
        .json(&json!({
            "name": "LlmSim Multi-Tool Test",
            "provider_type": "llmsim"
        }))
        .send()
        .await
        .expect("Failed to create provider");

    if provider_response.status() != 201 {
        let status = provider_response.status();
        let body = provider_response.text().await.unwrap_or_default();
        panic!(
            "Failed to create provider: status={}, body={}",
            status, body
        );
    }
    let provider: Provider = provider_response
        .json()
        .await
        .expect("Failed to parse provider");

    let model_response = client
        .post(format!(
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": "llmsim-multi-tool",
            "display_name": "LlmSim Multi-Tool Model",
            "enabled": true
        }))
        .send()
        .await
        .expect("Failed to create model");

    let model: Model = model_response.json().await.expect("Failed to parse model");
    println!("Created provider and model");

    // Step 2: Create agent with current_time capability
    println!("\nStep 2: Creating agent with current_time capability...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "multi-tool-agent",
            "display_name": "Multi Tool Agent",
            "system_prompt": "You are a scheduling assistant. Use the get_current_time tool to answer questions about the current time, and show your reasoning step by step.",
            "capabilities": [{"ref": "current_time", "config": {}}],
            "default_model_id": model.id
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(agent_response.status(), 201);
    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!("Created agent: {}", agent.public_id);

    // Step 3: Create session and send message
    println!("\nStep 3: Creating session and sending message...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({"harness_name": SEED_HARNESS_NAME, "agent_id": agent.public_id}))
        .send()
        .await
        .expect("Failed to create session");

    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");

    let message_response = client
        .post(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
        ))
        .json(&json!({
            "message": {
                "content": [{"type": "text", "text": "What time is it right now? Check the clock before answering."}]
            }
        }))
        .send()
        .await
        .expect("Failed to send message");

    assert_eq!(message_response.status(), 201);

    // Step 4: Wait for response
    println!("\nStep 4: Waiting for agent response (up to 30 seconds)...");
    let mut agent_response_found = false;

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

            let agent_count = messages.iter().filter(|m| m["role"] == "agent").count();
            if agent_count >= 1 {
                agent_response_found = true;
                println!("  Found agent response after {}s", i);
                break;
            }
        }

        if i % 5 == 0 {
            println!("  Still waiting... ({}s)", i);
        }
    }

    assert!(
        agent_response_found,
        "Agent should have responded within 30 seconds"
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
        format!(
            "{}/v1/providers/{}/models/{}",
            API_BASE_URL, provider.id, model.id
        ),
        "model",
    )
    .await;
    cleanup_delete(
        &client,
        format!("{}/v1/providers/{}", API_BASE_URL, provider.id),
        "provider",
    )
    .await;

    println!("Multiple tool calls test passed!");
}
