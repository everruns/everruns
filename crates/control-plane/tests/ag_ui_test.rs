// AG-UI Protocol Integration Tests
// Run with: cargo test -p everruns-control-plane --test ag_ui_test -- --test-threads=1
// Requires: API running (uses LlmSim, no real API keys needed)

use everruns_core::{Agent, Session};
use serde_json::json;

const API_BASE_URL: &str = "http://localhost:9000";
const DEFAULT_ORG: &str = "org_00000000000000000000000000000001";

/// Test that AG-UI SSE endpoint returns connected event
#[tokio::test]
async fn test_ag_ui_sse_endpoint_connected() {
    let client = reqwest::Client::new();

    println!("Testing AG-UI SSE endpoint...");

    // Create an agent
    let create_agent_response = client
        .post(format!("{}/v1/orgs/{}/agents", API_BASE_URL, DEFAULT_ORG))
        .json(&json!({
            "name": "AG-UI Test Agent",
            "system_prompt": "You are a test agent."
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(create_agent_response.status(), 201);
    let agent: Agent = create_agent_response
        .json()
        .await
        .expect("Failed to parse agent");
    println!("Created agent: {}", agent.id);

    // Create a session
    let create_session_response = client
        .post(format!(
            "{}/v1/orgs/{}/agents/{}/sessions",
            API_BASE_URL, DEFAULT_ORG, agent.id
        ))
        .json(&json!({}))
        .send()
        .await
        .expect("Failed to create session");

    assert_eq!(create_session_response.status(), 201);
    let session: Session = create_session_response
        .json()
        .await
        .expect("Failed to parse session");
    println!("Created session: {}", session.id);

    // Connect to AG-UI SSE endpoint
    let sse_url = format!(
        "{}/v1/orgs/{}/agents/{}/sessions/{}/ag-ui/sse",
        API_BASE_URL, DEFAULT_ORG, agent.id, session.id
    );
    println!("Connecting to AG-UI SSE: {}", sse_url);

    let sse_response = client
        .get(&sse_url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .expect("Failed to connect to AG-UI SSE");

    assert_eq!(sse_response.status(), 200);
    println!("Connected to AG-UI SSE endpoint");

    // Read first event (should be "connected")
    let body = sse_response.text().await.expect("Failed to read SSE body");
    println!(
        "SSE response (first chunk): {}",
        &body[..std::cmp::min(200, body.len())]
    );

    // Should contain "connected" event
    assert!(
        body.contains("event: connected"),
        "Should receive connected event"
    );
    assert!(
        body.contains("\"status\":\"connected\""),
        "Should have connected status in data"
    );

    // Cleanup
    client
        .delete(format!(
            "{}/v1/orgs/{}/agents/{}",
            API_BASE_URL, DEFAULT_ORG, agent.id
        ))
        .send()
        .await
        .expect("Failed to delete agent");

    println!("AG-UI SSE endpoint test passed!");
}
