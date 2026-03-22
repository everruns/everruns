//! Integration tests: tool execute_with_context against wiremock E2B API.
//!
//! These tests exercise the full tool execution flow:
//! MockStorageStore → tool.execute_with_context() → E2BClient → wiremock
//!
//! Unlike the unit tests in src/tools.rs (which test schemas, "requires context" errors,
//! and parameter validation), these tests verify actual HTTP interactions with the
//! E2B API through the tool layer and client.

use async_trait::async_trait;
use everruns_core::capabilities::Capability;
use everruns_core::error::Result;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::{KeyInfo, SecretInfo, SessionStorageStore, ToolContext};
use everruns_core::typed_id::SessionId;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// Force linker to include the integration crate.
use everruns_integrations_e2b as _;

use everruns_integrations_e2b::E2BCapability;
use everruns_integrations_e2b::client::E2BClient;
use everruns_integrations_e2b::state::SandboxState;
use everruns_integrations_e2b::{E2B_API_KEY_SECRET, E2B_SANDBOX_SECRET_PREFIX};

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
// Helpers
// ============================================================================

fn get_tool(name: &str) -> Box<dyn Tool> {
    let cap = E2BCapability;
    cap.tools()
        .into_iter()
        .find(|t| t.name() == name)
        .unwrap_or_else(|| panic!("Tool {name} not found"))
}

/// Seed sandbox state into the store. API key comes via session secret.
async fn setup_context_with_sandbox(
    session_id: SessionId,
    store: &Arc<MockStorageStore>,
    sandbox_id: &str,
) {
    let state = SandboxState {
        sandbox_id: sandbox_id.to_string(),
        sandbox_domain: "e2b.app".to_string(),
        envd_version: "0.1.0".to_string(),
        envd_access_token: Some("envd_token".to_string()),
        workspace_path: "/home/user".to_string(),
        started_at: "2026-03-22T00:00:00Z".to_string(),
        timeout_seconds: 3600,
    };
    store
        .seed_secret(
            session_id,
            &format!("{E2B_SANDBOX_SECRET_PREFIX}{sandbox_id}"),
            &serde_json::to_string(&state).unwrap(),
        )
        .await;
}

async fn setup_api_key(session_id: SessionId, store: &Arc<MockStorageStore>) {
    store
        .seed_secret(session_id, E2B_API_KEY_SECRET, "test_api_key")
        .await;
}

// ============================================================================
// E2BClient integration tests (wiremock)
// ============================================================================

#[tokio::test]
async fn test_create_sandbox_via_client() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/sandboxes"))
        .and(header("X-API-KEY", "test_key"))
        .and(wiremock::matchers::body_json(json!({
            "templateID": "base",
            "timeout": 3600,
            "autoPause": true,
            "allowInternetAccess": true,
            "metadata": {"everruns": "true"},
            "envVars": {}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "clientId": "client_1",
            "envdVersion": "0.1.0",
            "sandboxId": "sb_create",
            "templateId": "base",
            "alias": null,
            "domain": "e2b.app",
            "envdAccessToken": "envd_token_123"
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = E2BClient::with_base_url("test_key".to_string(), mock_server.uri());

    let result = client
        .create_sandbox("base", 3600, json!({"everruns": "true"}), json!({}))
        .await
        .unwrap();

    assert_eq!(result.sandbox_id, "sb_create");
    assert_eq!(result.template_id, "base");
    assert_eq!(result.envd_access_token, Some("envd_token_123".to_string()));
    assert_eq!(result.domain, Some("e2b.app".to_string()));
}

#[tokio::test]
async fn test_get_sandbox_via_client() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/sandboxes/sb_detail"))
        .and(header("X-API-KEY", "test_key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "clientId": "client_1",
            "cpuCount": 2,
            "diskSizeMb": 512,
            "endAt": "2026-03-22T01:00:00Z",
            "envdVersion": "0.1.0",
            "memoryMb": 256,
            "sandboxId": "sb_detail",
            "startedAt": "2026-03-22T00:00:00Z",
            "state": "running",
            "templateId": "base",
            "domain": "e2b.app",
            "envdAccessToken": "envd_token_456"
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = E2BClient::with_base_url("test_key".to_string(), mock_server.uri());

    let detail = client.get_sandbox("sb_detail").await.unwrap();
    assert_eq!(detail.sandbox_id, "sb_detail");
    assert_eq!(detail.state, "running");
    assert_eq!(detail.cpu_count, 2);
    assert_eq!(detail.memory_mb, 256);
}

#[tokio::test]
async fn test_sandbox_lifecycle_via_client() {
    let mock_server = MockServer::start().await;

    // Pause
    Mock::given(method("POST"))
        .and(path("/sandboxes/sb_lifecycle/pause"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .expect(1)
        .mount(&mock_server)
        .await;

    // Resume
    Mock::given(method("POST"))
        .and(path("/sandboxes/sb_lifecycle/resume"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "clientId": "client_1",
            "envdVersion": "0.1.0",
            "sandboxId": "sb_lifecycle",
            "templateId": "base",
            "domain": "e2b.app",
            "envdAccessToken": "envd_token_resumed"
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    // Delete
    Mock::given(method("DELETE"))
        .and(path("/sandboxes/sb_lifecycle"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = E2BClient::with_base_url("test_key".to_string(), mock_server.uri());

    client.pause_sandbox("sb_lifecycle").await.unwrap();

    let resumed = client.resume_sandbox("sb_lifecycle", 7200).await.unwrap();
    assert_eq!(resumed.sandbox_id, "sb_lifecycle");
    assert_eq!(
        resumed.envd_access_token,
        Some("envd_token_resumed".to_string())
    );

    client.delete_sandbox("sb_lifecycle").await.unwrap();
}

#[tokio::test]
async fn test_set_timeout_via_client() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/sandboxes/sb_timeout/timeout"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = E2BClient::with_base_url("test_key".to_string(), mock_server.uri());
    client.set_timeout("sb_timeout", 7200).await.unwrap();
}

#[tokio::test]
async fn test_client_sends_api_key_header() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/sandboxes/sb_auth"))
        .and(header("X-API-KEY", "secret_key_abc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "clientId": "c1",
            "cpuCount": 1,
            "diskSizeMb": 256,
            "endAt": "",
            "envdVersion": "0.1.0",
            "memoryMb": 128,
            "sandboxId": "sb_auth",
            "startedAt": "",
            "state": "running",
            "templateId": "base"
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = E2BClient::with_base_url("secret_key_abc".to_string(), mock_server.uri());
    let detail = client.get_sandbox("sb_auth").await.unwrap();
    assert_eq!(detail.sandbox_id, "sb_auth");
}

#[tokio::test]
async fn test_client_api_error_returns_status() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/sandboxes/sb_404"))
        .respond_with(
            ResponseTemplate::new(404).set_body_json(json!({"message": "sandbox not found"})),
        )
        .mount(&mock_server)
        .await;

    let client = E2BClient::with_base_url("test_key".to_string(), mock_server.uri());
    let err = client.get_sandbox("sb_404").await.unwrap_err();
    assert!(err.contains("404"), "Got: {err}");
}

// ============================================================================
// Tool execute_with_context tests (state + parameter orchestration)
// ============================================================================

#[tokio::test]
async fn test_exec_tool_missing_api_key() {
    let tool = get_tool("e2b_exec");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let context = ToolContext::with_storage_store(session_id, store);

    let result = tool
        .execute_with_context(json!({"sandbox_id": "sb_test", "command": "ls"}), &context)
        .await;

    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("API key not configured"), "Got: {msg}");
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_exec_tool_missing_sandbox_state() {
    let tool = get_tool("e2b_exec");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    setup_api_key(session_id, &store).await;
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
    let tool = get_tool("e2b_exec");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    setup_api_key(session_id, &store).await;
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
async fn test_exec_tool_missing_sandbox_id() {
    let tool = get_tool("e2b_exec");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    setup_api_key(session_id, &store).await;
    let context = ToolContext::with_storage_store(session_id, store);

    let result = tool
        .execute_with_context(json!({"command": "ls"}), &context)
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
    let tool = get_tool("e2b_read_file");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    setup_api_key(session_id, &store).await;
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
    let tool = get_tool("e2b_write_file");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    setup_api_key(session_id, &store).await;
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
    let tool = get_tool("e2b_manage_sandbox");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    setup_api_key(session_id, &store).await;
    setup_context_with_sandbox(session_id, &store, "sb_test").await;
    let context = ToolContext::with_storage_store(session_id, store);

    let result = tool
        .execute_with_context(
            json!({"sandbox_id": "sb_test", "action": "restart"}),
            &context,
        )
        .await;

    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("Invalid action"), "Got: {msg}");
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_list_sandboxes_empty() {
    let tool = get_tool("e2b_list_sandboxes");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let context = ToolContext::with_storage_store(session_id, store);

    let result = tool.execute_with_context(json!({}), &context).await;

    match result {
        ToolExecutionResult::Success(output) => {
            assert_eq!(output["count"], 0);
            assert_eq!(output["sandboxes"].as_array().unwrap().len(), 0);
        }
        other => panic!("Expected Success, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_list_sandboxes_with_entries() {
    let tool = get_tool("e2b_list_sandboxes");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());

    setup_context_with_sandbox(session_id, &store, "sb_one").await;
    setup_context_with_sandbox(session_id, &store, "sb_two").await;

    let context = ToolContext::with_storage_store(session_id, store);

    let result = tool.execute_with_context(json!({}), &context).await;

    match result {
        ToolExecutionResult::Success(output) => {
            assert_eq!(output["count"], 2);
            let sandboxes = output["sandboxes"].as_array().unwrap();
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
// State management integration tests
// ============================================================================

#[tokio::test]
async fn test_sandbox_state_persistence_roundtrip() {
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());

    setup_context_with_sandbox(session_id, &store, "sb_persist").await;

    let context = ToolContext::with_storage_store(session_id, store.clone());

    let tool = get_tool("e2b_list_sandboxes");
    let result = tool.execute_with_context(json!({}), &context).await;

    match result {
        ToolExecutionResult::Success(output) => {
            assert_eq!(output["count"], 1);
            let sandbox = &output["sandboxes"][0];
            assert_eq!(sandbox["sandbox_id"], "sb_persist");
            assert_eq!(sandbox["workspace_path"], "/home/user");
        }
        other => panic!("Expected Success, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_create_sandbox_tool_no_storage_context() {
    let tool = get_tool("e2b_create_sandbox");
    let session_id = SessionId::new();
    let context = ToolContext::new(session_id);

    let result = tool.execute_with_context(json!({}), &context).await;

    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("API key not configured"), "Got: {msg}");
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_api_key_resolved_from_session_secret() {
    // Verify that session-secret API key resolution works by contrasting
    // behavior with and without a seeded E2B_API_KEY secret.
    // Uses read_file which requires API key + sandbox state — the error
    // changes from "API key not configured" to "not found" when the key
    // is present, proving the resolution path works without any network call.
    let tool = get_tool("e2b_read_file");
    let session_id = SessionId::new();

    // Without API key — should fail on key resolution
    let store_no_key = Arc::new(MockStorageStore::new());
    let ctx_no_key = ToolContext::with_storage_store(session_id, store_no_key);
    let result = tool
        .execute_with_context(
            json!({"sandbox_id": "sb_secret", "path": "/tmp/x"}),
            &ctx_no_key,
        )
        .await;
    match &result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("API key not configured"), "Got: {msg}");
        }
        other => panic!("Expected ToolError about API key, got: {other:?}"),
    }

    // With API key seeded as session secret — should get past key check
    // and fail on missing sandbox state instead
    let store_with_key = Arc::new(MockStorageStore::new());
    setup_api_key(session_id, &store_with_key).await;
    let ctx_with_key = ToolContext::with_storage_store(session_id, store_with_key);
    let result = tool
        .execute_with_context(
            json!({"sandbox_id": "sb_secret", "path": "/tmp/x"}),
            &ctx_with_key,
        )
        .await;
    match &result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(
                msg.contains("not found"),
                "Should fail on missing sandbox, not API key: {msg}"
            );
        }
        other => panic!("Expected ToolError about sandbox not found, got: {other:?}"),
    }
}
