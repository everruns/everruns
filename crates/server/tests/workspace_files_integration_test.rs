//! Integration tests for the canonical workspace filesystem HTTP surface
//! (`/v1/workspaces/{workspace_id}/fs/*`).
//!
//! Covers the endpoints added on top of #2140's CRUD/list surface — move,
//! copy, raw download, and `Accept: application/octet-stream` raw GET — plus
//! the `workspace_id` field now returned on the session response. Runs against
//! the in-memory backend (no PostgreSQL required).

mod test_harness;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use test_harness::TestServer;

/// Create a session (which auto-creates its default 1:1 workspace) and return
/// the `(session_id, workspace_id)` pair from the response body.
async fn create_session_with_workspace(server: &TestServer) -> (String, String) {
    let session: Value = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
                "title": "workspace-fs-test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json_value();

    let session_id = session["id"].as_str().expect("session id").to_string();
    let workspace_id = session["workspace_id"]
        .as_str()
        .expect("workspace_id must be present on the session response")
        .to_string();
    (session_id, workspace_id)
}

#[tokio::test]
async fn session_response_exposes_workspace_id_mirroring_session_id() {
    let server = TestServer::in_memory().await;
    let (session_id, workspace_id) = create_session_with_workspace(&server).await;

    // Default 1:1 case: workspace.id == session.id, so the public ids share the
    // same 32-hex body with different prefixes.
    let hex = session_id
        .strip_prefix("session_")
        .expect("session id prefix");
    assert_eq!(workspace_id, format!("wsp_{hex}"));
}

#[tokio::test]
async fn workspace_fs_move_copy_download_roundtrip() {
    let server = TestServer::in_memory().await;
    let (_session_id, workspace_id) = create_session_with_workspace(&server).await;
    let base = format!("/v1/workspaces/{workspace_id}/fs");

    // Create a file.
    server
        .post(
            &format!("{base}/notes/a.txt"),
            json!({ "content": "hello world", "encoding": "text" }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    // Copy it (201 Created).
    server
        .post(
            &format!("{base}/_/copy"),
            json!({ "src_path": "notes/a.txt", "dst_path": "notes/b.txt" }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    // Move the copy (200 OK).
    server
        .post(
            &format!("{base}/_/move"),
            json!({ "src_path": "notes/b.txt", "dst_path": "notes/c.txt" }),
        )
        .await
        .assert_status(StatusCode::OK);

    // The move source is gone; the original and destination remain.
    server
        .get(&format!("{base}/notes/a.txt"))
        .await
        .assert_status(StatusCode::OK);
    server
        .get(&format!("{base}/notes/b.txt"))
        .await
        .assert_status(StatusCode::NOT_FOUND);

    // Raw download via the dedicated action route (Content-Disposition: attachment).
    let download = server
        .get(&format!("{base}/_/download/notes/c.txt"))
        .await
        .assert_status(StatusCode::OK);
    assert_eq!(download.bytes(), b"hello world");
    let disposition = download
        .headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(
        disposition.starts_with("attachment"),
        "expected attachment disposition, got: {disposition}"
    );

    // Raw bytes via Accept header on the plain GET route.
    let raw = server
        .request_raw(
            Method::GET,
            &format!("{base}/notes/c.txt"),
            vec![("accept", "application/octet-stream")],
            Vec::new(),
        )
        .await
        .assert_status(StatusCode::OK);
    assert_eq!(raw.bytes(), b"hello world");

    // Delete.
    server
        .delete(&format!("{base}/notes/c.txt"))
        .await
        .assert_status(StatusCode::OK);
}

#[tokio::test]
async fn workspace_fs_rejects_unknown_workspace() {
    let server = TestServer::in_memory().await;
    server
        .get("/v1/workspaces/wsp_ffffffffffffffffffffffffffffffff/fs")
        .await
        .assert_status(StatusCode::NOT_FOUND);
}
