//! Integration tests: tool execute_with_context against wiremock Sprites API.
//!
//! These tests exercise the full tool execution flow:
//! MockStorageStore → tool.execute_with_context() → SpritesClient → wiremock

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
use everruns_integrations_sprites as _;

use everruns_integrations_sprites::client::SpritesClient;
use everruns_integrations_sprites::state::SpriteState;

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

fn sprites_resolver() -> Arc<dyn UserConnectionResolver> {
    Arc::new(MockConnectionResolver {
        token: Some("test_api_token".to_string()),
    })
}

// ============================================================================
// Helpers
// ============================================================================

async fn setup_context_with_sprite(
    session_id: SessionId,
    store: &Arc<MockStorageStore>,
    sprite_name: &str,
) {
    let state = SpriteState {
        sprite_name: sprite_name.to_string(),
        workspace_path: "/home/user".to_string(),
        started_at: "2026-03-23T10:00:00Z".to_string(),
        service_url: Some(format!("https://{sprite_name}.fly.dev")),
    };
    store
        .seed_secret(
            session_id,
            &format!("sprites_sprite:{sprite_name}"),
            &serde_json::to_string(&state).unwrap(),
        )
        .await;
}

fn get_tool(name: &str) -> Box<dyn Tool> {
    let cap = everruns_integrations_sprites::SpritesCapability;
    use everruns_core::capabilities::Capability;
    cap.tools()
        .into_iter()
        .find(|t| t.name() == name)
        .unwrap_or_else(|| panic!("Tool {name} not found"))
}

// ============================================================================
// SpritesClient integration tests (wiremock)
// ============================================================================

#[tokio::test]
async fn test_exec_tool_full_flow_via_client() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/sprites/my-sprite/exec"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "stdout": "Hello from sprite!\n",
            "stderr": "",
            "exit_code": 0
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = SpritesClient::with_base_url("test_token".to_string(), mock_server.uri());

    let result = client
        .exec("my-sprite", "echo Hello from sprite!", None)
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout, "Hello from sprite!\n");
}

#[tokio::test]
async fn test_file_roundtrip_via_client() {
    let mock_server = MockServer::start().await;

    // Mock write
    Mock::given(method("PUT"))
        .and(path("/sprites/my-sprite/files/%2Fhome%2Fuser%2Fmain.py"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .expect(1)
        .mount(&mock_server)
        .await;

    // Mock read
    Mock::given(method("GET"))
        .and(path("/sprites/my-sprite/files/%2Fhome%2Fuser%2Fmain.py"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"print('hello world')\n".to_vec()))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = SpritesClient::with_base_url("test_token".to_string(), mock_server.uri());

    // Write
    client
        .write_file("my-sprite", "/home/user/main.py", b"print('hello world')\n")
        .await
        .unwrap();

    // Read and verify
    let bytes = client
        .read_file("my-sprite", "/home/user/main.py")
        .await
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&bytes), "print('hello world')\n");
}

#[tokio::test]
async fn test_sprite_lifecycle_via_client() {
    let mock_server = MockServer::start().await;

    // Create
    Mock::given(method("PUT"))
        .and(path("/sprites/lifecycle-test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": "lifecycle-test",
            "status": "running",
            "url": "https://lifecycle-test.fly.dev"
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    // Get
    Mock::given(method("GET"))
        .and(path("/sprites/lifecycle-test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": "lifecycle-test",
            "status": "running"
        })))
        .mount(&mock_server)
        .await;

    // Delete
    Mock::given(method("DELETE"))
        .and(path("/sprites/lifecycle-test"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = SpritesClient::with_base_url("test_token".to_string(), mock_server.uri());

    // Full lifecycle
    let info = client
        .create_sprite("lifecycle-test", json!({}))
        .await
        .unwrap();
    assert_eq!(info.name, "lifecycle-test");
    assert_eq!(info.status, "running");
    assert_eq!(info.url, Some("https://lifecycle-test.fly.dev".to_string()));

    let info = client.get_sprite("lifecycle-test").await.unwrap();
    assert_eq!(info.status, "running");

    client.delete_sprite("lifecycle-test").await.unwrap();
}

#[tokio::test]
async fn test_checkpoint_roundtrip_via_client() {
    let mock_server = MockServer::start().await;

    // Create checkpoint
    Mock::given(method("POST"))
        .and(path("/sprites/my-sprite/checkpoints"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "cp_snap1",
            "created_at": "2026-03-23T10:05:00Z"
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    // List checkpoints
    Mock::given(method("GET"))
        .and(path("/sprites/my-sprite/checkpoints"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": "cp_snap1", "created_at": "2026-03-23T10:05:00Z"}
        ])))
        .expect(1)
        .mount(&mock_server)
        .await;

    // Restore checkpoint
    Mock::given(method("POST"))
        .and(path("/sprites/my-sprite/checkpoints/cp_snap1/restore"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = SpritesClient::with_base_url("test_token".to_string(), mock_server.uri());

    let cp = client.create_checkpoint("my-sprite").await.unwrap();
    assert_eq!(cp.id, "cp_snap1");

    let checkpoints = client.list_checkpoints("my-sprite").await.unwrap();
    assert_eq!(checkpoints.len(), 1);

    client
        .restore_checkpoint("my-sprite", "cp_snap1")
        .await
        .unwrap();
}

// ============================================================================
// Tool execute_with_context tests
// ============================================================================

#[tokio::test]
async fn test_exec_tool_missing_api_token() {
    let tool = get_tool("sprites_exec");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let context = ToolContext::with_storage_store(session_id, store);

    let result = tool
        .execute_with_context(json!({"sprite_name": "test", "command": "ls"}), &context)
        .await;

    match result {
        ToolExecutionResult::ConnectionRequired { provider } => {
            assert_eq!(provider, "sprites");
        }
        other => panic!("Expected ConnectionRequired, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_exec_tool_missing_sprite_state() {
    let tool = get_tool("sprites_exec");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let context = ToolContext::with_storage_store(session_id, store)
        .with_connection_resolver(sprites_resolver());

    let result = tool
        .execute_with_context(json!({"sprite_name": "missing", "command": "ls"}), &context)
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
    let tool = get_tool("sprites_exec");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let context = ToolContext::with_storage_store(session_id, store);

    let result = tool
        .execute_with_context(json!({"sprite_name": "test"}), &context)
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
    let tool = get_tool("sprites_read_file");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let context = ToolContext::with_storage_store(session_id, store);

    let result = tool
        .execute_with_context(json!({"sprite_name": "test"}), &context)
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
    let tool = get_tool("sprites_write_file");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let context = ToolContext::with_storage_store(session_id, store);

    let result = tool
        .execute_with_context(
            json!({"sprite_name": "test", "path": "/test.txt"}),
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
async fn test_manage_sprite_invalid_action() {
    let tool = get_tool("sprites_manage_sprite");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    setup_context_with_sprite(session_id, &store, "test-sprite").await;
    let context = ToolContext::with_storage_store(session_id, store)
        .with_connection_resolver(sprites_resolver());

    let result = tool
        .execute_with_context(
            json!({"sprite_name": "test-sprite", "action": "restart"}),
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
async fn test_list_sprites_empty() {
    let tool = get_tool("sprites_list_sprites");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let context = ToolContext::with_storage_store(session_id, store);

    let result = tool.execute_with_context(json!({}), &context).await;

    match result {
        ToolExecutionResult::Success(output) => {
            assert_eq!(output["count"], 0);
            assert_eq!(output["sprites"].as_array().unwrap().len(), 0);
        }
        other => panic!("Expected Success, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_list_sprites_with_entries() {
    let tool = get_tool("sprites_list_sprites");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());

    setup_context_with_sprite(session_id, &store, "sprite-one").await;
    let state2 = SpriteState {
        sprite_name: "sprite-two".to_string(),
        workspace_path: "/home/user".to_string(),
        started_at: "2026-03-23T11:00:00Z".to_string(),
        service_url: None,
    };
    store
        .seed_secret(
            session_id,
            "sprites_sprite:sprite-two",
            &serde_json::to_string(&state2).unwrap(),
        )
        .await;

    let context = ToolContext::with_storage_store(session_id, store);

    let result = tool.execute_with_context(json!({}), &context).await;

    match result {
        ToolExecutionResult::Success(output) => {
            assert_eq!(output["count"], 2);
            let sprites = output["sprites"].as_array().unwrap();
            let names: Vec<&str> = sprites
                .iter()
                .map(|s| s["sprite_name"].as_str().unwrap())
                .collect();
            assert!(names.contains(&"sprite-one"));
            assert!(names.contains(&"sprite-two"));
        }
        other => panic!("Expected Success, got: {other:?}"),
    }
}

// ============================================================================
// State management integration tests
// ============================================================================

#[tokio::test]
async fn test_sprite_state_persistence_roundtrip() {
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());

    setup_context_with_sprite(session_id, &store, "persist-test").await;

    let context = ToolContext::with_storage_store(session_id, store.clone());

    let tool = get_tool("sprites_list_sprites");
    let result = tool.execute_with_context(json!({}), &context).await;

    match result {
        ToolExecutionResult::Success(output) => {
            assert_eq!(output["count"], 1);
            let sprite = &output["sprites"][0];
            assert_eq!(sprite["sprite_name"], "persist-test");
            assert_eq!(sprite["workspace_path"], "/home/user");
            assert_eq!(sprite["service_url"], "https://persist-test.fly.dev");
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
        .and(path("/sprites/auth-test"))
        .and(wiremock::matchers::header(
            "Authorization",
            "Bearer secret_token_123",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": "auth-test",
            "status": "running"
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = SpritesClient::with_base_url("secret_token_123".to_string(), mock_server.uri());

    let info = client.get_sprite("auth-test").await.unwrap();
    assert_eq!(info.name, "auth-test");
}

// ============================================================================
// Resource options tests
// ============================================================================

#[tokio::test]
async fn test_create_sprite_tool_rejects_invalid_cpus() {
    let tool = get_tool("sprites_create_sprite");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let context = ToolContext::with_storage_store(session_id, store)
        .with_connection_resolver(sprites_resolver());

    let result = tool
        .execute_with_context(json!({"sprite_name": "test", "cpus": 100}), &context)
        .await;

    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("cpus"), "Got: {msg}");
            assert!(msg.contains("between 1 and 8"), "Got: {msg}");
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_create_sprite_tool_rejects_string_memory() {
    let tool = get_tool("sprites_create_sprite");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let context = ToolContext::with_storage_store(session_id, store)
        .with_connection_resolver(sprites_resolver());

    let result = tool
        .execute_with_context(json!({"sprite_name": "test", "memory_mb": "big"}), &context)
        .await;

    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("memory_mb"), "Got: {msg}");
            assert!(msg.contains("positive integer"), "Got: {msg}");
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

// ============================================================================
// Sprite name validation tests
// ============================================================================

#[tokio::test]
async fn test_create_sprite_rejects_path_traversal_name() {
    let tool = get_tool("sprites_create_sprite");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let context = ToolContext::with_storage_store(session_id, store)
        .with_connection_resolver(sprites_resolver());

    let result = tool
        .execute_with_context(json!({"sprite_name": "../../admin"}), &context)
        .await;

    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("lowercase"), "Got: {msg}");
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_create_sprite_rejects_uppercase_name() {
    let tool = get_tool("sprites_create_sprite");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let context = ToolContext::with_storage_store(session_id, store)
        .with_connection_resolver(sprites_resolver());

    let result = tool
        .execute_with_context(json!({"sprite_name": "BAD-NAME"}), &context)
        .await;

    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("lowercase"), "Got: {msg}");
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

// ============================================================================
// Service URL and checkpoint tool tests
// ============================================================================

#[tokio::test]
async fn test_checkpoint_tool_missing_sprite_name() {
    let tool = get_tool("sprites_checkpoint");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let context = ToolContext::with_storage_store(session_id, store);

    let result = tool.execute_with_context(json!({}), &context).await;

    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("Missing required parameter"), "Got: {msg}");
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_restore_checkpoint_missing_id() {
    let tool = get_tool("sprites_restore_checkpoint");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let context = ToolContext::with_storage_store(session_id, store);

    let result = tool
        .execute_with_context(json!({"sprite_name": "test"}), &context)
        .await;

    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("Missing required parameter"), "Got: {msg}");
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_exec_with_nonzero_exit() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/sprites/err-sprite/exec"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "stdout": "",
            "stderr": "bash: foobar: command not found\n",
            "exit_code": 127
        })))
        .mount(&mock_server)
        .await;

    let client = SpritesClient::with_base_url("test_token".to_string(), mock_server.uri());

    let result = client.exec("err-sprite", "foobar", None).await.unwrap();
    assert_eq!(result.exit_code, 127);
    assert!(result.stderr.contains("command not found"));
}
