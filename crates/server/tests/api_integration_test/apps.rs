//! API integration tests: apps.

use crate::test_harness;
use axum::http::StatusCode;
use everruns_core::DEFAULT_ORG_ID;
use everruns_provider::typed_id::{AppId, HarnessId, PrincipalId};
use everruns_server::storage::models::{CreateAppRow, CreatePrincipalRow};
use serde_json::{Value, json};
use test_harness::TestServer;
use uuid::Uuid;

#[tokio::test]
async fn test_verify_connection_no_connection_returns_404() {
    let server = TestServer::in_memory().await;

    server
        .post("/v1/user/connections/nonexistent/verify", json!({}))
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn app_archival_reads_remain_available() {
    let server = TestServer::in_memory().await;
    let principal_id = PrincipalId::new();
    server
        .db
        .create_principal(CreatePrincipalRow {
            id: principal_id,
            org_id: DEFAULT_ORG_ID,
            kind: "system".to_string(),
            subject_id: Some(Uuid::now_v7()),
            parent_principal_id: None,
            resolved_user_id: None,
            metadata: json!({}),
        })
        .await
        .expect("create archival App owner");
    let app_id = AppId::new();
    server
        .db
        .create_app(
            DEFAULT_ORG_ID,
            CreateAppRow {
                public_id: app_id.to_string(),
                name: "Archived management record".to_string(),
                description: Some("Retained for read-only access".to_string()),
                harness_id: server
                    .seed_generic_harness_id
                    .parse::<HarnessId>()
                    .expect("generic harness ID")
                    .uuid(),
                agent_id: None,
                agent_version_policy: "draft".to_string(),
                agent_version_id: None,
                agent_identity_id: None,
                owner_principal_id: principal_id,
                resolved_owner_user_id: None,
                channel_type: None,
                channel_config: json!({}),
                channel_config_encrypted: None,
            },
        )
        .await
        .expect("seed archival App");

    let list: Value = server
        .get("/v1/apps")
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(list["data"][0]["id"], app_id.to_string());
    assert_eq!(list["data"][0]["name"], "Archived management record");

    let detail: Value = server
        .get(&format!("/v1/apps/{app_id}"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(detail["id"], app_id.to_string());
    assert_eq!(detail["description"], "Retained for read-only access");
}

#[tokio::test]
async fn app_management_routes_are_retired() {
    let server = TestServer::in_memory().await;
    let app_id = AppId::new();
    let channel_id = "channel_019d166cd0147e638c72892ecb30ffff";

    server
        .post("/v1/apps", json!({}))
        .await
        .assert_status(StatusCode::METHOD_NOT_ALLOWED);
    server
        .patch(&format!("/v1/apps/{app_id}"), json!({}))
        .await
        .assert_status(StatusCode::METHOD_NOT_ALLOWED);
    server
        .delete(&format!("/v1/apps/{app_id}"))
        .await
        .assert_status(StatusCode::METHOD_NOT_ALLOWED);
    for path in [
        format!("/v1/apps/{app_id}/publish"),
        format!("/v1/apps/{app_id}/unpublish"),
        format!("/v1/apps/{app_id}/run"),
        format!("/v1/apps/{app_id}/runs"),
        format!("/v1/apps/{app_id}/channels"),
        format!("/v1/apps/{app_id}/channels/{channel_id}"),
    ] {
        server
            .post(&path, json!({}))
            .await
            .assert_status(StatusCode::NOT_FOUND);
        server
            .patch(&path, json!({}))
            .await
            .assert_status(StatusCode::NOT_FOUND);
        server
            .delete(&path)
            .await
            .assert_status(StatusCode::NOT_FOUND);
    }
}

#[tokio::test]
async fn public_chat_endpoint_route_matches_legacy_alias() {
    let server = TestServer::in_memory().await;
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": "public-chat-route-agent",
                "display_name": "Public Chat route agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let app = server
        .seed_app_endpoint(
            "Public Chat route",
            agent["id"].as_str().expect("agent ID"),
            "public_chat",
            json!({
                "anonymous": true,
                "branding": { "display_name": "Public Chat alias" }
            }),
        )
        .await;
    let app_id = app["id"].as_str().expect("App ID");
    let channel_id = app["channels"][0]["id"].as_str().expect("channel ID");
    server.set_app_endpoints_live(app_id, true).await;

    let endpoint_response = server
        .get(&format!("/v1/e/{channel_id}/public-chat/config"))
        .await;
    let legacy_response = server
        .get(&format!("/v1/apps/{app_id}/public-chat/config"))
        .await;
    let endpoint: Value = endpoint_response.assert_status(StatusCode::OK).json();
    let legacy: Value = legacy_response.assert_status(StatusCode::OK).json();

    assert_eq!(endpoint, legacy);
    assert_eq!(endpoint["app_id"], app_id);
    assert_eq!(endpoint["name"], "Public Chat alias");
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
