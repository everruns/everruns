//! API integration tests: apps.

use crate::test_harness;
use axum::http::StatusCode;
use everruns_core::DEFAULT_ORG_ID;
use everruns_provider::typed_id::AppChannelId;
use everruns_server::storage::models::CreateAppChannelRow;
use serde_json::{Value, json};
use test_harness::TestServer;

#[tokio::test]
async fn test_verify_connection_no_connection_returns_404() {
    let server = TestServer::in_memory().await;

    server
        .post("/v1/user/connections/nonexistent/verify", json!({}))
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_app_trigger_channels_rejected_and_legacy_webhook_persists_in_postgres() {
    let server = TestServer::new().await;

    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": "app-invocation-postgres-agent",
                "display_name": "Invocation Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let schedule_response: Value = server
        .post(
            "/v1/apps",
            json!({
                "name": "Scheduled Repo Check",
                "harness_id": server.seed_generic_harness_id,
                "agent_id": agent["id"],
                "channel_type": "schedule",
                "channel_config": {
                    "cron_expression": "0 15 * * * * *",
                    "timezone": "UTC",
                    "session_mode": "shared_session",
                    "message": "check repo"
                }
            }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST)
        .json();

    assert!(
        schedule_response["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("schedule trigger on the app's agent"),
        "unexpected response: {schedule_response:?}"
    );

    let webhook_response: Value = server
        .post(
            "/v1/apps",
            json!({
                "name": "Webhook Repo Check",
                "harness_id": server.seed_generic_harness_id,
                "agent_id": agent["id"],
                "channel_type": "webhook",
                "channel_config": {
                    "token": "secret-token",
                    "session_mode": "session_per_invocation",
                    "message": "check webhook"
                }
            }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST)
        .json();
    assert!(
        webhook_response["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("webhook trigger on the app's agent"),
        "unexpected response: {webhook_response:?}"
    );

    let webhook_app: Value = server
        .post(
            "/v1/apps",
            json!({
                "name": "Webhook Repo Check",
                "harness_id": server.seed_generic_harness_id,
                "agent_id": agent["id"]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let webhook_app_row = server
        .db
        .get_app_by_public_id(
            DEFAULT_ORG_ID,
            webhook_app["id"].as_str().expect("webhook App ID"),
        )
        .await
        .expect("get webhook App")
        .expect("webhook App exists");
    server
        .db
        .create_app_channel(
            webhook_app_row.id,
            CreateAppChannelRow {
                public_id: AppChannelId::new().to_string(),
                channel_type: "webhook".to_string(),
                channel_config: json!({
                    "token": "secret-token",
                    "session_mode": "session_per_invocation",
                    "message": "check webhook"
                }),
                channel_config_encrypted: None,
                durable_schedule_id: None,
                enabled: true,
            },
        )
        .await
        .expect("seed legacy webhook channel");

    let stored_webhook_app: Value = server
        .get(&format!("/v1/apps/{}", webhook_app["id"].as_str().unwrap()))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(stored_webhook_app["channels"].as_array().unwrap().len(), 1);
    assert_eq!(stored_webhook_app["channels"][0]["channel_type"], "webhook");
    assert!(
        stored_webhook_app["channels"][0]["channel_config"]["token"].is_null(),
        "webhook token must not be returned"
    );
    assert_eq!(
        stored_webhook_app["channels"][0]["channel_config"]["token_configured"],
        true
    );
    assert_eq!(
        stored_webhook_app["channels"][0]["channel_config"]["session_mode"],
        "session_per_invocation"
    );
    assert_eq!(
        stored_webhook_app["channels"][0]["channel_config"]["message"],
        "check webhook"
    );
}

#[tokio::test]
async fn test_publish_app_without_channels_returns_bad_request() {
    let server = TestServer::new().await;

    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": "channel-less-app-agent",
                "display_name": "Channel-less App Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let app: Value = server
        .post(
            "/v1/apps",
            json!({
                "name": "Channel-less App",
                "harness_id": server.seed_generic_harness_id,
                "agent_id": agent["id"]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let response: Value = server
        .post(
            &format!("/v1/apps/{}/publish", app["id"].as_str().unwrap()),
            json!({}),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST)
        .json();

    assert!(
        response["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("at least one channel"),
        "unexpected response: {response:?}"
    );
}

#[tokio::test]
async fn test_update_app_to_published_returns_bad_request() {
    let server = TestServer::new().await;

    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": "patch-published-app-agent",
                "display_name": "Patch Published App Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let app: Value = server
        .post(
            "/v1/apps",
            json!({
                "name": "Patch Published App",
                "harness_id": server.seed_generic_harness_id,
                "agent_id": agent["id"]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let response: Value = server
        .patch(
            &format!("/v1/apps/{}", app["id"].as_str().unwrap()),
            json!({ "status": "published" }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST)
        .json();

    assert!(
        response["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("publish/unpublish endpoints"),
        "unexpected response: {response:?}"
    );
}

#[tokio::test]
async fn test_publish_archived_app_returns_bad_request() {
    let server = TestServer::new().await;

    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": "archived-publish-app-agent",
                "display_name": "Archived Publish App Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let app: Value = server
        .post(
            "/v1/apps",
            json!({
                "name": "Archived Publish App",
                "harness_id": server.seed_generic_harness_id,
                "agent_id": agent["id"]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    server
        .patch(
            &format!("/v1/apps/{}", app["id"].as_str().unwrap()),
            json!({ "status": "archived" }),
        )
        .await
        .assert_status(StatusCode::OK);

    let response: Value = server
        .post(
            &format!("/v1/apps/{}/publish", app["id"].as_str().unwrap()),
            json!({}),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST)
        .json();

    assert!(
        response["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("draft before publishing"),
        "unexpected response: {response:?}"
    );
}

#[tokio::test]
async fn test_update_app_reencrypts_legacy_plaintext_channel_configs() {
    let server = TestServer::new().await;

    let agent: Value = server
        .post(
            "/v1/agents",
            json!({ "name": "update-app-reencrypt-agent", "display_name": "Test Agent", "system_prompt": "Test" }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let app: Value = server
        .post(
            "/v1/apps",
            json!({
                "name": "Legacy plaintext app",
                "harness_id": server.seed_generic_harness_id,
                "agent_id": agent["id"],
                "channel_type": "slack",
                "channel_config": {
                    "bot_token": "xoxb-reencrypt",
                    "signing_secret": "signing-reencrypt"
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let channel_id = app["channels"][0]["id"].as_str().unwrap().to_string();
    let channel_row = server
        .db
        .get_app_channel_by_public_id(&channel_id)
        .await
        .unwrap()
        .unwrap();

    // Simulate legacy migrated row: plaintext secrets + NULL ciphertext.
    // UpdateAppChannel uses COALESCE and cannot clear channel_config_encrypted,
    // so write the legacy state directly.
    let pool = server
        .db
        .pool()
        .expect("Postgres pool required for this test");
    sqlx::query(
        "UPDATE app_channels \
         SET channel_config = $1, channel_config_encrypted = NULL \
         WHERE id = $2",
    )
    .bind(json!({
        "bot_token": "xoxb-plaintext",
        "signing_secret": "signing-plaintext"
    }))
    .bind(channel_row.id)
    .execute(pool)
    .await
    .unwrap();

    server
        .patch(
            &format!("/v1/apps/{}", app["id"].as_str().unwrap()),
            json!({ "description": "touch app to trigger opportunistic encryption" }),
        )
        .await
        .assert_status(StatusCode::OK);

    let updated_channel_row = server
        .db
        .get_app_channel_by_public_id(&channel_id)
        .await
        .unwrap()
        .unwrap();

    assert!(
        updated_channel_row.channel_config_encrypted.is_some(),
        "channel config should be encrypted after app update"
    );
    assert_eq!(updated_channel_row.channel_config, json!({}));
}

#[tokio::test]
async fn test_list_connections_initially_empty() {
    let server = TestServer::in_memory().await;

    let resp: Vec<Value> = server
        .get("/v1/user/connections")
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert!(resp.is_empty());
}

// ============================================================================
// Agent Identity Connection tests
// ============================================================================

#[tokio::test]
async fn test_identity_connections_list_empty() {
    let server = TestServer::in_memory().await;

    // Create an identity first
    let identity: Value = server
        .post("/v1/agent-identities", json!({"name": "ConnTest"}))
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let id = identity["id"].as_str().unwrap();

    // List connections — should be empty
    let connections: Vec<Value> = server
        .get(&format!("/v1/agent-identities/{id}/connections"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert!(connections.is_empty());
}

#[tokio::test]
async fn test_identity_connections_not_found_for_missing_identity() {
    let server = TestServer::in_memory().await;

    server
        .get("/v1/agent-identities/identity_019d166cd0147e638c72892ecb30ffff/connections")
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_identity_connections_delete_not_found() {
    let server = TestServer::in_memory().await;

    let identity: Value = server
        .post("/v1/agent-identities", json!({"name": "ConnDel"}))
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let id = identity["id"].as_str().unwrap();

    server
        .delete(&format!(
            "/v1/agent-identities/{id}/connections/nonexistent"
        ))
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_identity_connections_create_unknown_provider() {
    let server = TestServer::in_memory().await;

    let identity: Value = server
        .post("/v1/agent-identities", json!({"name": "ConnProv"}))
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let id = identity["id"].as_str().unwrap();

    server
        .post(
            &format!("/v1/agent-identities/{id}/connections/nonexistent"),
            json!({"api_key": "test"}),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

// ============================================
// Account Deletion & Data Export Tests
// ============================================
