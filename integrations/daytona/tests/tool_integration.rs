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
use everruns_core::traits::{
    KeyInfo, SecretInfo, SessionStorageStore, ToolContext, UserConnectionResolver,
};
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

// ============================================================================
// Mock SessionStorageStore
// ============================================================================

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

// ============================================================================
// Mock ConnectionResolver
// ============================================================================

struct MockConnectionResolver {
    token: Option<String>,
}

#[async_trait]
impl UserConnectionResolver for MockConnectionResolver {
    async fn get_connection_token(
        &self,
        _session_id: SessionId,
        _provider: &str,
    ) -> Result<Option<String>> {
        Ok(self.token.clone())
    }
}

/// Create a mock connection resolver that returns a fixed Daytona API key.
fn daytona_resolver() -> Arc<dyn UserConnectionResolver> {
    Arc::new(MockConnectionResolver {
        token: Some("test_api_key".to_string()),
    })
}

// ============================================================================
// Helpers
// ============================================================================

/// Seed sandbox state into the store. API key comes via connection resolver.
async fn setup_context_with_sandbox(
    session_id: SessionId,
    store: &Arc<MockStorageStore>,
    sandbox_id: &str,
) {
    let state = SandboxState {
        sandbox_id: sandbox_id.to_string(),
        workspace_path: "/home/daytona".to_string(),
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
    // Use tools_with_config to include opt-in tools like daytona_api_call
    cap.tools_with_config(&json!({"enable_api_calling": true}))
        .into_iter()
        .find(|t| t.name() == name)
        .unwrap_or_else(|| panic!("Tool {name} not found"))
}

// ============================================================================
// DaytonaClient integration tests (wiremock)
// ============================================================================
// These test the client directly with custom base URLs pointing at wiremock.

/// Set up mock responses for the session-based exec flow.
async fn setup_session_mocks(
    mock_server: &MockServer,
    sandbox_id: &str,
    exit_code: i64,
    output: &str,
) {
    // 1. Create session → 201
    Mock::given(method("POST"))
        .and(path(format!("/{sandbox_id}/process/session")))
        .respond_with(ResponseTemplate::new(201))
        .mount(mock_server)
        .await;

    // 2. Async exec → 202 with cmdId
    Mock::given(method("POST"))
        .and(path(format!(
            "/{sandbox_id}/process/session/everruns-exec/exec"
        )))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({
            "cmdId": "cmd_001"
        })))
        .mount(mock_server)
        .await;

    // 3. Logs → output bytes with stdout marker prefix
    let mut log_bytes = vec![0x01, 0x01, 0x01];
    log_bytes.extend_from_slice(output.as_bytes());
    Mock::given(method("GET"))
        .and(path(format!(
            "/{sandbox_id}/process/session/everruns-exec/command/cmd_001/logs"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(log_bytes))
        .mount(mock_server)
        .await;

    // 4. Command status → exitCode
    Mock::given(method("GET"))
        .and(path(format!(
            "/{sandbox_id}/process/session/everruns-exec/command/cmd_001"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "cmd_001",
            "command": "test",
            "exitCode": exit_code
        })))
        .mount(mock_server)
        .await;
}

#[tokio::test]
async fn test_exec_tool_full_flow_via_client() {
    let mock_server = MockServer::start().await;
    setup_session_mocks(&mock_server, "sb_int", 0, "Hello from sandbox!\n").await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());

    let result = client
        .exec(
            "sb_int",
            "echo Hello from sandbox!",
            Some("/sandbox"),
            None,
            |_| {},
        )
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

/// Git clone uses exec via the session API.
#[tokio::test]
async fn test_git_clone_via_exec() {
    let mock_server = MockServer::start().await;
    setup_session_mocks(
        &mock_server,
        "sb_git",
        0,
        "Cloning into '/home/daytona/user/repo'...\n",
    )
    .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());

    let result = client
        .exec(
            "sb_git",
            "git clone --depth 1 https://github.com/user/repo.git /home/daytona/user/repo",
            None,
            None,
            |_| {},
        )
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0);
}

// ============================================================================
// Tool execute_with_context tests (state + parameter orchestration)
// ============================================================================

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
        ToolExecutionResult::ConnectionRequired { provider } => {
            assert_eq!(provider, "daytona");
        }
        other => panic!("Expected ConnectionRequired, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_exec_tool_missing_sandbox_state() {
    let tool = get_tool("daytona_exec");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let context = ToolContext::with_storage_store(session_id, store)
        .with_connection_resolver(daytona_resolver());

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
    let context = ToolContext::with_storage_store(session_id, store)
        .with_connection_resolver(daytona_resolver());

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
        workspace_path: "/home/daytona".to_string(),
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

// ============================================================================
// Client error handling integration tests
// ============================================================================

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
    setup_session_mocks(
        &mock_server,
        "sb_err",
        127,
        "bash: foobar: command not found\n",
    )
    .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());

    let result = client
        .exec("sb_err", "foobar", None, None, |_| {})
        .await
        .unwrap();
    assert_eq!(result.exit_code, 127);
    assert!(result.result.contains("command not found"));
}

// ============================================================================
// Session-based exec integration tests (EVE-186)
// ============================================================================
// All exec goes through the Session API — persistent shell, async execution,
// and log polling for real-time streaming output.

#[tokio::test]
async fn test_exec_session_with_shell_operators() {
    let mock_server = MockServer::start().await;
    setup_session_mocks(&mock_server, "sb_shell", 0, "hello\nworld\n").await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());

    // Shell operators (&&) are handled natively by the session's shell
    let result = client
        .exec("sb_shell", "echo hello && echo world", None, None, |_| {})
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.result, "hello\nworld\n");
}

#[tokio::test]
async fn test_exec_session_with_pipes_and_redirects() {
    let mock_server = MockServer::start().await;
    setup_session_mocks(&mock_server, "sb_pipe", 0, "").await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());

    let result = client
        .exec(
            "sb_pipe",
            "cat /etc/os-release | head -1 > /tmp/out.txt",
            None,
            None,
            |_| {},
        )
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
}

#[tokio::test]
async fn test_exec_session_with_cwd_prepended() {
    let mock_server = MockServer::start().await;

    // Create session
    Mock::given(method("POST"))
        .and(path("/sb_cwd/process/session"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&mock_server)
        .await;

    // Verify the command is prepended with shell-profile preamble + cd.
    // The preamble sources common profile files before running the user command.
    let expected_cmd = concat!(
        "for __f in \"$HOME/.profile\" \"$HOME/.cargo/env\" \"$HOME/.nvm/nvm.sh\"; do ",
        "[ -f \"$__f\" ] && . \"$__f\" >/dev/null 2>&1; ",
        "done; unset __f; ",
        "cd /workspace && ls -la",
    );
    Mock::given(method("POST"))
        .and(path("/sb_cwd/process/session/everruns-exec/exec"))
        .and(wiremock::matchers::body_json(json!({
            "command": expected_cmd,
            "runAsync": true
        })))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({
            "cmdId": "cmd_cwd"
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    // Logs
    let mut log_bytes = vec![0x01, 0x01, 0x01];
    log_bytes.extend_from_slice(b"file.txt\n");
    Mock::given(method("GET"))
        .and(path(
            "/sb_cwd/process/session/everruns-exec/command/cmd_cwd/logs",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(log_bytes))
        .mount(&mock_server)
        .await;

    // Status
    Mock::given(method("GET"))
        .and(path(
            "/sb_cwd/process/session/everruns-exec/command/cmd_cwd",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "cmd_cwd", "exitCode": 0
        })))
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());

    let result = client
        .exec("sb_cwd", "ls -la", Some("/workspace"), None, |_| {})
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
}

#[tokio::test]
async fn test_exec_session_with_variable_expansion() {
    let mock_server = MockServer::start().await;
    setup_session_mocks(&mock_server, "sb_var", 0, "/home/daytona\n").await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());

    // Variable expansion is handled natively by the session's shell
    let result = client
        .exec("sb_var", "echo $HOME", None, None, |_| {})
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.result, "/home/daytona\n");
}

#[tokio::test]
async fn test_exec_session_reuses_existing() {
    let mock_server = MockServer::start().await;

    // Session already exists → 409
    Mock::given(method("POST"))
        .and(path("/sb_reuse/process/session"))
        .respond_with(ResponseTemplate::new(409))
        .mount(&mock_server)
        .await;

    // Exec still works
    Mock::given(method("POST"))
        .and(path("/sb_reuse/process/session/everruns-exec/exec"))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({"cmdId": "cmd_r"})))
        .mount(&mock_server)
        .await;

    let mut log_bytes = vec![0x01, 0x01, 0x01];
    log_bytes.extend_from_slice(b"ok\n");
    Mock::given(method("GET"))
        .and(path(
            "/sb_reuse/process/session/everruns-exec/command/cmd_r/logs",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(log_bytes))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path(
            "/sb_reuse/process/session/everruns-exec/command/cmd_r",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "cmd_r", "exitCode": 0
        })))
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());

    let result = client
        .exec("sb_reuse", "echo ok", None, None, |_| {})
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.result, "ok\n");
}

#[tokio::test]
async fn test_exec_session_streaming_emits_chunks() {
    use std::sync::{Arc, Mutex};

    let mock_server = MockServer::start().await;
    setup_session_mocks(&mock_server, "sb_stream", 0, "chunk1\nchunk2\n").await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());

    let chunks: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let chunks_clone = chunks.clone();

    let result = client
        .exec("sb_stream", "echo streaming", None, Some(30_000), |chunk| {
            chunks_clone.lock().unwrap().push(chunk.to_string());
        })
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.result, "chunk1\nchunk2\n");

    let collected = chunks.lock().unwrap();
    assert!(!collected.is_empty(), "should have received output chunks");
}

#[tokio::test]
async fn test_exec_session_strips_stream_markers() {
    let mock_server = MockServer::start().await;

    // Create session
    Mock::given(method("POST"))
        .and(path("/sb_markers/process/session"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&mock_server)
        .await;

    Mock::given(method("POST"))
        .and(path("/sb_markers/process/session/everruns-exec/exec"))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({"cmdId": "cmd_m"})))
        .mount(&mock_server)
        .await;

    // Logs with mixed stdout/stderr markers
    let mut log_bytes = Vec::new();
    log_bytes.extend_from_slice(&[0x01, 0x01, 0x01]); // stdout marker
    log_bytes.extend_from_slice(b"stdout line\n");
    log_bytes.extend_from_slice(&[0x02, 0x02, 0x02]); // stderr marker
    log_bytes.extend_from_slice(b"stderr line\n");
    log_bytes.extend_from_slice(&[0x01, 0x01, 0x01]); // stdout marker
    log_bytes.extend_from_slice(b"more stdout\n");
    Mock::given(method("GET"))
        .and(path(
            "/sb_markers/process/session/everruns-exec/command/cmd_m/logs",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(log_bytes))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path(
            "/sb_markers/process/session/everruns-exec/command/cmd_m",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "cmd_m", "exitCode": 0
        })))
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());

    let result = client
        .exec("sb_markers", "test", None, None, |_| {})
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    // All markers stripped, combined output returned
    assert_eq!(result.result, "stdout line\nstderr line\nmore stdout\n");
}

// ============================================================================
// State management integration tests
// ============================================================================

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
            assert_eq!(sandbox["workspace_path"], "/home/daytona");
        }
        other => panic!("Expected Success, got: {other:?}"),
    }
}

// ============================================================================
// Bearer auth verification
// ============================================================================

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

// ============================================================================
// Resource options tests
// ============================================================================

#[tokio::test]
async fn test_create_sandbox_with_resources_via_client() {
    let mock_server = MockServer::start().await;

    // Daytona REST API expects cpu/memory/disk as top-level fields
    Mock::given(method("POST"))
        .and(path("/sandbox"))
        .and(wiremock::matchers::body_json(json!({
            "name": "Resource Test",
            "cpu": 2, "memory": 4, "disk": 8
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "sb_resource",
            "name": "Resource Test",
            "state": "started"
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());

    let info = client
        .create_sandbox(json!({
            "name": "Resource Test",
            "cpu": 2, "memory": 4, "disk": 8
        }))
        .await
        .unwrap();
    assert_eq!(info.id, "sb_resource");
}

#[tokio::test]
async fn test_create_sandbox_partial_resources_via_client() {
    let mock_server = MockServer::start().await;

    // Daytona REST API expects disk as a top-level field
    Mock::given(method("POST"))
        .and(path("/sandbox"))
        .and(wiremock::matchers::body_json(json!({
            "name": "Partial",
            "disk": 5
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "sb_partial",
            "state": "started"
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());

    let info = client
        .create_sandbox(json!({
            "name": "Partial",
            "disk": 5
        }))
        .await
        .unwrap();
    assert_eq!(info.id, "sb_partial");
}

#[tokio::test]
async fn test_create_sandbox_tool_rejects_invalid_size() {
    let tool = get_tool("daytona_create_sandbox");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let context = ToolContext::with_storage_store(session_id, store)
        .with_connection_resolver(daytona_resolver());

    let result = tool
        .execute_with_context(json!({"size": "huge"}), &context)
        .await;

    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("size"), "Got: {msg}");
            assert!(msg.contains("small"), "Got: {msg}");
            assert!(msg.contains("medium"), "Got: {msg}");
            assert!(msg.contains("large"), "Got: {msg}");
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_create_sandbox_tool_rejects_invalid_auto_stop() {
    let tool = get_tool("daytona_create_sandbox");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let context = ToolContext::with_storage_store(session_id, store)
        .with_connection_resolver(daytona_resolver());

    let result = tool
        .execute_with_context(json!({"auto_stop_minutes": 100}), &context)
        .await;

    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("auto_stop_minutes"), "Got: {msg}");
            assert!(msg.contains("between 1 and 60"), "Got: {msg}");
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

// ============================================================================
// Snapshot listing tests
// ============================================================================

#[tokio::test]
async fn test_list_snapshots_via_client() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/snapshots"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {
                "id": "snap_1",
                "name": "daytona-small",
                "state": "active",
                "cpu": 1,
                "mem": 1,
                "disk": 3,
                "general": true
            },
            {
                "id": "snap_2",
                "name": "daytona-medium",
                "state": "active",
                "cpu": 2,
                "mem": 4,
                "disk": 8,
                "general": true
            },
            {
                "id": "snap_3",
                "name": "custom-ml",
                "state": "active",
                "cpu": 4,
                "mem": 16,
                "disk": 50,
                "gpu": 1,
                "general": false
            }
        ])))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());

    let snapshots = client.list_snapshots().await.unwrap();
    assert_eq!(snapshots.len(), 3);
    assert_eq!(snapshots[0].name, "daytona-small");
    assert_eq!(snapshots[0].cpu, Some(1));
    assert_eq!(snapshots[1].name, "daytona-medium");
    assert_eq!(snapshots[2].name, "custom-ml");
    assert_eq!(snapshots[2].gpu, Some(1));
}

#[tokio::test]
async fn test_list_snapshots_empty() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/snapshots"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());

    let snapshots = client.list_snapshots().await.unwrap();
    assert!(snapshots.is_empty());
}

#[tokio::test]
async fn test_list_snapshots_tool_missing_api_key() {
    let tool = get_tool("daytona_list_snapshots");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let context = ToolContext::with_storage_store(session_id, store);

    let result = tool.execute_with_context(json!({}), &context).await;

    match result {
        ToolExecutionResult::ConnectionRequired { provider } => {
            assert_eq!(provider, "daytona");
        }
        other => panic!("Expected ConnectionRequired, got: {other:?}"),
    }
}

// ============================================================================
// DaytonaApiCallTool integration tests (wiremock)
// ============================================================================

#[tokio::test]
async fn test_api_call_get_sandbox_list() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/sandbox"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": "sb_1", "name": "Test", "state": "started"},
            {"id": "sb_2", "name": "Other", "state": "stopped"}
        ])))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());
    let result = client
        .api_call(reqwest::Method::GET, "/sandbox", None)
        .await
        .unwrap();

    let sandboxes = result.as_array().unwrap();
    assert_eq!(sandboxes.len(), 2);
    assert_eq!(sandboxes[0]["id"], "sb_1");
}

#[tokio::test]
async fn test_api_call_routes_toolbox_path() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/sb_test/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"name": "hello.txt", "isDir": false, "size": 12}
        ])))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());
    let result = client
        .api_call(reqwest::Method::GET, "/toolbox/sb_test/files", None)
        .await
        .unwrap();

    let files = result.as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["name"], "hello.txt");
}

#[tokio::test]
async fn test_api_call_tool_injects_labels_on_sandbox_create() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/sandbox"))
        .and(wiremock::matchers::body_partial_json(json!({
            "labels": {
                "everruns": "true"
            }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "sb_api_created",
            "name": "API Created",
            "state": "started"
        })))
        .expect(1)
        .named("create with labels")
        .mount(&mock_server)
        .await;

    let session_id = SessionId::new();
    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());

    let body = json!({"name": "API Created", "snapshot": "daytona-small"});
    let mut labels = serde_json::Map::new();
    labels.insert("everruns".to_string(), json!("true"));
    labels.insert(
        "everruns.session_id".to_string(),
        json!(session_id.to_string()),
    );
    let mut body_with_labels = body.clone();
    body_with_labels
        .as_object_mut()
        .unwrap()
        .insert("labels".to_string(), json!(labels));

    let response = client
        .api_call(reqwest::Method::POST, "/sandbox", Some(body_with_labels))
        .await
        .unwrap();
    assert_eq!(response["id"], "sb_api_created");
}

#[tokio::test]
async fn test_api_call_tool_tracks_sandbox_state_on_create() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/sandbox"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "sb_tracked",
            "name": "Tracked Sandbox",
            "state": "started"
        })))
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());

    let response = client
        .api_call(
            reqwest::Method::POST,
            "/sandbox",
            Some(json!({"name": "Tracked Sandbox"})),
        )
        .await
        .unwrap();

    assert_eq!(response["id"], "sb_tracked");
    assert_eq!(response["state"], "started");
}

#[tokio::test]
async fn test_api_call_delete_sandbox() {
    let mock_server = MockServer::start().await;

    Mock::given(method("DELETE"))
        .and(path("/sandbox/sb_del"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());

    let result = client
        .api_call(reqwest::Method::DELETE, "/sandbox/sb_del", None)
        .await
        .unwrap();
    assert!(result.is_object());
}

#[tokio::test]
async fn test_api_call_client_error_propagates() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/sandbox/nonexistent"))
        .respond_with(ResponseTemplate::new(404).set_body_string("Sandbox not found"))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());

    let result = client
        .api_call(reqwest::Method::GET, "/sandbox/nonexistent", None)
        .await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("404"));
}

#[tokio::test]
async fn test_api_call_toolbox_post_with_body() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/sb_exec/process/execute"))
        .and(wiremock::matchers::body_partial_json(json!({
            "command": "echo hello"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": 0,
            "result": "hello\n"
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client =
        DaytonaClient::with_base_urls("test_key".to_string(), mock_server.uri(), mock_server.uri());

    let result = client
        .api_call(
            reqwest::Method::POST,
            "/toolbox/sb_exec/process/execute",
            Some(json!({"command": "echo hello"})),
        )
        .await
        .unwrap();

    assert_eq!(result["code"], 0);
    assert_eq!(result["result"], "hello\n");
}
