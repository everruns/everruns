//! Integration tests for the App api_endpoint channel — app-scoped,
//! execution-only API keys driving native session routes.

mod test_harness;

use axum::http::{Method, StatusCode};
use everruns_core::DEFAULT_ORG_ID;
use everruns_server::storage::models::AuditLogQuery;
use serde_json::{Value, json};
use test_harness::TestServer;
use tokio::time::{Duration, sleep};

/// Create an app with an api_endpoint channel; returns (app_json_after, api_key).
async fn create_app_with_api_endpoint(server: &TestServer, name: &str) -> (Value, String) {
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
            &format!("/v1/apps/{app_id}/api-endpoint-channels"),
            json!({ "session_mode": "shared_session" }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let api_key = response["api_key"].as_str().unwrap().to_string();
    // The generated key is prefix-scoped and returned exactly once.
    assert!(api_key.starts_with("evr_app_"), "key was {api_key}");
    // The persisted channel must never echo the key hash on read.
    assert!(response["channel"]["channel_config"]["api_key_hash"].is_null());

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
async fn api_endpoint_create_session_dispatches_message_and_confines() {
    let server = TestServer::in_memory().await;
    let (app, api_key) = create_app_with_api_endpoint(&server, "api-endpoint-basic").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    // Create a session via the execution key.
    let created: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/api/{channel_id}/sessions"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            serde_json::to_vec(&json!({ "message": "hello agent" })).unwrap(),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session_id = created["session_id"].as_str().unwrap().to_string();
    assert_eq!(created["created_session"], true);

    // The caller-supplied message lands in the session verbatim (no template).
    let texts = list_user_message_texts(&server, &session_id).await;
    assert!(texts.iter().any(|t| t == "hello agent"), "texts: {texts:?}");

    // A follow-up message into the same session is accepted.
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/api/{channel_id}/sessions/{session_id}/messages"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            serde_json::to_vec(&json!({ "message": "follow up" })).unwrap(),
        )
        .await
        .assert_status(StatusCode::ACCEPTED);
    let texts = list_user_message_texts(&server, &session_id).await;
    assert!(texts.iter().any(|t| t == "follow up"), "texts: {texts:?}");

    // GET returns a derived status and a (projected) messages array.
    let status: Value = server
        .request_raw(
            Method::GET,
            &format!("/v1/apps/{app_id}/api/{channel_id}/sessions/{session_id}"),
            vec![("authorization", &format!("Bearer {api_key}"))],
            Vec::new(),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(status["session_id"].as_str().unwrap(), session_id);
    assert!(status["status"].is_string());
    assert!(status["messages"].is_array());
}

#[tokio::test]
async fn api_endpoint_rejects_missing_and_wrong_key() {
    let server = TestServer::in_memory().await;
    let (app, _api_key) = create_app_with_api_endpoint(&server, "api-endpoint-auth").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    let path = format!("/v1/apps/{app_id}/api/{channel_id}/sessions");
    let body = serde_json::to_vec(&json!({ "message": "hi" })).unwrap();

    // Missing key.
    server
        .request_raw(
            Method::POST,
            &path,
            vec![("content-type", "application/json")],
            body.clone(),
        )
        .await
        .assert_status(StatusCode::UNAUTHORIZED);

    // Wrong key.
    server
        .request_raw(
            Method::POST,
            &path,
            vec![
                ("content-type", "application/json"),
                ("authorization", "Bearer evr_app_wrong"),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn api_endpoint_unpublished_app_is_forbidden() {
    let server = TestServer::in_memory().await;
    let (app, api_key) = create_app_with_api_endpoint(&server, "api-endpoint-draft").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    // Intentionally not published.

    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/api/{channel_id}/sessions"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            serde_json::to_vec(&json!({ "message": "hi" })).unwrap(),
        )
        .await
        .assert_status(StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn api_endpoint_key_cannot_reach_another_apps_session() {
    let server = TestServer::in_memory().await;
    let (app_a, key_a) = create_app_with_api_endpoint(&server, "api-endpoint-a").await;
    let (app_b, key_b) = create_app_with_api_endpoint(&server, "api-endpoint-b").await;
    let app_a_id = app_a["id"].as_str().unwrap();
    let chan_a = app_a["channels"][0]["id"].as_str().unwrap();
    let app_b_id = app_b["id"].as_str().unwrap();
    let chan_b = app_b["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_a_id).await;
    publish_app(&server, app_b_id).await;

    // Create a session under app A.
    let created: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_a_id}/api/{chan_a}/sessions"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {key_a}")),
            ],
            serde_json::to_vec(&json!({ "message": "secret" })).unwrap(),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let session_a = created["session_id"].as_str().unwrap();

    // App B's key cannot read app A's session — confined to its own app/channel.
    server
        .request_raw(
            Method::GET,
            &format!("/v1/apps/{app_b_id}/api/{chan_b}/sessions/{session_a}"),
            vec![("authorization", &format!("Bearer {key_b}"))],
            Vec::new(),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);

    // Nor post a message into it.
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_b_id}/api/{chan_b}/sessions/{session_a}/messages"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {key_b}")),
            ],
            serde_json::to_vec(&json!({ "message": "intrude" })).unwrap(),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn api_endpoint_regenerate_key_invalidates_previous() {
    let server = TestServer::in_memory().await;
    let (app, old_key) = create_app_with_api_endpoint(&server, "api-endpoint-rotate").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    let rotated: Value = server
        .post(
            &format!("/v1/apps/{app_id}/api-endpoint-channels/{channel_id}/regenerate-key"),
            json!({}),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    let new_key = rotated["api_key"].as_str().unwrap().to_string();
    assert_ne!(new_key, old_key);

    let path = format!("/v1/apps/{app_id}/api/{channel_id}/sessions");
    let body = serde_json::to_vec(&json!({ "message": "hi" })).unwrap();

    // Old key no longer works.
    server
        .request_raw(
            Method::POST,
            &path,
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {old_key}")),
            ],
            body.clone(),
        )
        .await
        .assert_status(StatusCode::UNAUTHORIZED);

    // New key works.
    server
        .request_raw(
            Method::POST,
            &path,
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {new_key}")),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::CREATED);
}

#[tokio::test]
async fn api_endpoint_emits_app_invocation_audit_log() {
    let server = TestServer::in_memory().await;
    let (app, api_key) = create_app_with_api_endpoint(&server, "api-endpoint-audit").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    let created: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/api/{channel_id}/sessions"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            serde_json::to_vec(&json!({ "message": "audit me" })).unwrap(),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let session_id = created["session_id"].as_str().unwrap();

    let mut found = None;
    for _ in 0..20 {
        let rows = server
            .db
            .list_audit_logs(AuditLogQuery {
                org_id: DEFAULT_ORG_ID,
                limit: 50,
                action: Some("agent.app_invocation.started"),
                ..Default::default()
            })
            .await
            .expect("list audit logs");
        if let Some(row) = rows.into_iter().find(|row| {
            row.target_id.as_deref() == Some(channel_id)
                && row.metadata.get("session_id").and_then(Value::as_str) == Some(session_id)
        }) {
            found = Some(row);
            break;
        }
        sleep(Duration::from_millis(25)).await;
    }
    let audit = found.expect("expected app invocation audit log");
    assert_eq!(audit.action, "agent.app_invocation.started");
    assert_eq!(audit.actor_id, None);
    assert_eq!(audit.metadata["source"], "app_api_endpoint");
    assert_eq!(audit.metadata["app_channel_type"], "api_endpoint");
}
