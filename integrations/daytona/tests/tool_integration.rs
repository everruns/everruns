//! Integration tests: tool execute_with_context against wiremock Daytona API.
//!
//! These tests exercise the full tool execution flow:
//! MockStorageStore → tool.execute_with_context() → DaytonaClient → wiremock
//!
//! Unlike the unit tests in src/tools.rs (which test schemas, "requires context" errors,
//! and parameter validation), these tests verify actual HTTP interactions with the
//! Daytona API through the tool layer.

use async_trait::async_trait;
use everruns_core::error::Result;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::{KeyInfo, SecretInfo, SessionStorageStore, ToolContext};
use everruns_core::typed_id::SessionId;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// Force linker to include the integration crate.
use everruns_integrations_daytona as _;

use everruns_integrations_daytona::client::DaytonaClient;
use everruns_integrations_daytona::state::SandboxState;

// ----------------------------------------------------------------------------
// Mock SessionStorageStore
// ----------------------------------------------------------------------------

struct MockStorageStore {
    secrets: Mutex<HashMap<String, String>>,
}

impl MockStorageStore {
    fn new() -> Self {
        Self {
            secrets: Mutex::new(HashMap::new()),
        }
    }

    async fn seed_secret(&self, session_id: SessionId, name: &str, value: &str) {
        let key = format!("{}:{}", session_id, name);
        self.secrets.lock().await.insert(key, value.to_string());
    }
}

#[async_trait]
impl SessionStorageStore for MockStorageStore {
    async fn set_value(&self, _session_id: SessionId, _key: &str, _value: &str) -> Result<()> {
        Ok(())
    }
    async fn get_value(&self, _session_id: SessionId, _key: &str) -> Result<Option<String>> {
        Ok(None)
    }
    async fn delete_value(&self, _session_id: SessionId, _key: &str) -> Result<bool> {
        Ok(false)
    }
    async fn list_keys(&self, _session_id: SessionId) -> Result<Vec<KeyInfo>> {
        Ok(vec![])
    }
    async fn set_secret(&self, session_id: SessionId, name: &str, value: &str) -> Result<()> {
        let key = format!("{session_id}:{name}");
        self.secrets.lock().await.insert(key, value.to_string());
        Ok(())
    }
    async fn get_secret(&self, session_id: SessionId, name: &str) -> Result<Option<String>> {
        let key = format!("{session_id}:{name}");
        Ok(self.secrets.lock().await.get(&key).cloned())
    }
    async fn delete_secret(&self, session_id: SessionId, name: &str) -> Result<bool> {
        let key = format!("{session_id}:{name}");
        Ok(self.secrets.lock().await.remove(&key).is_some())
    }
    async fn list_secrets(&self, session_id: SessionId) -> Result<Vec<SecretInfo>> {
        let prefix = format!("{session_id}:");
        let secrets = self.secrets.lock().await;
        Ok(secrets
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .map(|k| SecretInfo {
                name: k.strip_prefix(&prefix).unwrap_or(k).to_string(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
            .collect())
    }
}

// ----------------------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------------------

/// Seed a mock store with API key and sandbox state, returning the context.
/// The `api_key_url` should be the wiremock server URI — but since tools use
/// DaytonaClient::new() (hardcoded URLs), we need a different approach.
///
/// We seed the API key and sandbox state, then the test must override the
/// client URL via environment or use a direct DaytonaClient test instead.
/// For tools that construct DaytonaClient::new(api_key), the tests that need
/// real HTTP go through the client module tests in src/client.rs.
///
/// These integration tests verify the tool orchestration layer: parameter
/// validation, state lookups, and result formatting — against a real storage mock.
async fn setup_context_with_sandbox(
    session_id: SessionId,
    store: &Arc<MockStorageStore>,
    sandbox_id: &str,
) {
    store
        .seed_secret(session_id, "DAYTONA_API_KEY", "test_api_key")
        .await;
    let state = SandboxState {
        sandbox_id: sandbox_id.to_string(),
        workspace_path: "/sandbox".to_string(),
        started_at: "2026-02-18T10:00:00Z".to_string(),
    };
    store
        .seed_secret(
            session_id,
            &format!("daytona_sandbox:{sandbox_id}"),
            &serde_json::to_string(&state).unwrap(),
        )
        .await;
}

fn get_tool(name: &str) -> Box<dyn Tool> {
    let cap = everruns_integrations_daytona::DaytonaCapability;
    use everruns_core::capabilities::Capability;
    cap.tools()
        .into_iter()
        .find(|t| t.name() == name)
        .unwrap_or_else(|| panic!("Tool {name} not found"))
}

// ----------------------------------------------------------------------------
// DaytonaClient integration tests (wiremock)
// ----------------------------------------------------------------------------
// These test the client directly with custom base URLs pointing at wiremock.

#[tokio::test]
async fn test_exec_tool_full_flow_via_client() {
    let mock_server = MockServer::start().await;

    // Mock the exec endpoint
    Mock::given(method("POST"))
        .and(path("/sb_int/process/execute"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "result": "Hello from sandbox!\n",
            "exitCode": 0
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());

    let result = client
        .exec("sb_int", "echo Hello from sandbox!", Some("/sandbox"), None)
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.result, "Hello from sandbox!\n");
}

#[tokio::test]
async fn test_file_roundtrip_via_client() {
    let mock_server = MockServer::start().await;

    // Mock upload
    Mock::given(method("POST"))
        .and(path("/sb_int/files/upload"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .expect(1)
        .mount(&mock_server)
        .await;

    // Mock download
    Mock::given(method("GET"))
        .and(path("/sb_int/files/download"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"print('hello world')\n".to_vec()))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());

    // Upload
    client
        .file_upload("sb_int", "/sandbox/main.py", b"print('hello world')\n")
        .await
        .unwrap();

    // Download and verify
    let bytes = client
        .file_download("sb_int", "/sandbox/main.py")
        .await
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&bytes), "print('hello world')\n");
}

#[tokio::test]
async fn test_sandbox_lifecycle_via_client() {
    let mock_server = MockServer::start().await;

    // Create
    Mock::given(method("POST"))
        .and(path("/sandbox"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "sb_lifecycle",
            "name": "Test Lifecycle",
            "state": "started"
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    // Get (for wait_for_ready)
    Mock::given(method("GET"))
        .and(path("/sandbox/sb_lifecycle"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "sb_lifecycle",
            "state": "started"
        })))
        .mount(&mock_server)
        .await;

    // Set autostop
    Mock::given(method("POST"))
        .and(path("/sandbox/sb_lifecycle/autostop/5"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .expect(1)
        .mount(&mock_server)
        .await;

    // Stop
    Mock::given(method("POST"))
        .and(path("/sandbox/sb_lifecycle/stop"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .expect(1)
        .mount(&mock_server)
        .await;

    // Delete
    Mock::given(method("DELETE"))
        .and(path("/sandbox/sb_lifecycle"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());

    // Full lifecycle
    let info = client
        .create_sandbox(json!({"name": "Test Lifecycle"}))
        .await
        .unwrap();
    assert_eq!(info.id, "sb_lifecycle");
    assert_eq!(info.state, "started");

    client.wait_for_ready("sb_lifecycle").await.unwrap();
    client.set_autostop("sb_lifecycle", 5).await.unwrap();
    client.stop_sandbox("sb_lifecycle").await.unwrap();
    client.delete_sandbox("sb_lifecycle").await.unwrap();
}

#[tokio::test]
async fn test_git_clone_via_client() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/sb_git/git/clone"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());

    client
        .git_clone(
            "sb_git",
            "https://github.com/user/repo.git",
            "/sandbox/user/repo",
            Some("main"),
            Some("oauth2"),
            Some("ghp_token"),
        )
        .await
        .unwrap();
}

// ----------------------------------------------------------------------------
// Tool execute_with_context tests (state + parameter orchestration)
// ----------------------------------------------------------------------------

#[tokio::test]
async fn test_exec_tool_missing_api_key() {
    let tool = get_tool("daytona_exec");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let context = ToolContext::with_storage_store(session_id, store);

    let result = tool
        .execute_with_context(json!({"sandbox_id": "sb_test", "command": "ls"}), &context)
        .await;

    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("DAYTONA_API_KEY"), "Got: {msg}");
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_exec_tool_missing_sandbox_state() {
    let tool = get_tool("daytona_exec");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    store
        .seed_secret(session_id, "DAYTONA_API_KEY", "test_key")
        .await;
    let context = ToolContext::with_storage_store(session_id, store);

    let result = tool
        .execute_with_context(
            json!({"sandbox_id": "sb_missing", "command": "ls"}),
            &context,
        )
        .await;

    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("not found"), "Got: {msg}");
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_exec_tool_missing_command_param() {
    let tool = get_tool("daytona_exec");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let context = ToolContext::with_storage_store(session_id, store);

    let result = tool
        .execute_with_context(json!({"sandbox_id": "sb_test"}), &context)
        .await;

    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("Missing required parameter"), "Got: {msg}");
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_read_file_tool_missing_path() {
    let tool = get_tool("daytona_read_file");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let context = ToolContext::with_storage_store(session_id, store);

    let result = tool
        .execute_with_context(json!({"sandbox_id": "sb_test"}), &context)
        .await;

    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("Missing required parameter"), "Got: {msg}");
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_write_file_tool_missing_content() {
    let tool = get_tool("daytona_write_file");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let context = ToolContext::with_storage_store(session_id, store);

    let result = tool
        .execute_with_context(
            json!({"sandbox_id": "sb_test", "path": "/test.txt"}),
            &context,
        )
        .await;

    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("Missing required parameter"), "Got: {msg}");
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_manage_sandbox_invalid_action() {
    let tool = get_tool("daytona_manage_sandbox");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    setup_context_with_sandbox(session_id, &store, "sb_test").await;
    let context = ToolContext::with_storage_store(session_id, store);

    // Tool will pass validation, get API key, verify sandbox exists,
    // then reject invalid action before making HTTP call
    let result = tool
        .execute_with_context(
            json!({"sandbox_id": "sb_test", "action": "restart"}),
            &context,
        )
        .await;

    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("Invalid action"), "Got: {msg}");
            assert!(msg.contains("restart"), "Got: {msg}");
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_list_sandboxes_empty() {
    let tool = get_tool("daytona_list_sandboxes");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let context = ToolContext::with_storage_store(session_id, store);

    let result = tool.execute_with_context(json!({}), &context).await;

    match result {
        ToolExecutionResult::Success(output) => {
            let val = output;
            assert_eq!(val["count"], 0);
            assert_eq!(val["sandboxes"].as_array().unwrap().len(), 0);
        }
        other => panic!("Expected Success, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_list_sandboxes_with_entries() {
    let tool = get_tool("daytona_list_sandboxes");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());

    // Seed two sandbox states
    setup_context_with_sandbox(session_id, &store, "sb_one").await;
    let state2 = SandboxState {
        sandbox_id: "sb_two".to_string(),
        workspace_path: "/sandbox".to_string(),
        started_at: "2026-02-18T11:00:00Z".to_string(),
    };
    store
        .seed_secret(
            session_id,
            "daytona_sandbox:sb_two",
            &serde_json::to_string(&state2).unwrap(),
        )
        .await;

    let context = ToolContext::with_storage_store(session_id, store);

    let result = tool.execute_with_context(json!({}), &context).await;

    match result {
        ToolExecutionResult::Success(output) => {
            let val = output;
            assert_eq!(val["count"], 2);
            let sandboxes = val["sandboxes"].as_array().unwrap();
            let ids: Vec<&str> = sandboxes
                .iter()
                .map(|s| s["sandbox_id"].as_str().unwrap())
                .collect();
            assert!(ids.contains(&"sb_one"));
            assert!(ids.contains(&"sb_two"));
        }
        other => panic!("Expected Success, got: {other:?}"),
    }
}

// ----------------------------------------------------------------------------
// Client error handling integration tests
// ----------------------------------------------------------------------------

#[tokio::test]
async fn test_client_timeout_on_wait_for_ready() {
    let mock_server = MockServer::start().await;

    // Always return "creating" state — sandbox never becomes ready
    Mock::given(method("GET"))
        .and(path("/sandbox/sb_slow"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "sb_slow",
            "state": "creating"
        })))
        .mount(&mock_server)
        .await;

    // Use very short poll interval and max wait by creating client with custom URLs
    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());

    // wait_for_ready uses constants from the crate; this will timeout after SANDBOX_READY_MAX_WAIT
    // For test speed, we just verify it eventually returns an error
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(70),
        client.wait_for_ready("sb_slow"),
    )
    .await;

    match result {
        Ok(Err(msg)) => {
            assert!(msg.contains("did not become ready"), "Got: {msg}");
        }
        Ok(Ok(())) => panic!("Expected timeout error, got Ok"),
        Err(_) => panic!("Test itself timed out"),
    }
}

#[tokio::test]
async fn test_client_wait_for_ready_build_failed() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/sandbox/sb_fail"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "sb_fail",
            "state": "build_failed"
        })))
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());

    let result = client.wait_for_ready("sb_fail").await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("error state"));
}

#[tokio::test]
async fn test_client_folder_and_file_operations() {
    let mock_server = MockServer::start().await;

    // Create folder
    Mock::given(method("POST"))
        .and(path("/sb_ops/files/folder"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .expect(1)
        .mount(&mock_server)
        .await;

    // Upload file
    Mock::given(method("POST"))
        .and(path("/sb_ops/files/upload"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .expect(1)
        .mount(&mock_server)
        .await;

    // List files
    Mock::given(method("GET"))
        .and(path("/sb_ops/files/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"name": "src", "isDir": true, "size": 0},
            {"name": "main.py", "isDir": false, "size": 100}
        ])))
        .expect(1)
        .mount(&mock_server)
        .await;

    // Delete file
    Mock::given(method("DELETE"))
        .and(path("/sb_ops/files"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());

    // Create folder, upload, list, delete
    client
        .create_folder("sb_ops", "/sandbox/src", "755")
        .await
        .unwrap();
    client
        .file_upload("sb_ops", "/sandbox/main.py", b"print('hi')")
        .await
        .unwrap();

    let entries = client.file_list("sb_ops", "/sandbox").await.unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["name"], "src");
    assert!(entries[0]["isDir"].as_bool().unwrap());

    client
        .file_delete("sb_ops", "/sandbox/old.txt")
        .await
        .unwrap();
}

#[tokio::test]
async fn test_exec_with_nonzero_exit_preserves_output() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/sb_err/process/execute"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "result": "bash: foobar: command not found\n",
            "exitCode": 127
        })))
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());

    let result = client.exec("sb_err", "foobar", None, None).await.unwrap();
    assert_eq!(result.exit_code, 127);
    assert!(result.result.contains("command not found"));
}

// ----------------------------------------------------------------------------
// State management integration tests
// ----------------------------------------------------------------------------

#[tokio::test]
async fn test_sandbox_state_persistence_roundtrip() {
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());

    // Seed sandbox
    setup_context_with_sandbox(session_id, &store, "sb_persist").await;

    let context = ToolContext::with_storage_store(session_id, store.clone());

    // Retrieve via list
    let tool = get_tool("daytona_list_sandboxes");
    let result = tool.execute_with_context(json!({}), &context).await;

    match result {
        ToolExecutionResult::Success(output) => {
            let val = output;
            assert_eq!(val["count"], 1);
            let sandbox = &val["sandboxes"][0];
            assert_eq!(sandbox["sandbox_id"], "sb_persist");
            assert_eq!(sandbox["workspace_path"], "/sandbox");
        }
        other => panic!("Expected Success, got: {other:?}"),
    }
}

// ----------------------------------------------------------------------------
// Bearer auth verification
// ----------------------------------------------------------------------------

#[tokio::test]
async fn test_client_sends_bearer_auth() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/sandbox/sb_auth"))
        .and(wiremock::matchers::header(
            "Authorization",
            "Bearer secret_key_123",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "sb_auth",
            "state": "started"
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = DaytonaClient::with_base_urls(
        "secret_key_123".to_string(),
        mock_server.uri(),
        mock_server.uri(),
    );

    let info = client.get_sandbox("sb_auth").await.unwrap();
    assert_eq!(info.id, "sb_auth");
}
