//! Every session gets its own workspace. Attaching a session to an existing
//! workspace (`CreateSession { workspace_id }`) and creating standalone
//! workspaces are retired: the only gate was the org-wide workspace
//! permission, so a new session on any agent could be pointed at another
//! session's files. Runs on the in-memory backend (no PostgreSQL).

use crate::test_harness;

use axum::http::StatusCode;
use serde_json::{Value, json};
use test_harness::TestServer;

#[tokio::test]
async fn default_session_keeps_its_own_isolated_workspace() {
    let server = TestServer::in_memory().await;

    // Two plain sessions (no workspace_id) must NOT share files.
    let a: Value = server
        .post(
            "/v1/sessions",
            json!({ "harness_id": server.seed_base_harness_id }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json_value();
    let b: Value = server
        .post(
            "/v1/sessions",
            json!({ "harness_id": server.seed_base_harness_id }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json_value();

    // Each default session's workspace mirrors its own id.
    let sid_a = a["id"].as_str().unwrap();
    let sid_b = b["id"].as_str().unwrap();
    let hex_a = sid_a.strip_prefix("session_").unwrap();
    assert_eq!(
        a["workspace_id"].as_str(),
        Some(format!("wsp_{hex_a}").as_str())
    );
    assert_ne!(a["workspace_id"], b["workspace_id"]);

    server
        .post(
            &format!("/v1/sessions/{sid_a}/fs/private.txt"),
            json!({ "content": "only A", "encoding": "text" }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    // Session B must not see A's private file.
    server
        .get(&format!("/v1/sessions/{sid_b}/fs/private.txt"))
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn create_session_refuses_to_attach_another_sessions_workspace() {
    let server = TestServer::in_memory().await;
    let victim: Value = server
        .post(
            "/v1/sessions",
            json!({ "harness_id": server.seed_base_harness_id }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json_value();
    let victim_id = victim["id"].as_str().unwrap();
    server
        .post(
            &format!("/v1/sessions/{victim_id}/fs/secret.txt"),
            json!({ "content": "victim data", "encoding": "text" }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    // Naming the victim's workspace, an unknown one, or anything else in that
    // field is refused before a session exists.
    for workspace_id in [
        victim["workspace_id"].clone(),
        json!("wsp_ffffffffffffffffffffffffffffffff"),
        json!(null),
    ] {
        let body = json!({
            "harness_id": server.seed_base_harness_id,
            "workspace_id": workspace_id,
        });
        if workspace_id.is_null() {
            // An explicit null is the same as leaving the field out.
            server
                .post("/v1/sessions", body)
                .await
                .assert_status(StatusCode::CREATED);
            continue;
        }
        let refused: Value = server
            .post("/v1/sessions", body)
            .await
            .assert_status(StatusCode::BAD_REQUEST)
            .json_value();
        assert!(
            refused
                .to_string()
                .contains("workspace_id is no longer supported"),
            "{refused}"
        );
    }

    // The command surface goes through the same creation boundary.
    server
        .post(
            "/v1/commands/create_session",
            json!({ "params": {
                "harness_id": server.seed_base_harness_id,
                "workspace_id": victim["workspace_id"],
            }}),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn standalone_workspaces_cannot_be_created() {
    let server = TestServer::in_memory().await;
    server
        .post("/v1/workspaces", json!({ "name": "shared" }))
        .await
        .assert_status(StatusCode::METHOD_NOT_ALLOWED);
    server
        .post(
            "/v1/commands/create_workspace",
            json!({ "params": { "name": "shared" } }),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
}
