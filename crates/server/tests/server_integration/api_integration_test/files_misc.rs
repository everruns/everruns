//! API integration tests: files misc.

use super::support::*;
use crate::test_harness;
use axum::http::StatusCode;
use everruns_core::SessionFile;
use everruns_platform::Agent;
use everruns_platform::Session;
use serde_json::{Value, json};
use test_harness::TestServer;

#[tokio::test]
async fn test_execute_btw_returns_ephemeral_response() {
    let server = TestServer::in_memory().await;
    let agent = create_llmsim_agent(&server, "btw-execute").await;

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_generic_harness_id,
                "agent_id": agent.public_id,
                "title": "BTW execute session"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let messages_before: Value = server
        .get(&format!("/v1/sessions/{}/messages", session.id))
        .await
        .assert_status(StatusCode::OK)
        .json();

    let result: Value = server
        .post(
            &format!("/v1/sessions/{}/commands/execute", session.id),
            json!({
                "name": "btw",
                "arguments": "What are you doing?"
            }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();

    let messages_after: Value = server
        .get(&format!("/v1/sessions/{}/messages", session.id))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(result["success"], Value::Bool(true));
    assert_eq!(
        result["message"],
        Value::String("Hello! I'm a simulated LLM response.".to_string())
    );
    assert_eq!(
        messages_before["data"].as_array().map(Vec::len),
        messages_after["data"].as_array().map(Vec::len)
    );
}

// ============================================
// Copy Agent Tests
// ============================================

#[tokio::test]
async fn test_list_capabilities() {
    let server = TestServer::new().await;

    let data: Value = server
        .get("/v1/capabilities")
        .await
        .assert_status(StatusCode::OK)
        .json();

    let capabilities = data["data"].as_array().expect("Expected array");
    // Should have built-in capabilities
    assert!(
        !capabilities.is_empty(),
        "Should have built-in capabilities"
    );
    assert!(
        capabilities
            .iter()
            .any(|cap| cap["id"] == "infinity_context"),
        "Should expose infinity_context as a standard capability"
    );
}

#[tokio::test]
async fn test_capability_info_includes_features() {
    let server = TestServer::new().await;

    // Check that the capabilities endpoint returns features
    let body: Value = server
        .get("/v1/capabilities")
        .await
        .assert_success()
        .json_value();

    let data = body["data"].as_array().expect("data array");

    // Find session_storage capability — should have secrets and key_value features
    let storage = data
        .iter()
        .find(|c| c["id"] == "session_storage")
        .expect("session_storage capability should exist");

    let features = storage["features"]
        .as_array()
        .expect("features should be an array");
    assert!(
        features.contains(&json!("secrets")),
        "session_storage should have secrets feature",
    );
    assert!(
        features.contains(&json!("key_value")),
        "session_storage should have key_value feature",
    );

    // Find session_schedule — should have schedules feature
    let schedule = data
        .iter()
        .find(|c| c["id"] == "session_schedule")
        .expect("session_schedule capability should exist");

    let schedule_features = schedule["features"]
        .as_array()
        .expect("features should be an array");
    assert!(
        schedule_features.contains(&json!("schedules")),
        "session_schedule should have schedules feature",
    );

    // noop is a test fixture (everruns-test-support) and must not appear in
    // the product capability registry.
    assert!(
        !data.iter().any(|c| c["id"] == "noop"),
        "noop fixture capability must not be registered in the product",
    );
}

// ============================================
// Retired Global Chat Session Routes
// ============================================

/// The per-user singleton chat session and its voice sibling are retired
/// (EVE-855). Chats binds each thread to an agent through the ordinary session
/// routes, so the singleton had no caller left; these assertions keep the paths
/// from being reintroduced by accident.
#[tokio::test]
async fn test_retired_global_chat_routes_are_gone() {
    // Routing only — no storage is touched, so the in-memory server is the
    // cheaper and equally conclusive harness here.
    let server = TestServer::in_memory().await;

    // 405, not 404: `/v1/sessions/chat` still matches `/v1/sessions/{session_id}`,
    // which serves GET/PATCH/DELETE but no POST. What matters is that no handler
    // resolves the singleton any more.
    server
        .post("/v1/sessions/chat", json!({}))
        .await
        .assert_status(StatusCode::METHOD_NOT_ALLOWED);

    // No `{session_id}` route matches this shape, so the voice sibling is a
    // plain 404.
    server
        .post("/v1/sessions/chat/voice", json!({ "sdp": "v=0" }))
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_update_profile_name() {
    let server = TestServer::in_memory().await;

    // Update profile name
    let resp: Value = server
        .patch("/v1/users/me", json!({ "name": "New Display Name" }))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(resp["name"], "New Display Name");
    assert!(resp["id"].is_string());
    assert!(resp["email"].is_string());
}

#[tokio::test]
async fn test_update_profile_trims_whitespace() {
    let server = TestServer::in_memory().await;

    let resp: Value = server
        .patch("/v1/users/me", json!({ "name": "  Trimmed Name  " }))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(resp["name"], "Trimmed Name");
}

#[tokio::test]
async fn test_update_profile_rejects_empty_name() {
    let server = TestServer::in_memory().await;

    server
        .patch("/v1/users/me", json!({ "name": "" }))
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_update_profile_rejects_whitespace_only_name() {
    let server = TestServer::in_memory().await;

    server
        .patch("/v1/users/me", json!({ "name": "   " }))
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_update_profile_rejects_too_long_name() {
    let server = TestServer::in_memory().await;

    let long_name = "a".repeat(256);
    server
        .patch("/v1/users/me", json!({ "name": long_name }))
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

// ============================================
// Readonly File Deletion Tests
// ============================================

#[tokio::test]
async fn test_cannot_delete_readonly_file() {
    let server = TestServer::in_memory().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "readonly-delete-test",
                "display_name": "Readonly Delete Test",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent.public_id
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let fs_url = format!("/v1/sessions/{}/fs", session.id);

    // Create a readonly file
    let _file: SessionFile = server
        .post(
            &format!("{}/protected.txt", fs_url),
            json!({
                "content": "Do not delete me",
                "is_readonly": true
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Attempt to delete should fail with 403 Forbidden
    server
        .delete(&format!("{}/protected.txt", fs_url))
        .await
        .assert_status(StatusCode::FORBIDDEN);

    // File should still exist
    let file: SessionFile = server
        .get(&format!("{}/protected.txt", fs_url))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(file.content.as_deref(), Some("Do not delete me"));
}

#[tokio::test]
async fn test_cannot_recursively_delete_directory_with_readonly_file() {
    let server = TestServer::in_memory().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "readonly-recursive-delete-test",
                "display_name": "Readonly Recursive Delete Test",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent.public_id
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let fs_url = format!("/v1/sessions/{}/fs", session.id);

    // Create directory with a readonly file inside
    let _dir: SessionFile = server
        .post(&format!("{}/docs", fs_url), json!({ "is_directory": true }))
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let _file: SessionFile = server
        .post(
            &format!("{}/docs/readme.txt", fs_url),
            json!({
                "content": "Protected content",
                "is_readonly": true
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Recursive delete of parent directory should fail with 403 Forbidden
    server
        .delete(&format!("{}/docs?recursive=true", fs_url))
        .await
        .assert_status(StatusCode::FORBIDDEN);

    // Readonly file should still exist
    let file: SessionFile = server
        .get(&format!("{}/docs/readme.txt", fs_url))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(file.content.as_deref(), Some("Protected content"));
}

#[tokio::test]
async fn test_can_delete_non_readonly_file() {
    let server = TestServer::in_memory().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "normal-delete-test",
                "display_name": "Normal Delete Test",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent.public_id
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let fs_url = format!("/v1/sessions/{}/fs", session.id);

    // Create a normal (non-readonly) file
    let _file: SessionFile = server
        .post(
            &format!("{}/temp.txt", fs_url),
            json!({
                "content": "Delete me"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Should succeed
    let result: Value = server
        .delete(&format!("{}/temp.txt", fs_url))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(result["deleted"], true);
}

// ============================================
// User Connection Verify Endpoint Tests
// ============================================

#[tokio::test]
async fn test_verify_connectors_listed() {
    let server = TestServer::in_memory().await;

    let resp: Value = server
        .get("/v1/user/connections/providers")
        .await
        .assert_status(StatusCode::OK)
        .json();

    // Should be an array (may include plugin-registered providers)
    assert!(resp.is_array());
}

// ============================================
// App Reference Validation Tests
// ============================================

#[tokio::test]
async fn test_export_user_data() {
    let server = TestServer::in_memory().await;

    let resp: Value = server
        .get("/v1/users/me/export")
        .await
        .assert_status(StatusCode::OK)
        .json();

    // Verify export structure
    assert!(resp["user"]["id"].is_string());
    assert!(resp["user"]["email"].is_string());
    assert!(resp["user"]["name"].is_string());
    assert!(resp["user"]["created_at"].is_string());
    assert!(resp["organizations"].is_array());
    assert!(resp["personal_access_tokens"].is_array());
    assert!(resp["exported_at"].is_string());
    // Verify no sensitive fields
    assert!(resp["user"].get("password_hash").is_none());
    assert!(resp["user"].get("roles").is_none());
}

#[tokio::test]
async fn test_delete_user_account() {
    let server = TestServer::in_memory().await;

    // Delete account
    let resp: Value = server
        .delete("/v1/users/me")
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(resp["deleted"], true);

    // After deletion, export should fail with 404
    server
        .get("/v1/users/me/export")
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_image_ids_round_trip_across_upload_list_and_get() {
    let server = TestServer::in_memory().await;
    let boundary = "----everruns-image-upload";
    let content_type = format!("multipart/form-data; boundary={boundary}");
    let image_bytes = vec![0x89, 0x50, 0x4E, 0x47];
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\n\
Content-Disposition: form-data; name=\"file\"; filename=\"upload.png\"\r\n\
Content-Type: image/png\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(&image_bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let upload: Value = server
        .request_raw(
            axum::http::Method::POST,
            "/v1/images",
            vec![("content-type", content_type.as_str())],
            body,
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let image_id = upload["id"].as_str().expect("upload image id");
    assert!(
        image_id.starts_with("img_"),
        "upload should return public image id, got {image_id}"
    );

    let listed: Vec<Value> = server
        .get("/v1/images")
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(listed.len(), 1, "expected uploaded image to be listed");
    assert_eq!(listed[0]["id"], upload["id"]);

    server
        .get(&format!("/v1/images/{image_id}"))
        .await
        .assert_status(StatusCode::OK);
}

// ============================================
// Environment Tests
// ============================================

#[tokio::test]
async fn test_environment_targets_say_why_an_absent_target_is_absent() {
    let server = TestServer::in_memory().await;

    let targets: Value = server
        .get("/v1/environment-targets")
        .await
        .assert_status(StatusCode::OK)
        .json();

    let items = targets["items"].as_array().expect("items array");
    let bashkit = items
        .iter()
        .find(|target| target["provider"] == "bashkit")
        .expect("bashkit is always available");
    assert_eq!(bashkit["available"], true);
    assert_eq!(bashkit["capabilities"]["native_processes"], false);

    let host = items
        .iter()
        .find(|target| target["kind"] == "host")
        .expect("host is listed even when unavailable");
    assert_eq!(host["available"], false);
    assert!(
        host["reason"].as_str().is_some_and(|r| !r.is_empty()),
        "an absent target owes a reason"
    );
    // Nothing may promise recovery from hardware Everruns does not own.
    assert_eq!(host["durability"], "none");
}

#[tokio::test]
async fn test_org_cannot_set_a_platform_managed_feature_flag() {
    let server = TestServer::in_memory().await;
    let org_id = "org_00000000000000000000000000000001";

    // Enabling and disabling are both refused: an org that cannot enrol itself
    // must not be able to unenrol either.
    for wanted in [true, false] {
        let refused: Value = server
            .patch(
                &format!("/v1/orgs/{org_id}/feature-flags"),
                json!({ "flags": { "environments": wanted } }),
            )
            .await
            .assert_status(StatusCode::BAD_REQUEST)
            .json();
        assert!(
            refused["detail"]
                .as_str()
                .is_some_and(|d| d.contains("managed by the platform")),
            "unexpected body: {refused}"
        );
    }

    // And it is absent from the tenant settings catalog, so no toggle exists.
    let settings: Value = server
        .get(&format!("/v1/orgs/{org_id}/feature-flags/settings"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let flags = settings["flags"].as_array().expect("flags array");
    assert!(flags.iter().all(|flag| flag["name"] != "environments"));
}
