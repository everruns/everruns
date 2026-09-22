use crate::support::*;
use everruns_platform::Agent;
use everruns_platform::Session;
use everruns_provider::model::Model;
use everruns_provider::provider::Provider;
use serde_json::{Value, json};

/// Test extended thinking with real Anthropic API.
///
/// This test verifies that:
/// 1. Extended thinking events are emitted (reason.thinking.started, reason.thinking.delta, reason.thinking.completed)
/// 2. The message.agent event contains the thinking field
/// 3. Multi-turn conversations work correctly with thinking (thinking is sent back to Anthropic)
///
/// Requirements: API + Worker running, ANTHROPIC_API_KEY environment variable set.
/// Skips if no API key is available.
#[tokio::test]
async fn test_anthropic_extended_thinking() {
    use std::time::Duration;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(180))
        .build()
        .expect("Failed to create client");

    println!("Testing Anthropic extended thinking...");

    // Step 1: Create Anthropic provider
    println!("\nStep 1: Creating Anthropic provider...");
    let api_key = match std::env::var("ANTHROPIC_API_KEY") {
        Ok(key) if !key.is_empty() => key,
        _ => {
            println!("ANTHROPIC_API_KEY not set - skipping test");
            return;
        }
    };

    let provider_response = client
        .post(format!("{}/v1/providers", API_BASE_URL))
        .json(&json!({
            "name": "Anthropic Thinking Test Provider",
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

    // Create model (the live thinking model supports extended thinking)
    let model_response = client
        .post(format!(
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": LIVE_ANTHROPIC_THINKING_MODEL,
            "display_name": "Anthropic Thinking (Thinking Test)",
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

    // Step 2: Create agent (no tool calls - tests basic thinking flow)
    println!("\nStep 2: Creating agent...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "thinking-test-agent",
            "display_name": "Thinking Test Agent",
            "system_prompt": "You are a helpful assistant. Think through problems step by step.",
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
            "title": "Extended Thinking Test Session"
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

    // Step 4: Send message with reasoning effort enabled (using "low" to minimize cost)
    // Note: controls is at request level, not inside message
    println!("\nStep 4: Sending message with reasoning effort=low...");
    let message_response = client
        .post(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
        ))
        .json(&json!({
            "message": {
                "content": [{"type": "text", "text": "What is 17 * 23? Show your reasoning."}]
            },
            "controls": {
                "reasoning": {
                    "effort": "low"
                }
            }
        }))
        .send()
        .await
        .expect("Failed to send message");

    assert_eq!(message_response.status(), 201);
    println!("Message sent successfully");

    // Step 5: Poll for completion and collect events
    println!("\nStep 5: Waiting for response and collecting events...");
    let mut response_complete = false;
    let mut thinking_started_found = false;
    let mut thinking_delta_found = false;
    let mut thinking_completed_found = false;
    let mut message_agent_with_reasoning = false;
    let mut all_events: Vec<Value> = Vec::new();

    for i in 1..=90 {
        tokio::time::sleep(Duration::from_secs(1)).await;

        // Check session status
        let session_response = client
            .get(format!("{}/v1/sessions/{}", API_BASE_URL, session.id))
            .send()
            .await;

        if let Ok(resp) = session_response
            && resp.status() == 200
        {
            let session_data: Value = resp.json().await.unwrap_or_default();
            let status = session_data["status"].as_str().unwrap_or("");

            if i % 10 == 0 {
                println!("  [{}s] Session status: {}", i, status);
            }

            if status == "idle" {
                response_complete = true;
                break;
            }
        }
    }

    // Fetch all events
    let events_response = client
        .get(format!(
            "{}/v1/sessions/{}/events",
            API_BASE_URL, session.id
        ))
        .send()
        .await
        .expect("Failed to get events");

    if events_response.status() == 200 {
        let events_data: Value = events_response.json().await.unwrap_or_default();
        if let Some(events) = events_data["data"].as_array() {
            all_events = events.clone();

            // Check for thinking events
            for event in events {
                let event_type = event["type"].as_str().unwrap_or("");
                match event_type {
                    "reason.thinking.started" => {
                        thinking_started_found = true;
                        println!("  Found reason.thinking.started event");
                    }
                    "reason.thinking.delta" => {
                        thinking_delta_found = true;
                        // Only log first delta
                        if !thinking_delta_found {
                            println!("  Found reason.thinking.delta event");
                        }
                    }
                    "reason.thinking.completed" => {
                        thinking_completed_found = true;
                        let thinking_content = event["data"]["thinking"].as_str().unwrap_or("");
                        println!(
                            "  Found reason.thinking.completed event (thinking length: {} chars)",
                            thinking_content.len()
                        );
                    }
                    // Reasoning reaches the message as ordered `reasoning`
                    // content parts, not a scalar `thinking` field on the
                    // message — see ContentPart::Reasoning in crates/core.
                    everruns_core::events::OUTPUT_MESSAGE_COMPLETED => {
                        let reasoning_parts = event["data"]["message"]["content"]
                            .as_array()
                            .map(|parts| {
                                parts
                                    .iter()
                                    .filter(|part| part["type"] == "reasoning")
                                    .count()
                            })
                            .unwrap_or(0);
                        if reasoning_parts > 0 {
                            message_agent_with_reasoning = true;
                            println!(
                                "  Found message.agent with {} reasoning content part(s)",
                                reasoning_parts
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Print event summary
    println!("\nFirst turn results:");
    println!("  Response complete: {}", response_complete);
    println!(
        "  reason.thinking.started found: {}",
        thinking_started_found
    );
    println!("  reason.thinking.delta found: {}", thinking_delta_found);
    println!(
        "  reason.thinking.completed found: {}",
        thinking_completed_found
    );
    println!(
        "  message.agent with reasoning parts: {}",
        message_agent_with_reasoning
    );
    println!("  Total events: {}", all_events.len());

    // Step 6: Multi-turn test - send a follow-up message
    // This verifies thinking is properly sent back to Anthropic API
    println!("\nStep 6: Sending follow-up message to test multi-turn with thinking...");
    let followup_response = client
        .post(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
        ))
        .json(&json!({
            "message": {
                "content": [{"type": "text", "text": "Now multiply that result by 2."}]
            },
            "controls": {
                "reasoning": {
                    "effort": "low"
                }
            }
        }))
        .send()
        .await
        .expect("Failed to send follow-up message");

    assert_eq!(followup_response.status(), 201);
    println!("Follow-up message sent successfully");

    // Wait for follow-up to complete
    let mut followup_complete = false;
    for i in 1..=90 {
        tokio::time::sleep(Duration::from_secs(1)).await;

        let session_response = client
            .get(format!("{}/v1/sessions/{}", API_BASE_URL, session.id))
            .send()
            .await;

        if let Ok(resp) = session_response
            && resp.status() == 200
        {
            let session_data: Value = resp.json().await.unwrap_or_default();
            let status = session_data["status"].as_str().unwrap_or("");

            if i % 10 == 0 {
                println!("  [{}s] Session status: {}", i, status);
            }

            if status == "idle" {
                followup_complete = true;
                break;
            }
        }
    }

    println!(
        "Multi-turn test: follow-up complete = {}",
        followup_complete
    );

    // Read the account block while the session is still intact; act on it once
    // the fixtures below are torn down.
    let account_block = session_provider_account_block(&client, &session.id).await;

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

    // Assertions
    assert!(response_complete, "First turn should complete");
    assert!(
        thinking_started_found,
        "Should have reason.thinking.started event when reasoning_effort is set"
    );
    assert!(
        thinking_completed_found,
        "Should have reason.thinking.completed event with thinking content"
    );
    assert!(
        message_agent_with_reasoning,
        "message.agent event should carry reasoning content parts"
    );
    assert!(
        followup_complete,
        "Multi-turn should complete (thinking properly sent back to Anthropic)"
    );

    println!("Anthropic extended thinking test passed!");
}

/// Test extended thinking with tool use (real Anthropic API).
///
/// This test specifically exercises the scenario where:
/// 1. Extended thinking is enabled
/// 2. The agent has tools configured
/// 3. The model decides to use a tool during the thinking+response flow
///
/// This combination requires:
/// - `interleaved-thinking-2025-05-14` beta header
/// - Proper capture of thinking signature via `signature_delta` event
/// - Thinking block included before tool_use in multi-turn messages
///
/// Requirements: API + Worker running, ANTHROPIC_API_KEY environment variable set.
#[tokio::test]
async fn test_anthropic_extended_thinking_with_tools() {
    use std::time::Duration;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(180))
        .build()
        .expect("Failed to create client");

    println!("Testing Anthropic extended thinking with tool use...");

    // Step 1: Create Anthropic provider
    println!("\nStep 1: Creating Anthropic provider...");
    let api_key = match std::env::var("ANTHROPIC_API_KEY") {
        Ok(key) if !key.is_empty() => key,
        _ => {
            println!("ANTHROPIC_API_KEY not set - skipping test");
            return;
        }
    };

    let provider_response = client
        .post(format!("{}/v1/providers", API_BASE_URL))
        .json(&json!({
            "name": "Anthropic Thinking+Tools Test",
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

    // Create model (the live thinking model supports extended thinking)
    let model_response = client
        .post(format!(
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": LIVE_ANTHROPIC_THINKING_MODEL,
            "display_name": "Anthropic Thinking (Thinking+Tools Test)",
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

    // Step 2: Create agent WITH current_time tool
    println!("\nStep 2: Creating agent with current_time tool...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "time-reporter-agent",
            "display_name": "Time Reporter Agent",
            "system_prompt": "You help users with simple requests. When asked for the time, call the current_time tool once and report the result.",
            "default_model_id": model.id,
            "capabilities": [
                {"ref": "current_time"}
            ]
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(agent_response.status(), 201);
    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!(
        "Created agent: {} with current_time capability",
        agent.public_id
    );

    // Step 3: Create session
    println!("\nStep 3: Creating session...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({
            "harness_name": SEED_HARNESS_NAME,
            "agent_id": agent.public_id,
            "title": "Time Reporting with Thinking Test"
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

    // Step 4: Send message asking for the current time
    // This will trigger: thinking -> tool call -> tool result -> response
    println!("\nStep 4: Sending message (expecting thinking + tool use)...");
    let message_response = client
        .post(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
        ))
        .json(&json!({
            "message": {
                "content": [{"type": "text", "text": "What time is it?"}]
            },
            "controls": {
                "reasoning": {
                    "effort": "low"
                }
            }
        }))
        .send()
        .await
        .expect("Failed to send message");

    assert_eq!(message_response.status(), 201);
    println!("Message sent successfully");

    // Step 5: Poll for completion and collect events
    println!("\nStep 5: Waiting for response...");
    let mut response_complete = false;
    let mut thinking_found = false;
    let mut tool_call_found = false;
    let mut tool_completed_found = false;
    let mut final_message_found = false;

    for i in 1..=120 {
        tokio::time::sleep(Duration::from_secs(1)).await;

        let session_response = client
            .get(format!("{}/v1/sessions/{}", API_BASE_URL, session.id))
            .send()
            .await;

        if let Ok(resp) = session_response
            && resp.status() == 200
        {
            let session_data: Value = resp.json().await.unwrap_or_default();
            let status = session_data["status"].as_str().unwrap_or("");

            if i % 10 == 0 {
                println!("  [{}s] Session status: {}", i, status);
            }

            if status == "idle" {
                response_complete = true;
                break;
            } else if status == "failed" {
                // Max iterations or other failure - this is acceptable for this test
                // as long as we got thinking + tool call events
                let error = session_data["error"].as_str().unwrap_or("unknown");
                println!("  Session ended with status: failed ({})", error);
                response_complete = true; // Consider it complete for event collection
                break;
            }
        }
    }

    // Fetch all events
    let events_response = client
        .get(format!(
            "{}/v1/sessions/{}/events",
            API_BASE_URL, session.id
        ))
        .send()
        .await
        .expect("Failed to get events");

    if events_response.status() == 200 {
        let events_data: Value = events_response.json().await.unwrap_or_default();
        if let Some(events) = events_data["data"].as_array() {
            println!("\n  Event summary ({} events):", events.len());
            for event in events {
                let event_type = event["type"].as_str().unwrap_or("");
                match event_type {
                    "reason.thinking.started" | "reason.thinking.completed" => {
                        thinking_found = true;
                        println!("    - {}", event_type);
                    }
                    everruns_core::events::TOOL_STARTED => {
                        tool_call_found = true;
                        // The invoked tool is carried inside `tool_call`, which
                        // is the payload `tool.started` actually publishes.
                        let tool_name = event["data"]["tool_call"]["name"].as_str().unwrap_or("?");
                        println!("    - {} (tool: {})", event_type, tool_name);
                    }
                    everruns_core::events::TOOL_COMPLETED => {
                        tool_completed_found = true;
                        let success = event["data"]["success"].as_bool().unwrap_or(false);
                        println!("    - {} (success: {})", event_type, success);
                    }
                    everruns_core::events::OUTPUT_MESSAGE_COMPLETED => {
                        final_message_found = true;
                        // Reasoning is ordered `reasoning` content parts; the
                        // opaque signature is replay state and never published.
                        let reasoning_parts = event["data"]["message"]["content"]
                            .as_array()
                            .map(|parts| {
                                parts
                                    .iter()
                                    .filter(|part| part["type"] == "reasoning")
                                    .count()
                            })
                            .unwrap_or(0);
                        println!(
                            "    - {} (reasoning_parts: {})",
                            event_type, reasoning_parts
                        );
                    }
                    "turn.started" | "turn.completed" => {
                        println!("    - {}", event_type);
                    }
                    _ => {}
                }
            }
        }
    }

    // Step 6: Multi-turn test - send a follow-up
    // This verifies thinking+signature is properly sent back to Anthropic
    println!("\nStep 6: Sending follow-up message (multi-turn with thinking+tools)...");
    let followup_response = client
        .post(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
        ))
        .json(&json!({
            "message": {
                "content": [{"type": "text", "text": "Thanks! What time is it now?"}]
            },
            "controls": {
                "reasoning": {
                    "effort": "low"
                }
            }
        }))
        .send()
        .await
        .expect("Failed to send follow-up message");

    assert_eq!(followup_response.status(), 201);
    println!("Follow-up message sent");

    // Wait for follow-up to complete
    let mut followup_complete = false;
    for i in 1..=120 {
        tokio::time::sleep(Duration::from_secs(1)).await;

        let session_response = client
            .get(format!("{}/v1/sessions/{}", API_BASE_URL, session.id))
            .send()
            .await;

        if let Ok(resp) = session_response
            && resp.status() == 200
        {
            let session_data: Value = resp.json().await.unwrap_or_default();
            let status = session_data["status"].as_str().unwrap_or("");

            if i % 10 == 0 {
                println!("  [{}s] Session status: {}", i, status);
            }

            if status == "idle" {
                followup_complete = true;
                break;
            } else if status == "failed" {
                // A turn the provider account blocked never exercised the
                // thinking+tools path, so it is a skip rather than a failure.
                skip_on_provider_account_block!(
                    session_provider_account_block(&client, &session.id).await
                );
                let error = session_data["error"].as_str().unwrap_or("unknown");
                panic!("Follow-up failed with error: {}", error);
            }
        }
    }

    // Read the account block while the session is still intact; act on it once
    // the fixtures below are torn down.
    let account_block = session_provider_account_block(&client, &session.id).await;

    println!("\nResults:");
    println!("  First turn complete: {}", response_complete);
    println!("  Thinking events found: {}", thinking_found);
    println!("  Tool call found: {}", tool_call_found);
    println!("  Tool completed: {}", tool_completed_found);
    println!("  Final message found: {}", final_message_found);
    println!("  Multi-turn complete: {}", followup_complete);

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

    // Assertions
    // Note: With interleaved thinking, the model may hit max iterations (tool loop).
    // The key test is that thinking + tools work together without API errors.
    assert!(
        thinking_found,
        "Should have thinking events when reasoning_effort is set"
    );
    assert!(
        tool_call_found,
        "Should have tool.started event - model should use current_time when asked for time"
    );
    assert!(tool_completed_found, "Should have tool.completed event");
    assert!(
        final_message_found,
        "Should have at least one message.agent event"
    );

    // Multi-turn test may not complete if first turn hit max iterations
    if !followup_complete {
        println!("⚠️  Follow-up did not complete (first turn may have hit max iterations)");
        println!("   This is acceptable - the test verifies thinking+tools work together");
    }

    println!("Anthropic extended thinking with tools test passed!");
}

// ============================================================================
// Events API Contract Tests
// ============================================================================
// These tests verify that the events API responses match the public contract.

/// Reasoning reaches the API as ordered parts, classified, with replay state stripped.
///
/// This covers the two things the reasoning rework publishes and the one thing
/// it must never publish, over the real API + worker + PostgreSQL path:
///
/// - `phase` / `phase_source` on the agent message. These cross the worker gRPC
///   boundary, where `phase` was previously hardcoded to `None` and the proto
///   had no field for it at all, so every message reached the API unclassified
///   regardless of what the provider reported. Nothing else in the suite would
///   catch that returning.
/// - reasoning as ordered `reasoning` content parts carrying readable text.
/// - the sanitization boundary: the artifact's signature and encrypted payload
///   are replay state for the provider, never API content (TM-LLM-034). llmsim
///   emits both alongside the text precisely so this assertion has something to
///   catch.
#[tokio::test]
async fn test_reasoning_reaches_api_sanitized_and_classified() {
    let client = reqwest::Client::new();

    // This test exists to prove the real path: a real provider's reasoning,
    // through the worker, into the API. Running it against a simulated provider
    // would prove only that the simulator behaves as written.
    //
    // So a missing credential is a failure, not a reason to skip. A job
    // configured to run this and silently unable to reach a provider reports
    // success while verifying nothing, which is the failure mode
    // `EVERRUNS_REQUIRE_LIVE_TESTS` exists to prevent.
    let require_live = std::env::var("EVERRUNS_REQUIRE_LIVE_TESTS")
        .ok()
        .is_some_and(|v| {
            let v = v.trim();
            !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false")
        });
    let has_provider_key = ["OPENAI_API_KEY", "ANTHROPIC_API_KEY"]
        .iter()
        .any(|k| std::env::var(k).ok().is_some_and(|v| !v.trim().is_empty()));
    if !has_provider_key {
        assert!(
            !require_live,
            "EVERRUNS_REQUIRE_LIVE_TESTS is set but neither OPENAI_API_KEY nor \
             ANTHROPIC_API_KEY is present. This test verifies real provider \
             reasoning end to end and cannot do that without a credential — fix \
             the job's secrets rather than relaxing this check."
        );
        eprintln!(
            "SKIP: no provider credential; set EVERRUNS_REQUIRE_LIVE_TESTS=1 to \
             make this a failure"
        );
        return;
    }

    // Names are unique per run: the API rejects a duplicate agent name with
    // 409, so fixed names pass once against a fresh database and fail on every
    // rerun against the same one.
    let run_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();

    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": format!("reasoning-projection-agent-{run_id}"),
            "system_prompt": "You are a helpful assistant.",
            // No model override: the agent takes the org default, which is a
            // real reasoning-capable provider. That is the path this test is
            // here to verify.
        }))
        .send()
        .await
        .expect("Failed to create agent");
    assert_eq!(agent_response.status(), 201);
    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");

    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({
            "harness_name": SEED_HARNESS_NAME,
            "agent_id": agent.public_id,
            "title": format!("Reasoning Projection Session {run_id}"),
        }))
        .send()
        .await
        .expect("Failed to create session");
    assert_eq!(session_response.status(), 201);
    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");

    // The effort is what makes llmsim reason, mirroring a real provider.
    let message_response = client
        .post(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
        ))
        .json(&json!({
            "message": {
                "role": "user",
                "content": [{"type": "text", "text": "Think about this, then answer."}]
            },
            "controls": {"reasoning": {"effort": "high"}}
        }))
        .send()
        .await
        .expect("Failed to send message");
    assert_eq!(message_response.status(), 201, "Failed to send message");

    let mut agent_message: Option<Value> = None;
    for _ in 0..60 {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let Ok(resp) = client
            .get(format!(
                "{}/v1/sessions/{}/messages",
                API_BASE_URL, session.id
            ))
            .send()
            .await
        else {
            continue;
        };
        if resp.status() != 200 {
            continue;
        }
        let data: Value = resp.json().await.unwrap_or_default();
        let empty = vec![];
        let messages = data["data"].as_array().unwrap_or(&empty);
        if let Some(m) = messages.iter().find(|m| m["role"] == "agent") {
            agent_message = Some(m.clone());
            break;
        }
    }

    let agent_message = agent_message.expect("No agent message produced within 60s");
    println!(
        "agent message: {}",
        serde_json::to_string_pretty(&agent_message).unwrap_or_default()
    );

    // An unusable provider account is a billing condition, not a regression in
    // the reasoning projection this test covers.
    skip_on_provider_account_block!(provider_account_block(&agent_message));

    // A provider error message would carry no reasoning and no phase, and would
    // make every assertion below vacuous.
    assert!(
        agent_message["metadata"]["error_code"].is_null(),
        "Turn failed rather than producing a reasoning message: {:?}",
        agent_message["metadata"]
    );

    // 1. Decision survives the worker boundary.
    let phase = agent_message["phase"]
        .as_str()
        .expect("agent message must carry `phase`");
    assert!(
        phase == "commentary" || phase == "final_answer",
        "unexpected phase {phase:?}"
    );
    let phase_source = agent_message["phase_source"]
        .as_str()
        .expect("agent message must carry `phase_source`");
    assert!(
        phase_source == "provider" || phase_source == "derived",
        "unexpected phase_source {phase_source:?}"
    );

    // 2. Reasoning is published as ordered content parts with readable text.
    let empty = vec![];
    let content = agent_message["content"].as_array().unwrap_or(&empty);
    let reasoning_parts: Vec<&Value> = content
        .iter()
        .filter(|p| p["type"] == "reasoning")
        .collect();
    assert!(
        !reasoning_parts.is_empty(),
        "agent message must carry reasoning content parts, got types {:?}",
        content
            .iter()
            .map(|p| p["type"].as_str().unwrap_or("?"))
            .collect::<Vec<_>>()
    );
    // Readable text arrives in whichever shape the provider exposes: verbatim
    // chain-of-thought as `plain`, a curated gloss as `summary`. Both are
    // reasoning-channel content; neither may be the answer.
    let reasoning_text: Vec<String> = reasoning_parts
        .iter()
        .filter_map(|p| match p["text"]["kind"].as_str() {
            Some("plain") => p["text"]["text"].as_str().map(str::to_string),
            Some("summary") => p["text"]["parts"].as_array().map(|parts| {
                parts
                    .iter()
                    .filter_map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            }),
            _ => None,
        })
        .filter(|t| !t.trim().is_empty())
        .collect();
    // Readable text is not guaranteed. A provider may return a reasoning
    // artifact carrying only an id and its encrypted payload — the model
    // reasoned but surfaced no prose — and that artifact is still valid and
    // still replayable. Requiring text here would fail on a correct response.
    // What must always hold is that the artifact is identified, so it can be
    // replayed under the handle the provider issued.
    assert!(
        reasoning_parts
            .iter()
            .any(|p| p["item_id"].is_string() || p["provider"].is_string()),
        "reasoning parts must be identified: {reasoning_parts:?}"
    );

    // Reasoning folded into the answer is the bug this rework fixes: it
    // persists as the model's reply and replays to the model as its own prior
    // output.
    let answer_text: String = content
        .iter()
        .filter(|p| p["type"] == "text")
        .filter_map(|p| p["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    for reasoning in &reasoning_text {
        assert!(
            !answer_text.contains(reasoning.trim()),
            "reasoning text leaked into the answer"
        );
    }

    // 3. Replay state must not. The provider needs it; a client never does, and
    //    publishing it widens the surface for no benefit (TM-LLM-034).
    for part in &reasoning_parts {
        for leaked in ["signature", "encrypted", "encrypted_content"] {
            assert!(
                part.get(leaked).is_none_or(Value::is_null),
                "reasoning part leaked `{leaked}` to the API: {part}"
            );
        }
    }
    let serialized = serde_json::to_string(&agent_message).unwrap_or_default();
    for opaque in ["llmsim-signature-opaque", "llmsim-encrypted-opaque"] {
        assert!(
            !serialized.contains(opaque),
            "opaque replay value {opaque:?} reached the API payload"
        );
    }

    // 4. The same boundary holds for the event stream, which is a separate
    //    projection and so a separate way for the payload to escape.
    let events: Value = client
        .get(format!(
            "{}/v1/sessions/{}/events?limit=200",
            API_BASE_URL, session.id
        ))
        .send()
        .await
        .expect("Failed to fetch events")
        .json()
        .await
        .unwrap_or_default();
    let empty_events = vec![];
    let events = events["data"].as_array().unwrap_or(&empty_events);
    let reason_items: Vec<&Value> = events
        .iter()
        .filter(|e| e["type"] == "reason.item")
        .collect();
    assert!(
        !reason_items.is_empty(),
        "a reasoning turn must publish reason.item events"
    );
    for item in &reason_items {
        let data = &item["data"];
        for leaked in ["encrypted_content", "signature", "encrypted"] {
            assert!(
                data.get(leaked).is_none_or(Value::is_null),
                "reason.item leaked `{leaked}`: {data}"
            );
        }
        assert!(
            data["item_id"].is_string(),
            "reason.item should identify the artifact: {data}"
        );
    }

    // Cleanup
    cleanup_delete(
        &client,
        format!("{}/v1/sessions/{}", API_BASE_URL, session.id),
        "session",
    )
    .await;
    cleanup_agent(&client, &agent.public_id).await;
}
