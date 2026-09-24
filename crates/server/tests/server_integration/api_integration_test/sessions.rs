//! API integration tests: sessions.

use super::support::*;
use crate::test_harness;
use axum::http::StatusCode;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use everruns_core::{SessionContextReport, SessionFile};
use everruns_platform::Agent;
use everruns_platform::Harness;
use everruns_platform::Session;
use serde_json::{Value, json};
use test_harness::TestServer;

#[tokio::test]
async fn test_create_session() {
    let server = TestServer::new().await;

    // Create an agent first
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "session-test-agent",
                "display_name": "Session Test Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Create a session
    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent.public_id,
                "title": "Test Session",
                "locale": "uk-UA"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert_eq!(session.agent_id, Some(agent.public_id));
    assert_eq!(session.title.as_deref(), Some("Test Session"));
    assert_eq!(session.locale.as_deref(), Some("uk-UA"));

    let fetched: Session = server
        .get(&format!("/v1/sessions/{}", session.id))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(fetched.locale.as_deref(), Some("uk-UA"));
}

#[tokio::test]
async fn test_session_secret_lifecycle_is_write_only_and_reserves_internal_names() {
    let server = TestServer::in_memory().await;
    let sentinel = "session-secret-must-never-be-returned";
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "session-secret-agent",
                "display_name": "Session Secret Agent",
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
                "agent_id": agent.public_id,
                "title": "Session secret lifecycle"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let secrets_url = format!("/v1/sessions/{}/storage/secrets", session.id);

    let stored: Value = server
        .put(
            &secrets_url,
            json!({ "secrets": { "DISPOSABLE_KEY": sentinel } }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(stored["count"], 1);
    assert!(!stored.to_string().contains(sentinel));

    let listed: Value = server
        .get(&secrets_url)
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(listed["data"][0]["name"], "DISPOSABLE_KEY");
    assert!(listed["data"][0].get("value").is_none());
    assert!(!listed.to_string().contains(sentinel));

    server
        .put(
            &secrets_url,
            json!({ "secrets": { "mcp_oauth:server:access_token": "blocked" } }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);

    server
        .delete(&format!("{secrets_url}/DISPOSABLE_KEY"))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    let empty: Value = server
        .get(&secrets_url)
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(empty["data"], json!([]));
}

#[tokio::test]
async fn test_create_session_nonexistent_model_returns_404() {
    let server = TestServer::new().await;

    server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
                "model_id": "model_00000000000000000000000000000000",
                "title": "Should fail"
            }),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_get_session_schedule_wrong_parent_returns_not_found() {
    let server = TestServer::in_memory().await;
    let session = create_schedule_test_session(&server, "Schedule Owner").await;
    let other_session = create_schedule_test_session(&server, "Wrong Parent").await;
    let schedule_id = seed_session_schedule(&server, &session).await;

    let body: Value = server
        .get(&format!(
            "/v1/sessions/{}/schedules/{}",
            other_session.id, schedule_id
        ))
        .await
        .assert_status(StatusCode::NOT_FOUND)
        .json();

    assert_eq!(body["detail"], "Schedule not found");
}

#[tokio::test]
async fn test_update_session_schedule_wrong_parent_returns_not_found() {
    let server = TestServer::in_memory().await;
    let session = create_schedule_test_session(&server, "Schedule Owner").await;
    let other_session = create_schedule_test_session(&server, "Wrong Parent").await;
    let schedule_id = seed_session_schedule(&server, &session).await;

    let body: Value = server
        .patch(
            &format!(
                "/v1/sessions/{}/schedules/{}",
                other_session.id, schedule_id
            ),
            json!({ "enabled": false }),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND)
        .json();
    assert_eq!(body["detail"], "Schedule not found");

    let persisted = server
        .db
        .get_session_schedule(1, schedule_id)
        .await
        .unwrap();
    assert!(persisted.unwrap().enabled);
}

#[tokio::test]
async fn test_delete_session_schedule_wrong_parent_returns_not_found() {
    let server = TestServer::in_memory().await;
    let session = create_schedule_test_session(&server, "Schedule Owner").await;
    let other_session = create_schedule_test_session(&server, "Wrong Parent").await;
    let schedule_id = seed_session_schedule(&server, &session).await;

    let body: Value = server
        .delete(&format!(
            "/v1/sessions/{}/schedules/{}",
            other_session.id, schedule_id
        ))
        .await
        .assert_status(StatusCode::NOT_FOUND)
        .json();
    assert_eq!(body["detail"], "Schedule not found");

    let persisted = server
        .db
        .get_session_schedule(1, schedule_id)
        .await
        .unwrap();
    assert!(persisted.is_some());
}

#[tokio::test]
async fn test_trigger_session_schedule_wrong_parent_returns_not_found() {
    let server = TestServer::in_memory().await;
    let session = create_schedule_test_session(&server, "Schedule Owner").await;
    let other_session = create_schedule_test_session(&server, "Wrong Parent").await;
    let schedule_id = seed_session_schedule(&server, &session).await;

    let body: Value = server
        .post(
            &format!(
                "/v1/sessions/{}/schedules/{}/trigger",
                other_session.id, schedule_id
            ),
            json!({}),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND)
        .json();
    assert_eq!(body["detail"], "Schedule not found");

    let persisted = server
        .db
        .get_session_schedule(1, schedule_id)
        .await
        .unwrap();
    let persisted = persisted.unwrap();
    assert_eq!(persisted.trigger_count, 0);
    assert!(persisted.last_triggered_at.is_none());
}

#[tokio::test]
async fn test_get_session() {
    let server = TestServer::new().await;

    // Create agent and session
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "get-session-test-agent",
                "display_name": "Get Session Test Agent",
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
                "agent_id": agent.public_id,
                "title": "Get Test Session"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Get the session
    let fetched_session: Session = server
        .get(&format!("/v1/sessions/{}", session.id))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(fetched_session.id, session.id);
}

#[tokio::test]
async fn test_get_session_context_report_without_generation() {
    let server = TestServer::in_memory().await;

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
                "title": "Context Report Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let report: SessionContextReport = server
        .get(&format!("/v1/sessions/{}/context-report", session.id))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(report.session_id, session.id.to_string());
    assert_eq!(report.estimated_input_tokens, 0);
    assert!(report.sections.is_empty());
}

#[tokio::test]
async fn test_sessions_pagination() {
    let server = TestServer::new().await;

    // Create an agent
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "pagination-test-agent",
                "display_name": "Pagination Test Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Create 15 sessions
    for i in 1..=15 {
        let _: Session = server
            .post(
                "/v1/sessions",
                json!({
                    "harness_id": server.seed_base_harness_id,
                    "agent_id": agent.public_id,
                    "title": format!("Session {}", i)
                }),
            )
            .await
            .assert_status(StatusCode::CREATED)
            .json();
    }

    // Test default pagination
    let body: Value = server
        .get(&format!("/v1/sessions?agent_id={}", agent.public_id))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(body["total"], 15);
    assert_eq!(body["offset"], 0);

    // Test custom limit
    let body: Value = server
        .get(&format!(
            "/v1/sessions?agent_id={}&limit=5",
            agent.public_id
        ))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(body["limit"], 5);
    assert_eq!(body["data"].as_array().unwrap().len(), 5);

    // Test offset pagination
    let body: Value = server
        .get(&format!(
            "/v1/sessions?agent_id={}&offset=10&limit=10",
            agent.public_id
        ))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(body["data"].as_array().unwrap().len(), 5); // Only 5 remaining
}

#[tokio::test]
async fn test_session_filesystem() {
    let server = TestServer::new().await;

    // Create agent and session
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "filesystem-test-agent",
                "display_name": "Filesystem Test Agent",
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

    // List root (new sessions mount scoped memory under the reserved memory root)
    let data: Value = server
        .get(&fs_url)
        .await
        .assert_status(StatusCode::OK)
        .json();
    let root_entries = data["data"].as_array().unwrap();
    assert_eq!(root_entries.len(), 1);
    assert_eq!(root_entries[0]["path"], "/memory");
    assert_eq!(root_entries[0]["is_directory"], true);

    // Create a file
    let file: SessionFile = server
        .post(
            &format!("{}/hello.txt", fs_url),
            json!({
                "content": "Hello, World!",
                "encoding": "text"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    assert_eq!(file.path, "/hello.txt");

    // Read the file
    let file: SessionFile = server
        .get(&format!("{}/hello.txt", fs_url))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(file.content.as_deref(), Some("Hello, World!"));

    // Update the file
    let file: SessionFile = server
        .put(
            &format!("{}/hello.txt", fs_url),
            json!({
                "content": "Updated content"
            }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(file.content.as_deref(), Some("Updated content"));

    // Create directory
    let dir: SessionFile = server
        .post(
            &format!("{}/docs", fs_url),
            json!({
                "is_directory": true
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    assert!(dir.is_directory);

    // Delete file
    let result: Value = server
        .delete(&format!("{}/hello.txt", fs_url))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(result["deleted"], true);
}

#[tokio::test]
async fn test_session_filesystem_list_nonexistent_directory_returns_404() {
    let server = TestServer::new().await;

    // Create agent and session
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "fs-404-test-agent",
                "display_name": "FS 404 Test Agent",
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

    // List a directory that doesn't exist — should return 404, not 500
    let resp = server.get(&format!("{}/.agents/skills", fs_url)).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_session_file_download_path_returns_raw_bytes() {
    let server = TestServer::in_memory().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "fs-download-agent",
                "display_name": "FS Download Agent",
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
    let pdf_bytes = b"%PDF-1.7\n%\x80\x81\x82\x83\n".to_vec();

    server
        .post(
            &format!("{}/report.pdf", fs_url),
            json!({
                "content": BASE64.encode(&pdf_bytes),
                "encoding": "base64"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    let json_read: SessionFile = server
        .get(&format!("{}/report.pdf", fs_url))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(json_read.encoding, "base64");
    assert_eq!(
        json_read.content.as_deref(),
        Some(BASE64.encode(&pdf_bytes).as_str())
    );

    let raw = server
        .get(&format!("{}/_/download/report.pdf", fs_url))
        .await
        .assert_status(StatusCode::OK);

    assert_eq!(
        raw.headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/pdf")
    );
    assert_eq!(
        raw.headers()
            .get("content-disposition")
            .and_then(|value| value.to_str().ok()),
        Some("attachment; filename=\"report.pdf\"; filename*=UTF-8''report.pdf")
    );
    assert_eq!(raw.bytes(), pdf_bytes.as_slice());
}

#[tokio::test]
async fn test_session_file_read_accept_octet_stream_returns_raw_bytes() {
    let server = TestServer::in_memory().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "fs-negotiate-agent",
                "display_name": "FS Negotiate Agent",
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
    let raw_bytes = vec![0, 159, 146, 150];

    server
        .post(
            &format!("{}/payload.bin", fs_url),
            json!({
                "content": BASE64.encode(&raw_bytes),
                "encoding": "base64"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    let response = server
        .request_raw(
            axum::http::Method::GET,
            &format!("{}/payload.bin", fs_url),
            vec![("accept", "application/octet-stream")],
            vec![],
        )
        .await
        .assert_status(StatusCode::OK);

    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/octet-stream")
    );
    assert_eq!(
        response
            .headers()
            .get("content-disposition")
            .and_then(|value| value.to_str().ok()),
        Some("inline; filename=\"payload.bin\"; filename*=UTF-8''payload.bin")
    );
    assert_eq!(response.bytes(), raw_bytes.as_slice());
}

#[tokio::test]
async fn test_session_file_read_accept_octet_stream_q_zero_keeps_json_response() {
    let server = TestServer::in_memory().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "fs-negotiate-q-zero-agent",
                "display_name": "FS Negotiate Q Zero Agent",
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
    let raw_bytes = vec![0, 159, 146, 150];

    server
        .post(
            &format!("{}/payload.bin", fs_url),
            json!({
                "content": BASE64.encode(&raw_bytes),
                "encoding": "base64"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    let response = server
        .request_raw(
            axum::http::Method::GET,
            &format!("{}/payload.bin", fs_url),
            vec![("accept", "application/octet-stream;q=0, application/json")],
            vec![],
        )
        .await
        .assert_status(StatusCode::OK);

    let file: SessionFile = response.json();
    assert_eq!(file.encoding, "base64");
    assert_eq!(
        file.content.as_deref(),
        Some(BASE64.encode(&raw_bytes).as_str())
    );
}

#[tokio::test]
async fn test_session_file_download_path_rejects_directories() {
    let server = TestServer::in_memory().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "fs-download-dir-agent",
                "display_name": "FS Download Dir Agent",
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

    server
        .post(
            &format!("{}/docs", fs_url),
            json!({
                "is_directory": true
            }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    let response = server.get(&format!("{}/_/download/docs", fs_url)).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// ============================================
// Harness Tests
// ============================================

#[tokio::test]
async fn test_session_commands_are_scoped_to_active_capabilities() {
    let server = TestServer::in_memory().await;

    let generic_session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_generic_harness_id,
                "title": "Generic command session"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let base_session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
                "title": "Base command session"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let generic_commands: Value = server
        .get(&format!("/v1/sessions/{}/commands", generic_session.id))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let base_commands: Value = server
        .get(&format!("/v1/sessions/{}/commands", base_session.id))
        .await
        .assert_status(StatusCode::OK)
        .json();

    let generic_names: Vec<&str> = generic_commands["commands"]
        .as_array()
        .expect("generic commands")
        .iter()
        .filter_map(|command| command["name"].as_str())
        .collect();
    let base_names: Vec<&str> = base_commands["commands"]
        .as_array()
        .expect("base commands")
        .iter()
        .filter_map(|command| command["name"].as_str())
        .collect();

    assert!(generic_names.contains(&"btw"));
    assert!(!base_names.contains(&"btw"));
}

#[tokio::test]
async fn test_session_databases_crud() {
    let server = TestServer::new().await;

    // Create agent + session for the test
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "sql-test-agent",
                "display_name": "SQL Test Agent",
                "system_prompt": "Test",
                "capabilities": [{"ref": "session_sql_database", "config": {}}]
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
                "agent_id": agent.public_id.to_string()
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session_id = session.id.to_string();

    // List databases (should be empty)
    let list: Value = server
        .get(&format!("/v1/sessions/{session_id}/databases"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(list["data"].as_array().unwrap().len(), 0);

    // Create a database
    let db_info: Value = server
        .post(
            &format!("/v1/sessions/{session_id}/databases"),
            json!({"name": "analytics"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    assert_eq!(db_info["name"], "analytics");
    assert_eq!(db_info["size_bytes"], 0);

    // Get database
    let db_info: Value = server
        .get(&format!("/v1/sessions/{session_id}/databases/analytics"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(db_info["name"], "analytics");

    // List databases (should have 1)
    let list: Value = server
        .get(&format!("/v1/sessions/{session_id}/databases"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(list["data"].as_array().unwrap().len(), 1);

    // Create duplicate (should conflict)
    server
        .post(
            &format!("/v1/sessions/{session_id}/databases"),
            json!({"name": "analytics"}),
        )
        .await
        .assert_status(StatusCode::CONFLICT);

    // Get nonexistent database (should 404)
    server
        .get(&format!("/v1/sessions/{session_id}/databases/nope"))
        .await
        .assert_status(StatusCode::NOT_FOUND);

    // Delete database
    server
        .delete(&format!("/v1/sessions/{session_id}/databases/analytics"))
        .await
        .assert_status(StatusCode::NO_CONTENT);

    // Delete again (should 404)
    server
        .delete(&format!("/v1/sessions/{session_id}/databases/analytics"))
        .await
        .assert_status(StatusCode::NOT_FOUND);

    // List should be empty again
    let list: Value = server
        .get(&format!("/v1/sessions/{session_id}/databases"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(list["data"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_session_databases_schema() {
    let server = TestServer::new().await;

    // Create agent + session
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "sql-schema-agent",
                "display_name": "SQL Schema Agent",
                "system_prompt": "Test",
                "capabilities": [{"ref": "session_sql_database", "config": {}}]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({"harness_id": server.seed_base_harness_id, "agent_id": agent.public_id.to_string()}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session_id = session.id.to_string();

    // Create a database
    server
        .post(
            &format!("/v1/sessions/{session_id}/databases"),
            json!({"name": "test_db"}),
        )
        .await
        .assert_status(StatusCode::CREATED);

    // Schema of empty database should have no tables
    let schema: Value = server
        .get(&format!(
            "/v1/sessions/{session_id}/databases/test_db/schema"
        ))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(schema["database"], "test_db");
    assert_eq!(schema["tables"].as_array().unwrap().len(), 0);

    // Schema of nonexistent database should 404
    server
        .get(&format!("/v1/sessions/{session_id}/databases/nope/schema"))
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_session_databases_invalid_name() {
    let server = TestServer::new().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "sql-invalid-name-agent",
                "display_name": "SQL Invalid Name Agent",
                "system_prompt": "Test",
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({"harness_id": server.seed_base_harness_id, "agent_id": agent.public_id.to_string()}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session_id = session.id.to_string();

    // Invalid name (starts with number)
    server
        .post(
            &format!("/v1/sessions/{session_id}/databases"),
            json!({"name": "1bad"}),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);

    // Invalid name (contains dash)
    server
        .post(
            &format!("/v1/sessions/{session_id}/databases"),
            json!({"name": "my-db"}),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_session_databases_limit_exceeded_returns_422() {
    let server = TestServer::in_memory().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "sql-limit-agent",
                "display_name": "SQL Limit Agent",
                "system_prompt": "Test",
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({"harness_id": server.seed_base_harness_id, "agent_id": agent.public_id.to_string()}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session_id = session.id.to_string();

    for i in 0..10 {
        server
            .post(
                &format!("/v1/sessions/{session_id}/databases"),
                json!({ "name": format!("db_{i}") }),
            )
            .await
            .assert_status(StatusCode::CREATED);
    }

    server
        .post(
            &format!("/v1/sessions/{session_id}/databases"),
            json!({ "name": "db_overflow" }),
        )
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
}

// ============================================================================
// Session Features Tests
// ============================================================================

#[tokio::test]
async fn test_session_features_persisted_in_get() {
    let server = TestServer::new().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({"name": "get-features-agent", "display_name": "Get Features Agent", "system_prompt": "Test"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_generic_harness_id,
                "agent_id": agent.public_id,
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Features should also appear in GET response
    let fetched: Session = server
        .get(&format!("/v1/sessions/{}", session.id))
        .await
        .assert_success()
        .json();

    assert_eq!(
        fetched.features, session.features,
        "GET session should return same features as POST",
    );
}

#[tokio::test]
async fn test_session_features_in_list() {
    let server = TestServer::new().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({"name": "list-features-agent", "display_name": "List Features Agent", "system_prompt": "Test"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let _session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_generic_harness_id,
                "agent_id": agent.public_id,
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Features should appear in list response
    let list_body: Value = server
        .get(&format!("/v1/sessions?agent_id={}", agent.public_id))
        .await
        .assert_success()
        .json_value();

    let data = list_body["data"].as_array().expect("data array");
    assert!(!data.is_empty());

    let first = &data[0];
    let features = first["features"]
        .as_array()
        .expect("features should be an array");
    assert!(
        features.contains(&json!("file_system")),
        "List should include file_system feature",
    );
}

#[tokio::test]
async fn test_session_features_with_session_capabilities() {
    let server = TestServer::new().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({"name": "session-cap-features-agent", "display_name": "Session Cap Features Agent", "system_prompt": "Test"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Base harness has no caps, but add session_schedule at session level
    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent.public_id,
                "capabilities": [{"ref": "session_schedule"}],
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert!(
        session.features.contains(&"schedules".to_string()),
        "Session-level capability should contribute features, got: {:?}",
        session.features,
    );
}

#[tokio::test]
async fn test_session_features_sql_database() {
    let server = TestServer::new().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({"name": "sqldb-features-agent", "display_name": "SqlDb Features Agent", "system_prompt": "Test"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Add session_sql_database via session-level capability
    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent.public_id,
                "capabilities": [{"ref": "session_sql_database"}],
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert!(
        session.features.contains(&"sql_database".to_string()),
        "session_sql_database should contribute sql_database feature, got: {:?}",
        session.features,
    );
}

#[tokio::test]
async fn test_delete_entities_referenced_only_by_session_succeeds() {
    let server = TestServer::in_memory().await;

    let harness: Harness = server
        .post(
            "/v1/harnesses",
            json!({
                "name": "session-only-delete-harness",
                "display_name": "Session Only Delete Harness",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": "session-only-delete-agent",
                "display_name": "Session Only Delete Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let identity: Value = server
        .post(
            "/v1/agent-identities",
            json!({"name": "Session Only Delete Identity"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": harness.id,
                "agent_id": agent["id"],
                "agent_identity_id": identity["id"],
                "title": "Session-only references"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    server
        .delete(&format!("/v1/agents/{}", agent["id"].as_str().unwrap()))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    server
        .delete(&format!("/v1/harnesses/{}", harness.id))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    server
        .delete(&format!(
            "/v1/agent-identities/{}",
            identity["id"].as_str().unwrap()
        ))
        .await
        .assert_status(StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn test_session_environment_reports_what_the_session_can_actually_do() {
    let server = TestServer::in_memory().await;

    let session: Value = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_generic_harness_id,
                "title": "Environment smoke",
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let session_id = session["id"].as_str().expect("session id");

    let environment: Value = server
        .get(&format!("/v1/sessions/{session_id}/environment"))
        .await
        .assert_status(StatusCode::OK)
        .json();

    // The generic harness carries `bashkit_shell`, which is a virtual
    // filesystem rather than a machine.
    assert_eq!(environment["target"]["kind"], "vfs");
    assert_eq!(environment["target"]["provider"], "bashkit");
    assert_eq!(environment["source_capability"], "bashkit_shell");

    // The load-bearing claim: a caller learns a build cannot run here before
    // running one, rather than from a confusing tool error afterwards.
    assert_eq!(environment["capabilities"]["native_processes"], false);
    assert_eq!(environment["capabilities"]["portable_checkpoint"], true);
    assert_eq!(environment["containment"]["level"], "isolated");
    assert_eq!(environment["durability"], "checkpointed");

    // Says how it knows, rather than implying a stored profile that does not
    // exist yet.
    assert_eq!(environment["resolved_from"], "capabilities");
}
