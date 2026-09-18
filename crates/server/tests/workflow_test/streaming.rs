use crate::support::*;
use everruns_platform::Agent;
use everruns_platform::Session;
use everruns_provider::model::Model;
use everruns_provider::provider::Provider;
use serde_json::{Value, json};

/// Test that streaming events are emitted during LLM generation.
///
/// This test verifies that:
/// 1. `output.message.started` event is emitted when LLM starts generating (durable, persisted)
/// 2. `output.message.delta` events are ephemeral (may not appear in PG events list)
/// 3. `llm.generation` events are durable and appear in PG events list
///
/// Note: This test uses LlmSim which simulates streaming but does not produce
/// extended thinking events (reason.thinking.delta, reason.thinking.completed).
/// To test full extended thinking flow, use a real Anthropic model with reasoning_effort.
#[tokio::test]
async fn test_streaming_events_emitted() {
    use std::time::Duration;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("Failed to create client");

    println!("Testing streaming events are emitted...");

    // Step 1: Create LlmSim provider and model
    println!("\nStep 1: Creating LlmSim provider and model...");
    let provider_response = client
        .post(format!("{}/v1/providers", API_BASE_URL))
        .json(&json!({
            "name": "Streaming Test Provider",
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
    println!("Created provider: {}", provider.id);

    let model_response = client
        .post(format!(
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": "llmsim-streaming",
            "display_name": "LlmSim Streaming Model",
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

    // Step 2: Create agent
    println!("\nStep 2: Creating agent...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "streaming-test-agent",
            "display_name": "Streaming Test Agent",
            "system_prompt": "You are a helpful assistant. Respond briefly.",
            "default_model_id": model.id.to_string()
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
        .json(&json!({"harness_name": SEED_HARNESS_NAME, "agent_id": agent.public_id, "title": "Streaming Test Session"}))
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
                "content": [{"type": "text", "text": "Hello, how are you?"}]
            }
        }))
        .send()
        .await
        .expect("Failed to create message");

    assert_eq!(message_response.status(), 201);
    println!("Message created");

    // Step 5: Wait for workflow to complete
    println!("\nStep 5: Waiting for agent response (up to 30 seconds)...");
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

            for msg in messages {
                if msg["role"] == "agent" {
                    agent_response_found = true;
                    println!("Found agent response after {}s", i);
                    break;
                }
            }

            if agent_response_found {
                break;
            }
        }

        if i % 5 == 0 {
            println!("Still waiting... ({}s)", i);
        }
    }

    assert!(
        agent_response_found,
        "Agent should have responded within 30 seconds"
    );

    // Step 6: Verify streaming events
    println!("\nStep 6: Verifying streaming events...");
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

    // Check for output.message.started event
    let started_events: Vec<_> = events
        .iter()
        .filter(|e| e["type"] == "output.message.started")
        .collect();
    println!(
        "Found {} output.message.started events",
        started_events.len()
    );
    assert!(
        !started_events.is_empty(),
        "Expected at least one output.message.started event"
    );

    // Verify output.message.started has turn_id
    let started_event = &started_events[0];
    assert!(
        started_event["data"]["turn_id"].is_string(),
        "output.message.started should have turn_id"
    );
    println!(
        "output.message.started event: turn_id={}, model={:?}",
        started_event["data"]["turn_id"], started_event["data"]["model"]
    );

    // Note: reason.thinking.started events are only emitted when reasoning_effort is set
    // LLMSim doesn't support extended thinking, so we don't expect thinking events here.
    let thinking_events: Vec<_> = events
        .iter()
        .filter(|e| e["type"] == "reason.thinking.started")
        .collect();
    println!(
        "Found {} reason.thinking.started events (expected 0 for LLMSim)",
        thinking_events.len()
    );

    // Check for output.message.delta events
    // Note: delta events are ephemeral (skip PG persistence, delivered via EventDelivery only).
    // They may be absent from the PG events list — this is expected behavior.
    let delta_events: Vec<_> = events
        .iter()
        .filter(|e| e["type"] == "output.message.delta")
        .collect();
    println!(
        "Found {} output.message.delta events (ephemeral — may be 0 in PG)",
        delta_events.len()
    );

    // If deltas are present (e.g., EventDelivery::InMemory mode still persists), verify structure
    if let Some(delta_event) = delta_events.first() {
        assert!(
            delta_event["data"]["turn_id"].is_string(),
            "output.message.delta should have turn_id"
        );
        assert!(
            delta_event["data"]["delta"].is_string(),
            "output.message.delta should have delta field"
        );
        assert!(
            delta_event["data"]["accumulated"].is_string(),
            "output.message.delta should have accumulated field"
        );
        println!(
            "output.message.delta event: delta='{}', accumulated='{}'",
            delta_event["data"]["delta"].as_str().unwrap_or(""),
            delta_event["data"]["accumulated"].as_str().unwrap_or("")
        );
    }

    // Check for output.message.completed event
    let completed_events: Vec<_> = events
        .iter()
        .filter(|e| e["type"] == "output.message.completed")
        .collect();
    println!(
        "Found {} output.message.completed events",
        completed_events.len()
    );
    assert!(
        !completed_events.is_empty(),
        "Expected at least one output.message.completed event"
    );

    // Verify output.message.completed has required fields (message)
    let completed_event = &completed_events[0];
    assert!(
        completed_event["data"]["message"].is_object(),
        "output.message.completed should have message object"
    );
    assert!(
        completed_event["data"]["message"]["role"] == "agent",
        "output.message.completed message should have role=agent"
    );
    assert!(
        completed_event["data"]["message"]["content"].is_array(),
        "output.message.completed message should have content array"
    );
    println!(
        "output.message.completed event: message_id={}",
        completed_event["data"]["message"]["id"]
            .as_str()
            .unwrap_or("unknown")
    );

    // Check for llm.generation event with time_to_first_token_ms
    let llm_events: Vec<_> = events
        .iter()
        .filter(|e| e["type"] == "llm.generation")
        .collect();
    println!("Found {} llm.generation events", llm_events.len());
    assert!(
        !llm_events.is_empty(),
        "Expected at least one durable llm.generation event"
    );

    if let Some(llm_event) = llm_events.first() {
        assert!(
            llm_event["data"]["metadata"]["success"].as_bool() == Some(true),
            "llm.generation should be successful"
        );
        if let Some(ttft) = llm_event["data"]["metadata"]["time_to_first_token_ms"].as_u64() {
            println!("llm.generation time_to_first_token_ms: {}ms", ttft);
        } else {
            println!("llm.generation time_to_first_token_ms: not set (optional)");
        }
    }

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

    println!("Streaming events test passed!");
}

/// Test cancel turn endpoint behavior.
///
/// This test verifies the cancel endpoint returns correct responses
/// for idle sessions (no active turn to cancel).
///
/// Requirements: API + Worker running
#[tokio::test]
async fn test_cancel_turn_endpoint() {
    let client = reqwest::Client::new();

    println!("Testing cancel turn endpoint...");

    // Step 1: Create LlmSim provider
    println!("\nStep 1: Creating LlmSim provider...");
    let provider_response = client
        .post(format!("{}/v1/providers", API_BASE_URL))
        .json(&json!({
            "name": "LlmSim Cancel Test",
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

    // Step 2: Create model
    println!("\nStep 2: Creating model...");
    let model_response = client
        .post(format!(
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": "llmsim-cancel-test",
            "display_name": "LlmSim Cancel Test Model",
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

    // Step 3: Create agent with LlmSim model
    println!("\nStep 3: Creating agent...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "cancel-test-agent",
            "display_name": "Cancel Test Agent",
            "system_prompt": "You are a helpful assistant.",
            "default_model_id": model.id.to_string()
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(agent_response.status(), 201, "Failed to create agent");
    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!("Created agent: {}", agent.public_id);

    // Step 4: Create session
    println!("\nStep 4: Creating session...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({
            "harness_name": SEED_HARNESS_NAME,
            "agent_id": agent.public_id,
            "title": "Cancel Test Session"
        }))
        .send()
        .await
        .expect("Failed to create session");

    assert_eq!(session_response.status(), 201, "Failed to create session");
    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");
    println!("Created session: {}", session.id);

    // Step 5: Test cancel on idle session (should return 200 with no_op status)
    println!("\nStep 5: Testing cancel on idle session...");
    let cancel_response = client
        .post(format!(
            "{}/v1/sessions/{}/cancel",
            API_BASE_URL, session.id
        ))
        .send()
        .await
        .expect("Failed to call cancel endpoint");

    assert_eq!(
        cancel_response.status(),
        200,
        "Expected 200 for cancelling idle session (no-op), got {}",
        cancel_response.status()
    );
    let cancel_body: Value = cancel_response
        .json()
        .await
        .expect("Failed to parse cancel response");
    assert_eq!(
        cancel_body["status"], "no_op",
        "Expected no_op status for idle session, got {:?}",
        cancel_body["status"]
    );
    println!("Correctly returned no_op: {}", cancel_body["message"]);

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
        format!("{}/v1/providers/{}", API_BASE_URL, provider.id),
        "provider",
    )
    .await;

    println!("Cancel turn endpoint test passed!");
}

/// Test events list endpoint returns correct JSON structure
#[tokio::test]
async fn test_events_api_contract() {
    let client = reqwest::Client::new();

    println!("Testing events API contract...");

    // Step 1: Create agent and session
    let agent: Agent = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "events-contract-test-agent",
            "display_name": "Events Contract Test Agent",
            "system_prompt": "You are helpful"
        }))
        .send()
        .await
        .expect("Failed to create agent")
        .json()
        .await
        .expect("Failed to parse agent");

    let session: Session = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({"harness_name": SEED_HARNESS_NAME, "agent_id": agent.public_id, "title": "Events Contract Test"}))
        .send()
        .await
        .expect("Failed to create session")
        .json()
        .await
        .expect("Failed to parse session");

    println!("Created session: {}", session.id);

    // Step 2: List events and verify contract structure (may be empty for new session)
    println!("\nVerifying events list API contract...");
    let events_response = client
        .get(format!(
            "{}/v1/sessions/{}/events",
            API_BASE_URL, session.id
        ))
        .send()
        .await
        .expect("Failed to list events");

    assert_eq!(events_response.status(), 200);

    let events_json: Value = events_response
        .json()
        .await
        .expect("Failed to parse events");

    // Verify response structure has 'data' array (may be empty)
    assert!(
        events_json["data"].is_array(),
        "Response should have 'data' array"
    );
    let events = events_json["data"].as_array().unwrap();
    println!("Found {} events", events.len());

    // If there are events, verify each has required contract fields
    for event in events {
        // Required fields per knowledge/execution/events.md
        assert!(event["id"].is_string(), "Event must have 'id' string");
        assert!(event["type"].is_string(), "Event must have 'type' string");
        assert!(event["ts"].is_string(), "Event must have 'ts' string");
        assert!(
            event["session_id"].is_string(),
            "Event must have 'session_id' string"
        );
        assert!(
            event["context"].is_object(),
            "Event must have 'context' object"
        );
        assert!(event["data"].is_object(), "Event must have 'data' object");

        // Verify ID format (event_ prefix)
        let id = event["id"].as_str().unwrap();
        assert!(
            id.starts_with("event_"),
            "Event ID should have 'event_' prefix, got: {}",
            id
        );

        // Verify session_id format (session_ prefix)
        let sid = event["session_id"].as_str().unwrap();
        assert!(
            sid.starts_with("session_"),
            "Session ID should have 'session_' prefix, got: {}",
            sid
        );

        // Verify type is known (not "unsupported")
        let event_type = event["type"].as_str().unwrap();
        assert_ne!(
            event_type, "unsupported",
            "Unsupported events should be filtered"
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

    println!("Events API contract test passed!");
}

/// Test SSE streaming endpoint returns correct headers
#[tokio::test]
async fn test_events_sse_contract() {
    use futures::StreamExt;

    let client = reqwest::Client::new();

    println!("Testing SSE events contract...");

    // Step 1: Create agent and session
    let agent: Agent = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "sse-contract-test-agent",
            "display_name": "SSE Contract Test Agent",
            "system_prompt": "You are helpful"
        }))
        .send()
        .await
        .expect("Failed to create agent")
        .json()
        .await
        .expect("Failed to parse agent");

    let session: Session = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({"harness_name": SEED_HARNESS_NAME, "agent_id": agent.public_id, "title": "SSE Contract Test"}))
        .send()
        .await
        .expect("Failed to create session")
        .json()
        .await
        .expect("Failed to parse session");

    println!("Created session: {}", session.id);

    // Step 2: Connect to SSE stream and verify headers and initial data
    println!("\nConnecting to SSE stream...");
    let sse_url = format!("{}/v1/sessions/{}/sse", API_BASE_URL, session.id);

    let sse_response = client
        .get(&sse_url)
        .header("Accept", "text/event-stream")
        .send()
        .await
        .expect("Failed to connect to SSE");

    assert_eq!(sse_response.status(), 200);

    // Verify content type is text/event-stream
    let content_type = sse_response.headers().get("content-type");
    assert!(
        content_type.is_some(),
        "SSE response should have content-type header"
    );
    let ct = content_type.unwrap().to_str().unwrap();
    assert!(
        ct.contains("text/event-stream"),
        "Content-type should be text/event-stream, got: {}",
        ct
    );

    // Read first chunk with timeout (SSE streams are long-lived, can't use .text())
    let mut stream = sse_response.bytes_stream();
    let first_chunk = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
        .await
        .expect("Timeout reading first SSE chunk")
        .expect("Stream ended unexpectedly")
        .expect("Failed to read chunk");

    let chunk_str = String::from_utf8_lossy(&first_chunk);
    println!("SSE first chunk:\n{}", chunk_str);

    // Verify SSE format (should have event: line and data: line)
    assert!(
        chunk_str.contains("event:") || chunk_str.contains("data:"),
        "SSE should have event: or data: lines"
    );

    // Cleanup
    cleanup_delete(
        &client,
        format!("{}/v1/sessions/{}", API_BASE_URL, session.id),
        "session",
    )
    .await;
    cleanup_agent(&client, &agent.public_id).await;

    println!("SSE contract test passed!");
}

// =============================================================================
// Durable SSE In-Process Integration Tests
// =============================================================================
//
// These tests use in-process testing with tower::ServiceExt for regular endpoints.
// For SSE endpoints (infinite streams), we spawn a TCP server and use reqwest
// to read streaming chunks with timeout.

mod durable_sse_tests {
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
    };
    use everruns_durable::InMemoryWorkflowEventStore;
    use everruns_server::api::durable;
    use futures::StreamExt;
    use http_body_util::BodyExt;
    use std::sync::Arc;
    use tokio::net::TcpListener;
    use tower::ServiceExt;

    fn test_auth_state() -> everruns_server::auth::middleware::AuthState {
        use everruns_server::auth::config::AuthConfig;
        use everruns_server::storage::StorageBackend;
        let config = AuthConfig::default();
        everruns_server::auth::middleware::AuthState::builtin(
            config,
            std::sync::Arc::new(StorageBackend::in_memory()),
        )
    }

    /// Create a test router with in-memory durable store
    fn create_test_app() -> Router {
        let store = Arc::new(InMemoryWorkflowEventStore::new());
        let state = durable::AppState::new(
            Some(store),
            test_auth_state(),
            None,
            "in_memory".to_string(),
        );
        durable::routes(state)
    }

    /// Create a test router with no durable store (simulates 503)
    fn create_test_app_no_store() -> Router {
        let state = durable::AppState::new(None, test_auth_state(), None, "in_memory".to_string());
        durable::routes(state)
    }

    /// Spawn a test server and return its base URL
    async fn spawn_test_server(app: Router) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{}", addr);

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        // Give the server a moment to start
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        url
    }

    /// Test global durable SSE stream contract (/v1/durable/sse)
    ///
    /// Verifies:
    /// - SSE connection establishes successfully (200 OK)
    /// - Content-Type is text/event-stream
    /// - First event is "connected"
    /// - Subsequent event is "snapshot" with expected structure
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_durable_sse_global_stream() {
        let app = create_test_app();
        let base_url = spawn_test_server(app).await;

        let client = reqwest::Client::new();
        let response = client
            .get(format!("{}/v1/durable/sse", base_url))
            .header("Accept", "text/event-stream")
            .send()
            .await
            .expect("Failed to connect to SSE");

        assert_eq!(response.status(), 200);

        // Verify content-type header
        let content_type = response
            .headers()
            .get("content-type")
            .expect("Missing content-type header")
            .to_str()
            .unwrap();
        assert!(
            content_type.contains("text/event-stream"),
            "Content-type should be text/event-stream, got: {}",
            content_type
        );

        // Read chunks with timeout until we have enough data
        let mut stream = response.bytes_stream();
        let mut collected = String::new();

        for _ in 0..10 {
            match tokio::time::timeout(std::time::Duration::from_secs(2), stream.next()).await {
                Ok(Some(Ok(chunk))) => {
                    collected.push_str(&String::from_utf8_lossy(&chunk));
                    // Stop when we have both events
                    if collected.contains("event: connected")
                        && collected.contains("event: snapshot")
                    {
                        break;
                    }
                }
                Ok(Some(Err(e))) => panic!("Stream error: {}", e),
                Ok(None) => break, // Stream ended
                Err(_) => break,   // Timeout - that's OK for SSE
            }
        }

        println!("Durable SSE received:\n{}", collected);

        // Verify "connected" event
        assert!(
            collected.contains("event: connected"),
            "Should receive 'connected' event"
        );

        // Verify "snapshot" event with JSON data
        assert!(
            collected.contains("event: snapshot"),
            "Should receive 'snapshot' event"
        );
        assert!(
            collected.contains("\"health\""),
            "Snapshot should contain health"
        );
        assert!(
            collected.contains("\"workers\""),
            "Snapshot should contain workers"
        );
        assert!(
            collected.contains("\"workflows\""),
            "Snapshot should contain workflows"
        );
        assert!(
            collected.contains("\"tasks\""),
            "Snapshot should contain tasks"
        );
        assert!(collected.contains("\"dlq\""), "Snapshot should contain dlq");
        assert!(
            collected.contains("\"circuit_breakers\""),
            "Snapshot should contain circuit_breakers"
        );

        // Verify retry hint is present
        assert!(
            collected.contains("retry:"),
            "SSE should include retry hint"
        );

        println!("Durable global SSE test passed!");
    }

    /// Test SSE connected event has valid JSON format with timestamp
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_durable_sse_connected_event_format() {
        let app = create_test_app();
        let base_url = spawn_test_server(app).await;

        let client = reqwest::Client::new();
        let response = client
            .get(format!("{}/v1/durable/sse", base_url))
            .header("Accept", "text/event-stream")
            .send()
            .await
            .expect("Failed to connect to SSE");

        assert_eq!(response.status(), 200);

        // Read first chunk
        let mut stream = response.bytes_stream();
        let mut collected = String::new();

        match tokio::time::timeout(std::time::Duration::from_secs(2), stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                collected.push_str(&String::from_utf8_lossy(&chunk));
            }
            Ok(Some(Err(e))) => panic!("Stream error: {}", e),
            Ok(None) => panic!("Stream ended unexpectedly"),
            Err(_) => panic!("Timeout reading first chunk"),
        }

        // Verify SSE format (event: and data: lines)
        assert!(collected.contains("event:"), "SSE should have event: field");
        assert!(
            collected.contains("event: connected"),
            "Should have connected event"
        );
        assert!(
            collected.contains("data:"),
            "connected event should have data: field"
        );

        // Extract and validate connected event JSON
        if let Some(data_start) = collected.find("data:") {
            let data_line = &collected[data_start..];
            if let Some(json_start) = data_line.find('{') {
                let potential_json = &data_line[json_start..];
                if let Some(json_end) = potential_json.find('}') {
                    let json_str = &potential_json[..=json_end];
                    let parsed: serde_json::Value = serde_json::from_str(json_str)
                        .expect("connected data should be valid JSON");
                    // Connected event has {"status":"connected"}
                    assert!(
                        parsed.get("status").is_some(),
                        "connected event should have status field"
                    );
                    assert_eq!(
                        parsed["status"], "connected",
                        "status should be 'connected'"
                    );
                }
            }
        }

        println!("Durable SSE format test passed!");
    }

    /// Test durable SSE returns 503 when store is not available
    #[tokio::test]
    async fn test_durable_sse_no_store_returns_503() {
        let app = create_test_app_no_store();

        let request = Request::builder()
            .uri("/v1/durable/sse")
            .header("Accept", "text/event-stream")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(
            response.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "Should return 503 when store is not available"
        );

        println!("Durable SSE no-store test passed!");
    }

    /// Test workflow-specific SSE stream returns 404 for non-existent workflow
    #[tokio::test]
    async fn test_durable_workflow_sse_not_found() {
        let app = create_test_app();

        let request = Request::builder()
            .uri("/v1/durable/workflows/01234567-89ab-cdef-0123-456789abcdef/sse")
            .header("Accept", "text/event-stream")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "Should return 404 for non-existent workflow"
        );

        println!("Workflow SSE not found test passed!");
    }

    /// Test health endpoint returns system health data
    #[tokio::test]
    async fn test_durable_health_endpoint() {
        let app = create_test_app();

        let request = Request::builder()
            .uri("/v1/durable/health")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let health: serde_json::Value =
            serde_json::from_slice(&body_bytes).expect("Health should be valid JSON");

        // Verify health response structure
        assert!(health.get("total_workers").is_some());
        assert!(health.get("active_workers").is_some());
        assert!(health.get("total_capacity").is_some());
        assert!(health.get("current_load").is_some());

        println!("Durable health endpoint test passed!");
    }

    /// Test workers list endpoint
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_durable_workers_endpoint() {
        let app = create_test_app();

        let request = Request::builder()
            .uri("/v1/durable/workers")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let workers: serde_json::Value =
            serde_json::from_slice(&body_bytes).expect("Workers should be valid JSON");

        // Verify response structure: { data: [...], total: N }
        assert!(workers.get("data").is_some(), "Should have data field");
        assert!(workers.get("total").is_some(), "Should have total field");
        assert!(workers["data"].is_array(), "data should be an array");

        println!("Durable workers endpoint test passed!");
    }

    /// Test workflows list endpoint
    #[tokio::test]
    async fn test_durable_workflows_endpoint() {
        let app = create_test_app();

        let request = Request::builder()
            .uri("/v1/durable/workflows")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let workflows: serde_json::Value =
            serde_json::from_slice(&body_bytes).expect("Workflows should be valid JSON");

        // Verify structure
        assert!(workflows.get("total").is_some());
        assert!(workflows.get("data").is_some());
        assert!(workflows["data"].is_array());

        println!("Durable workflows endpoint test passed!");
    }

    /// Test circuit breakers list endpoint
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_durable_circuit_breakers_endpoint() {
        let app = create_test_app();

        let request = Request::builder()
            .uri("/v1/durable/circuit-breakers")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let breakers: serde_json::Value =
            serde_json::from_slice(&body_bytes).expect("Circuit breakers should be valid JSON");

        // Verify response structure: { data: [...], total: N }
        assert!(breakers.get("data").is_some(), "Should have data field");
        assert!(breakers.get("total").is_some(), "Should have total field");
        assert!(breakers["data"].is_array(), "data should be an array");

        println!("Durable circuit breakers endpoint test passed!");
    }
}
