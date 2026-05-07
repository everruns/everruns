//! Integration tests for the App A2A (Agent2Agent) channel.

mod test_harness;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use test_harness::TestServer;

async fn create_app_with_a2a(server: &TestServer, name: &str, message: &str) -> (Value, String) {
    let app: Value = server
        .post(
            "/v1/apps",
            json!({
                "name": name,
                "harness_id": server.seed_generic_harness_id.clone(),
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let app_id = app["id"].as_str().unwrap().to_string();
    let response: Value = server
        .post(
            &format!("/v1/apps/{app_id}/a2a-channels"),
            json!({
                "session_mode": "shared_session",
                "message": message,
                "agent_card_name": "Inbox triage",
                "agent_card_description": "Triages inbound A2A traffic",
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let api_key = response["api_key"].as_str().unwrap().to_string();
    let app_after: Value = server
        .get(&format!("/v1/apps/{app_id}"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    (app_after, api_key)
}

async fn publish_app(server: &TestServer, app_id: &str) {
    server
        .post(&format!("/v1/apps/{app_id}/publish"), json!({}))
        .await
        .assert_status(StatusCode::OK);
}

async fn list_user_message_texts(server: &TestServer, session_id: &str) -> Vec<String> {
    let body: Value = server
        .get(&format!("/v1/sessions/{session_id}/messages"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    body["data"]
        .as_array()
        .expect("messages array")
        .iter()
        .filter(|m| m["role"].as_str() == Some("user"))
        .filter_map(|m| {
            m["content"].as_array().and_then(|parts| {
                parts.iter().find_map(|p| {
                    (p["type"].as_str() == Some("text"))
                        .then(|| p["text"].as_str().map(str::to_owned))
                        .flatten()
                })
            })
        })
        .collect()
}

#[tokio::test]
async fn a2a_message_send_creates_session_and_returns_completed_task() {
    let server = TestServer::in_memory().await;
    let (app, api_key) = create_app_with_a2a(&server, "a2a-shared", "from a2a: {{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "req-1",
        "method": "message/send",
        "params": {
            "message": {
                "role": "user",
                "messageId": "msg-1",
                "parts": [{ "kind": "text", "text": "hello" }]
            }
        }
    }))
    .unwrap();

    let response: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            body.clone(),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], "req-1");
    assert_eq!(response["result"]["status"]["state"], "completed");
    assert_eq!(response["result"]["kind"], "task");
    let session_id = response["result"]["contextId"].as_str().unwrap();
    let texts = list_user_message_texts(&server, session_id).await;
    assert!(texts.iter().any(|t| t == "from a2a: hello"));

    // Second invocation in shared_session reuses the same session.
    let body2 = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "req-2",
        "method": "message/send",
        "params": {
            "message": {
                "role": "user",
                "messageId": "msg-2",
                "parts": [{ "kind": "text", "text": "again" }]
            }
        }
    }))
    .unwrap();
    let response2: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            body2,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    // Second invocation must land in the same shared session — `contextId`
    // echoes the Everruns SessionId and shared_session mode reuses one
    // session for the channel.
    assert_eq!(
        response2["result"]["contextId"].as_str().unwrap(),
        session_id,
        "shared_session A2A invocations must reuse the same session",
    );
    let texts = list_user_message_texts(&server, session_id).await;
    assert!(texts.iter().any(|t| t == "from a2a: hello"));
    assert!(texts.iter().any(|t| t == "from a2a: again"));
}

#[tokio::test]
async fn a2a_rejects_missing_or_invalid_api_key() {
    let server = TestServer::in_memory().await;
    let (app, api_key) = create_app_with_a2a(&server, "a2a-auth", "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "message/send",
        "params": {
            "message": { "role": "user", "parts": [{ "kind": "text", "text": "hi" }] }
        }
    }))
    .unwrap();

    // No auth header.
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![("content-type", "application/json")],
            body.clone(),
        )
        .await
        .assert_status(StatusCode::UNAUTHORIZED);

    // Wrong key.
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", "Bearer evra2a_wrong"),
            ],
            body.clone(),
        )
        .await
        .assert_status(StatusCode::UNAUTHORIZED);

    // Right key still works.
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::OK);
}

#[tokio::test]
async fn a2a_rejects_unpublished_or_disabled() {
    let server = TestServer::in_memory().await;
    let (app, api_key) = create_app_with_a2a(&server, "a2a-pub", "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();

    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "message/send",
        "params": {
            "message": { "role": "user", "parts": [{ "kind": "text", "text": "hi" }] }
        }
    }))
    .unwrap();

    // Unpublished: 403.
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            body.clone(),
        )
        .await
        .assert_status(StatusCode::FORBIDDEN);

    publish_app(&server, app_id).await;

    server
        .patch(
            &format!("/v1/apps/{app_id}/channels/{channel_id}"),
            json!({ "enabled": false }),
        )
        .await
        .assert_status(StatusCode::OK);

    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a2a_rejects_unsupported_methods_and_empty_text() {
    let server = TestServer::in_memory().await;
    let (app, api_key) = create_app_with_a2a(&server, "a2a-method", "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    // message/stream is not supported.
    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "x",
        "method": "message/stream",
        "params": {
            "message": { "role": "user", "parts": [{ "kind": "text", "text": "hi" }] }
        }
    }))
    .unwrap();
    let response: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(response["error"]["code"], -32601);

    // Empty parts.
    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "y",
        "method": "message/send",
        "params": {
            "message": { "role": "user", "parts": [] }
        }
    }))
    .unwrap();
    let response: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(response["error"]["code"], -32602);
}

#[tokio::test]
async fn a2a_agent_card_published_only_when_live() {
    let server = TestServer::in_memory().await;
    let (app, _api_key) = create_app_with_a2a(&server, "a2a-card", "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    let card_path = format!("/v1/apps/{app_id}/a2a/{channel_id}/.well-known/agent-card.json");

    // Draft -> 404.
    server
        .request_raw(Method::GET, &card_path, vec![], vec![])
        .await
        .assert_status(StatusCode::NOT_FOUND);

    publish_app(&server, app_id).await;

    let card: Value = server
        .request_raw(
            Method::GET,
            &card_path,
            vec![("host", "example.test")],
            vec![],
        )
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(card["name"], "Inbox triage");
    assert_eq!(card["protocolVersion"], "0.3.0");
    assert_eq!(card["preferredTransport"], "JSONRPC");
    assert!(
        card["url"]
            .as_str()
            .unwrap()
            .ends_with(&format!("/v1/apps/{app_id}/a2a/{channel_id}"))
    );
    assert_eq!(card["capabilities"]["streaming"], false);
    // Card never echoes secrets.
    let serialized = serde_json::to_string(&card).unwrap();
    assert!(!serialized.contains("api_key"));
    assert!(!serialized.contains("api_key_hash"));

    // Disable channel -> 404.
    server
        .patch(
            &format!("/v1/apps/{app_id}/channels/{channel_id}"),
            json!({ "enabled": false }),
        )
        .await
        .assert_status(StatusCode::OK);

    server
        .request_raw(Method::GET, &card_path, vec![], vec![])
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a2a_patch_preserves_api_key_when_omitted() {
    // PATCH on an A2A channel must preserve the server-managed api_key_hash
    // and api_key_prefix even when the client only sends the user-editable
    // fields (message, session_mode, agent card metadata). Otherwise an edit
    // would silently break authentication for previously issued keys.
    let server = TestServer::in_memory().await;
    let (app, api_key) = create_app_with_a2a(&server, "a2a-preserve", "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    server
        .patch(
            &format!("/v1/apps/{app_id}/channels/{channel_id}"),
            json!({
                "channel_config": {
                    "session_mode": "session_per_invocation",
                    "message": "edited {{a2a.text}}",
                    "agent_card_name": "Edited",
                }
            }),
        )
        .await
        .assert_status(StatusCode::OK);

    // The originally issued API key still works after the edit.
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            serde_json::to_vec(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "message/send",
                "params": {
                    "message": { "role": "user", "parts": [{ "kind": "text", "text": "post-edit" }] }
                }
            }))
            .unwrap(),
        )
        .await
        .assert_status(StatusCode::OK);
}

#[tokio::test]
async fn a2a_regenerate_key_invalidates_previous_key() {
    let server = TestServer::in_memory().await;
    let (app, original_key) = create_app_with_a2a(&server, "a2a-rotate", "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    // Original key works.
    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "message/send",
        "params": {
            "message": { "role": "user", "parts": [{ "kind": "text", "text": "hi" }] }
        }
    }))
    .unwrap();
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {original_key}")),
            ],
            body.clone(),
        )
        .await
        .assert_status(StatusCode::OK);

    // Rotate.
    let rotated: Value = server
        .post(
            &format!("/v1/apps/{app_id}/a2a-channels/{channel_id}/regenerate-key"),
            json!({}),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    let new_key = rotated["api_key"].as_str().unwrap().to_string();
    assert_ne!(new_key, original_key);

    // Original key fails.
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {original_key}")),
            ],
            body.clone(),
        )
        .await
        .assert_status(StatusCode::UNAUTHORIZED);

    // New key works.
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {new_key}")),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::OK);
}
