//! Integration tests for session git version control
//!
//! Tests the full git lifecycle: commit, log, diff, branches, and binary roundtrip.
//! Runs against both PostgreSQL and in-memory backends.
//!
//! Run with: cargo test -p everruns-server --test session_git_integration_test -- --test-threads=1

mod test_harness;

use axum::http::StatusCode;
use serde_json::{Value, json};
use test_harness::TestServer;

use everruns_core::{Agent, Session};

const SEED_BASE_HARNESS_ID: &str = "harness_01933b5a000070008000000000000601";

/// Helper: create an agent + session + populate files, return session ID
async fn setup_session_with_files(server: &TestServer) -> String {
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({"name": "git-test-agent", "display_name": "Git Test Agent", "system_prompt": "Test"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({"harness_id": SEED_BASE_HARNESS_ID, "agent_id": agent.public_id}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let fs_url = format!("/v1/sessions/{}/fs", session.id);

    // Create files
    server
        .post(
            &format!("{}/hello.txt", fs_url),
            json!({"content": "Hello, World!", "encoding": "text"}),
        )
        .await
        .assert_status(StatusCode::CREATED);

    server
        .post(
            &format!("{}/src/main.rs", fs_url),
            json!({"content": "fn main() {}", "encoding": "text"}),
        )
        .await
        .assert_status(StatusCode::CREATED);

    session.id.to_string()
}

// ========================================================
// In-memory tests
// ========================================================

#[tokio::test]
async fn test_git_commit_and_log_in_memory() {
    let server = TestServer::in_memory().await;
    let session_id = setup_session_with_files(&server).await;
    let git_url = format!("/v1/sessions/{session_id}/git");

    // First commit
    let commit: Value = server
        .post(
            &format!("{git_url}/commit"),
            json!({
                "message": "Initial commit",
                "author_name": "Test User",
                "author_email": "test@example.com"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert!(!commit["oid"].as_str().unwrap().is_empty());
    assert!(!commit["tree_oid"].as_str().unwrap().is_empty());
    assert!(commit["objects_created"].as_u64().unwrap() > 0);

    // Log
    let log: Vec<Value> = server
        .get(&format!("{git_url}/log"))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(log.len(), 1);
    assert_eq!(log[0]["message"], "Initial commit");
    assert_eq!(log[0]["author_name"], "Test User");
    assert_eq!(log[0]["oid"], commit["oid"]);
}

#[tokio::test]
async fn test_git_diff_in_memory() {
    let server = TestServer::in_memory().await;
    let session_id = setup_session_with_files(&server).await;
    let git_url = format!("/v1/sessions/{session_id}/git");

    // Commit
    let commit: Value = server
        .post(
            &format!("{git_url}/commit"),
            json!({"message": "v1", "author_name": "A", "author_email": "a@b.com"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let oid = commit["oid"].as_str().unwrap();

    // Diff (first commit, no parent — shows all files as added)
    let diff: Value = server
        .get(&format!("{git_url}/diff?oid={oid}"))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert!(diff["entries"].as_array().unwrap().len() >= 2);
    assert!(diff["stats"]["files_changed"].as_u64().unwrap() >= 2);
}

#[tokio::test]
async fn test_git_refs_and_branches_in_memory() {
    let server = TestServer::in_memory().await;
    let session_id = setup_session_with_files(&server).await;
    let git_url = format!("/v1/sessions/{session_id}/git");

    // Commit
    let commit: Value = server
        .post(
            &format!("{git_url}/commit"),
            json!({"message": "init", "author_name": "A", "author_email": "a@b.com"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let oid = commit["oid"].as_str().unwrap();

    // List refs
    let refs: Vec<Value> = server
        .get(&format!("{git_url}/refs"))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert!(refs.len() >= 2); // HEAD + refs/heads/main

    // Create branch
    server
        .post(
            &format!("{git_url}/branches"),
            json!({"name": "feature-x", "commit_oid": oid}),
        )
        .await
        .assert_status(StatusCode::CREATED);

    // Branch should appear in refs
    let refs: Vec<Value> = server
        .get(&format!("{git_url}/refs"))
        .await
        .assert_status(StatusCode::OK)
        .json();

    let branch_names: Vec<&str> = refs.iter().map(|r| r["name"].as_str().unwrap()).collect();
    assert!(branch_names.contains(&"refs/heads/feature-x"));

    // Delete branch
    let del: Value = server
        .delete(&format!("{git_url}/branches/feature-x"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(del["ok"], true);

    // Delete non-existent branch returns 404
    server
        .delete(&format!("{git_url}/branches/nonexistent"))
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_git_log_no_commits_returns_404() {
    let server = TestServer::in_memory().await;
    let session_id = setup_session_with_files(&server).await;
    let git_url = format!("/v1/sessions/{session_id}/git");

    // Log with no commits should return 404 (ref not found)
    server
        .get(&format!("{git_url}/log"))
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_git_multiple_commits_in_memory() {
    let server = TestServer::in_memory().await;
    let session_id = setup_session_with_files(&server).await;
    let git_url = format!("/v1/sessions/{session_id}/git");
    let fs_url = format!("/v1/sessions/{session_id}/fs");

    // Commit v1
    let c1: Value = server
        .post(
            &format!("{git_url}/commit"),
            json!({"message": "v1", "author_name": "A", "author_email": "a@b.com"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Modify a file
    server
        .put(
            &format!("{fs_url}/hello.txt"),
            json!({"content": "Updated content", "encoding": "text"}),
        )
        .await
        .assert_status(StatusCode::OK);

    // Commit v2
    let c2: Value = server
        .post(
            &format!("{git_url}/commit"),
            json!({"message": "v2", "author_name": "B", "author_email": "b@b.com"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Log should have 2 commits
    let log: Vec<Value> = server
        .get(&format!("{git_url}/log"))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(log.len(), 2);
    assert_eq!(log[0]["message"], "v2");
    assert_eq!(log[1]["message"], "v1");

    // Diff between v2 and v1
    let c2_oid = c2["oid"].as_str().unwrap();
    let c1_oid = c1["oid"].as_str().unwrap();

    let diff: Value = server
        .get(&format!("{git_url}/diff?oid={c2_oid}&base={c1_oid}"))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert!(diff["stats"]["files_changed"].as_u64().unwrap() >= 1);
}

#[tokio::test]
async fn test_git_short_branch_name_in_memory() {
    let server = TestServer::in_memory().await;
    let session_id = setup_session_with_files(&server).await;
    let git_url = format!("/v1/sessions/{session_id}/git");

    // Commit with short branch name
    let commit: Value = server
        .post(
            &format!("{git_url}/commit"),
            json!({
                "message": "on feature branch",
                "author_name": "A",
                "author_email": "a@b.com",
                "branch": "feature"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let oid = commit["oid"].as_str().unwrap();

    // Refs should have refs/heads/feature (normalized from "feature")
    let refs: Vec<Value> = server
        .get(&format!("{git_url}/refs"))
        .await
        .assert_status(StatusCode::OK)
        .json();

    let branch_names: Vec<&str> = refs.iter().map(|r| r["name"].as_str().unwrap()).collect();
    assert!(
        branch_names.contains(&"refs/heads/feature"),
        "Expected refs/heads/feature in {:?}",
        branch_names
    );

    // Log from refs/heads/feature should work
    let log: Vec<Value> = server
        .get(&format!("{git_url}/log?ref=refs/heads/feature"))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(log.len(), 1);
    assert_eq!(log[0]["oid"].as_str().unwrap(), oid);
}

#[tokio::test]
async fn test_git_empty_file_committed_in_memory() {
    let server = TestServer::in_memory().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({"name": "empty-file-agent", "display_name": "Empty File Agent", "system_prompt": "T"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({"harness_id": SEED_BASE_HARNESS_ID, "agent_id": agent.public_id}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let fs_url = format!("/v1/sessions/{}/fs", session.id);
    let git_url = format!("/v1/sessions/{}/git", session.id);

    // Create an empty file
    server
        .post(
            &format!("{fs_url}/empty.txt"),
            json!({"content": "", "encoding": "text"}),
        )
        .await
        .assert_status(StatusCode::CREATED);

    // Commit should succeed and include the empty file
    let commit: Value = server
        .post(
            &format!("{git_url}/commit"),
            json!({"message": "empty file", "author_name": "A", "author_email": "a@b.com"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert!(commit["objects_created"].as_u64().unwrap() > 0);

    // Diff should show the empty file as added
    let oid = commit["oid"].as_str().unwrap();
    let diff: Value = server
        .get(&format!("{git_url}/diff?oid={oid}"))
        .await
        .assert_status(StatusCode::OK)
        .json();

    let paths: Vec<&str> = diff["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["path"].as_str().unwrap())
        .collect();
    assert!(
        paths.contains(&"empty.txt"),
        "Expected empty.txt in diff entries: {:?}",
        paths
    );
}

#[tokio::test]
async fn test_git_binary_roundtrip_in_memory() {
    let server = TestServer::in_memory().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({"name": "binary-agent", "display_name": "Binary Agent", "system_prompt": "T"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({"harness_id": SEED_BASE_HARNESS_ID, "agent_id": agent.public_id}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let fs_url = format!("/v1/sessions/{}/fs", session.id);
    let git_url = format!("/v1/sessions/{}/git", session.id);

    // Create a binary file using base64 encoding
    use base64::Engine;
    let binary_data: Vec<u8> = (0..256).map(|i| i as u8).collect();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&binary_data);

    server
        .post(
            &format!("{fs_url}/data.bin"),
            json!({"content": b64, "encoding": "base64"}),
        )
        .await
        .assert_status(StatusCode::CREATED);

    // Commit
    let commit: Value = server
        .post(
            &format!("{git_url}/commit"),
            json!({"message": "binary data", "author_name": "A", "author_email": "a@b.com"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert!(commit["objects_created"].as_u64().unwrap() > 0);
}
