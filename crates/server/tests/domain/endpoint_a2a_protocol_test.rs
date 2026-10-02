//! A2A protocol conformance for endpoints created through the Agent endpoints
//! API, the way every new A2A endpoint is created. The fixtures in
//! `endpoint_a2a_integration_test.rs` seed App-era endpoints instead.

use crate::test_harness;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use test_harness::TestServer;

struct A2aEndpoint {
    id: String,
    api_key: String,
}

async fn create_agent(server: &TestServer) -> String {
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": format!("a2a-protocol-{}", uuid::Uuid::new_v4().simple()),
                "display_name": "A2A protocol agent",
                "description": "Answers A2A calls.",
                "system_prompt": "You are a brief test agent."
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    agent["id"].as_str().unwrap().to_string()
}

async fn create_a2a_endpoint(server: &TestServer, session_mode: &str) -> A2aEndpoint {
    let agent_id = create_agent(server).await;
    let api_key = format!("evra2a_{}", uuid::Uuid::new_v4().simple());
    let endpoint: Value = server
        .post(
            &format!("/v1/agents/{agent_id}/endpoints"),
            json!({
                "channel_type": "a2a",
                "channel_config": {
                    "session_mode": session_mode,
                    "message": "{{a2a.text}}",
                    "api_key_hash": hex::encode(Sha256::digest(api_key.as_bytes())),
                    "api_key_prefix": &api_key[..15],
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let id = endpoint["id"].as_str().unwrap().to_string();
    server
        .post(
            &format!("/v1/agents/{agent_id}/endpoints/{id}/publish"),
            json!({}),
        )
        .await
        .assert_success();
    A2aEndpoint { id, api_key }
}

async fn rpc(
    server: &TestServer,
    endpoint: &A2aEndpoint,
    extra_headers: Vec<(&str, &str)>,
    body: Value,
) -> test_harness::TestResponse {
    let auth = format!("Bearer {}", endpoint.api_key);
    let mut headers = vec![
        ("content-type", "application/json"),
        ("authorization", auth.as_str()),
    ];
    headers.extend(extra_headers);
    server
        .request_raw(
            Method::POST,
            &format!("/v1/e/{}/a2a", endpoint.id),
            headers,
            serde_json::to_vec(&body).unwrap(),
        )
        .await
}

/// Regression: canonical `/v1/e/{endpoint_id}/a2a` routes compared the
/// endpoint's synthetic App id against a legacy App id it never had, so every
/// A2A endpoint created through the Agent endpoints API answered 404.
#[tokio::test]
async fn api_created_a2a_endpoint_serves_card_and_accepts_messages() {
    let server = TestServer::in_memory().await;
    let endpoint = create_a2a_endpoint(&server, "session_per_invocation").await;

    let card: Value = server
        .get(&format!(
            "/v1/e/{}/a2a/.well-known/agent-card.json",
            endpoint.id
        ))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(card["name"], "A2A protocol agent");

    let sent: Value = rpc(
        &server,
        &endpoint,
        vec![],
        json!({
            "jsonrpc": "2.0",
            "id": "send-1",
            "method": "message/send",
            "params": {
                "message": {
                    "role": "user",
                    "messageId": "m-1",
                    "parts": [{ "kind": "text", "text": "hello" }]
                }
            }
        }),
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    assert!(sent.get("error").is_none(), "{sent}");
    assert_eq!(sent["result"]["kind"], "task");
}

/// The same endpoint must still refuse a request without its key.
#[tokio::test]
async fn api_created_a2a_endpoint_still_requires_its_key() {
    let server = TestServer::in_memory().await;
    let endpoint = create_a2a_endpoint(&server, "session_per_invocation").await;
    let wrong = A2aEndpoint {
        id: endpoint.id.clone(),
        api_key: "evra2a_wrong".to_string(),
    };
    rpc(
        &server,
        &wrong,
        vec![],
        json!({ "jsonrpc": "2.0", "id": 1, "method": "message/send", "params": {} }),
    )
    .await
    .assert_status(StatusCode::UNAUTHORIZED);
}
