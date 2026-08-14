//! Integration tests for attaching a session to an existing shared workspace
//! via `CreateSession { workspace_id }`, and for the file-store re-keying that
//! makes the attached workspace's files visible through the session-scoped
//! filesystem alias. Runs on the in-memory backend (no PostgreSQL).

mod test_harness;

use axum::http::StatusCode;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU64, Ordering};
use test_harness::TestServer;

fn unique(prefix: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!("{prefix}-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

async fn create_workspace(server: &TestServer) -> String {
    let ws: Value = server
        .post("/v1/workspaces", json!({ "name": unique("shared-ws") }))
        .await
        .assert_status(StatusCode::CREATED)
        .json_value();
    ws["id"].as_str().expect("workspace id").to_string()
}

async fn create_session_attached(server: &TestServer, workspace_id: &str) -> Value {
    server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
                "workspace_id": workspace_id,
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json_value()
}

#[tokio::test]
async fn attached_sessions_share_the_workspace_filesystem() {
    let server = TestServer::in_memory().await;
    let wsp = create_workspace(&server).await;

    // Seed a file directly into the shared workspace.
    server
        .post(
            &format!("/v1/workspaces/{wsp}/fs/shared/notes.txt"),
            json!({ "content": "hello shared", "encoding": "text" }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    // Two distinct sessions attach to the same workspace.
    let a = create_session_attached(&server, &wsp).await;
    let b = create_session_attached(&server, &wsp).await;
    assert_eq!(a["workspace_id"].as_str(), Some(wsp.as_str()));
    assert_eq!(b["workspace_id"].as_str(), Some(wsp.as_str()));
    assert_ne!(a["id"], b["id"], "sessions must be distinct");

    let sid_a = a["id"].as_str().unwrap();
    let sid_b = b["id"].as_str().unwrap();

    // Each session sees the workspace-seeded file via its session-fs alias
    // (proves the alias resolves session -> workspace, not session id).
    for sid in [sid_a, sid_b] {
        let file = server
            .get(&format!("/v1/sessions/{sid}/fs/shared/notes.txt"))
            .await
            .assert_status(StatusCode::OK)
            .json_value();
        assert_eq!(
            file["content"].as_str(),
            Some("hello shared"),
            "session {sid} should see the shared workspace file"
        );
    }

    // A file written through session A's alias is visible through session B's
    // alias — they share one underlying workspace keyspace.
    server
        .post(
            &format!("/v1/sessions/{sid_a}/fs/from_a.txt"),
            json!({ "content": "written by A", "encoding": "text" }),
        )
        .await
        .assert_status(StatusCode::CREATED);
    let via_b = server
        .get(&format!("/v1/sessions/{sid_b}/fs/from_a.txt"))
        .await
        .assert_status(StatusCode::OK)
        .json_value();
    assert_eq!(via_b["content"].as_str(), Some("written by A"));

    // And it is the same file the workspace API exposes.
    let via_ws = server
        .get(&format!("/v1/workspaces/{wsp}/fs/from_a.txt"))
        .await
        .assert_status(StatusCode::OK)
        .json_value();
    assert_eq!(via_ws["content"].as_str(), Some("written by A"));
}

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
async fn create_session_rejects_unknown_workspace() {
    let server = TestServer::in_memory().await;
    server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
                "workspace_id": "wsp_ffffffffffffffffffffffffffffffff",
            }),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn create_session_rejects_legacy_workspace_with_mismatched_id() {
    use everruns_core::DEFAULT_ORG_ID;
    use everruns_provider::typed_id::WorkspaceId;
    use everruns_server::storage::models::CreateWorkspaceRow;

    let server = TestServer::in_memory().await;
    // Simulate a workspace created before the `id.hex == public_id` invariant:
    // a random internal PK unrelated to the public id.
    let public = WorkspaceId::new();
    let internal = uuid::Uuid::now_v7();
    assert_ne!(internal, public.uuid());
    server
        .db
        .create_workspace(
            DEFAULT_ORG_ID,
            CreateWorkspaceRow {
                id: Some(internal),
                public_id: public.to_string(),
                name: unique("legacy-ws"),
                description: None,
                owner_principal_id: None,
                resolved_owner_user_id: None,
            },
        )
        .await
        .expect("seed legacy workspace");

    // Attaching to it is rejected (the session would otherwise report a
    // non-round-tripping workspace_id).
    server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
                "workspace_id": public.to_string(),
            }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_session_rejects_archived_workspace() {
    let server = TestServer::in_memory().await;
    let wsp = create_workspace(&server).await;

    // Archive the workspace.
    server
        .delete(&format!("/v1/workspaces/{wsp}"))
        .await
        .assert_success();

    // Attaching a new session to an archived workspace is rejected.
    server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
                "workspace_id": wsp,
            }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}
