#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Integration tests: tool execute_with_context against wiremock Sprites API.
//!
//! These tests exercise the full tool execution flow:
//! MockStorageStore → tool.execute_with_context() → SpritesClient → wiremock

use async_trait::async_trait;
use everruns_contracts::error::Result;
use everruns_contracts::runtime::leased_resource::{
    LeasedResource, LeasedResourceStatus, UpsertLeasedResource,
};
use everruns_contracts::runtime::tools::{Tool, ToolExecutionResult};
use everruns_contracts::runtime::{
    connection_services::UserConnectionResolver, session_services::KeyInfo,
    session_services::LeasedResourceStore, session_services::SecretInfo,
    session_services::SessionStorageStore, tool_context::ToolContext,
};
use everruns_contracts::typed_id::{LeasedResourceId, SessionId};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

// Force linker to include the integration crate.
#[allow(unused_imports)]
use everruns_integrations_experimental::sprites as _;

use everruns_integrations_experimental::sprites::client::SpritesClient;
use everruns_integrations_experimental::sprites::state::{
    SpriteState, get_sprite_state, save_sprite_state, touch_sprite_lease,
};

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
// Mock LeasedResourceStore
// ============================================================================

struct MockLeasedResourceStore {
    resources: Mutex<Vec<LeasedResource>>,
}

impl MockLeasedResourceStore {
    fn new() -> Self {
        Self {
            resources: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl LeasedResourceStore for MockLeasedResourceStore {
    async fn upsert_resource(&self, input: UpsertLeasedResource) -> Result<LeasedResource> {
        let now = chrono::Utc::now();
        let resource = LeasedResource {
            id: LeasedResourceId::new(),
            session_id: Some(input.session_id),
            provider: input.provider,
            resource_type: input.resource_type,
            external_id: input.external_id,
            display_name: input.display_name,
            status: LeasedResourceStatus::Active,
            owner_user_id: input.owner_user_id,
            lease_duration_seconds: input.lease_duration_seconds,
            last_touched_at: now,
            lease_expires_at: now
                + chrono::TimeDelta::seconds(i64::from(input.lease_duration_seconds)),
            cleanup_started_at: None,
            cleanup_completed_at: None,
            cleanup_attempts: 0,
            last_cleanup_error: None,
            metadata: input.metadata,
            created_at: now,
            updated_at: now,
        };
        self.resources.lock().await.push(resource.clone());
        Ok(resource)
    }

    async fn release_resource(
        &self,
        session_id: SessionId,
        provider: &str,
        resource_type: &str,
        external_id: &str,
    ) -> Result<Option<LeasedResource>> {
        let mut resources = self.resources.lock().await;
        let resource = resources.iter_mut().find(|resource| {
            resource.session_id == Some(session_id)
                && resource.provider == provider
                && resource.resource_type == resource_type
                && resource.external_id == external_id
        });
        if let Some(resource) = resource {
            resource.status = LeasedResourceStatus::Released;
            resource.updated_at = chrono::Utc::now();
            return Ok(Some(resource.clone()));
        }
        Ok(None)
    }

    async fn list_resources(&self, session_id: SessionId) -> Result<Vec<LeasedResource>> {
        Ok(self
            .resources
            .lock()
            .await
            .iter()
            .filter(|resource| resource.session_id == Some(session_id))
            .cloned()
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
        workspace_path: "/home/sprite".to_string(),
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
    let cap = everruns_integrations_experimental::sprites::SpritesCapability;
    use everruns_contracts::runtime::capabilities::Capability;
    cap.tools()
        .into_iter()
        .find(|t| t.name() == name)
        .unwrap_or_else(|| panic!("Tool {name} not found"))
}

// ============================================================================
// SpritesClient integration tests (wiremock)
// ============================================================================

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
        ToolExecutionResult::ConnectionRequired { provider, .. } => {
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
        workspace_path: "/home/sprite".to_string(),
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
            assert_eq!(sprite["workspace_path"], "/home/sprite");
            assert_eq!(sprite["service_url"], "https://persist-test.fly.dev");
        }
        other => panic!("Expected Success, got: {other:?}"),
    }
}

// ============================================================================
// Bearer auth verification
// ============================================================================

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

#[tokio::test]
async fn test_exec_rejects_path_traversal_name() {
    let tool = get_tool("sprites_exec");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let context = ToolContext::with_storage_store(session_id, store)
        .with_connection_resolver(sprites_resolver());

    let result = tool
        .execute_with_context(
            json!({"sprite_name": "../account", "command": "pwd"}),
            &context,
        )
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
        .and(query_param("cmd", "sh"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![
            0x02, b'b', b'a', b's', b'h', b':', b' ', b'f', b'o', b'o', b'b', b'a', b'r', b':',
            b' ', b'c', b'o', b'm', b'm', b'a', b'n', b'd', b' ', b'n', b'o', b't', b' ', b'f',
            b'o', b'u', b'n', b'd', b'\n', 0x03, 127,
        ]))
        .mount(&mock_server)
        .await;

    let client = SpritesClient::with_base_url("test_token".to_string(), mock_server.uri());

    let result = client.exec("err-sprite", "foobar", None).await.unwrap();
    assert_eq!(result.exit_code, 127);
    assert!(result.stderr.contains("command not found"));
}

// ============================================================================
// Session ownership (EVE-1170)
// ============================================================================

/// A `sprites_sprite:` secret naming a sprite this session never leased must be
/// rejected, while the session's own sprite (registered via the create path's
/// save + lease helpers) stays usable.
#[tokio::test]
async fn test_forged_sprite_state_rejected_own_sprite_usable() {
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let leased_resources = Arc::new(MockLeasedResourceStore::new());
    let context = ToolContext::with_storage_store(session_id, store.clone())
        .with_connection_resolver(sprites_resolver())
        .with_leased_resource_store(leased_resources);

    // Own sprite: same helpers sprites_create_sprite uses.
    let own = SpriteState {
        sprite_name: "own-sprite".to_string(),
        workspace_path: "/home/sprite".to_string(),
        started_at: "2026-03-23T10:00:00Z".to_string(),
        service_url: None,
    };
    save_sprite_state(&context, &own).await.unwrap();
    touch_sprite_lease(&context, &own, Some("own-sprite".to_string()))
        .await
        .unwrap();

    // Forged state for a sprite owned elsewhere, with no lease in this session.
    setup_context_with_sprite(session_id, &store, "foreign-sprite").await;

    let state = get_sprite_state(&context, "own-sprite").await.unwrap();
    assert_eq!(state.sprite_name, "own-sprite");

    match get_sprite_state(&context, "foreign-sprite").await {
        Err(ToolExecutionResult::ToolError(msg)) => {
            assert!(
                msg.contains("was not created by this session"),
                "Got: {msg}"
            );
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }

    // Tools that act on a sprite name reject the forged entry before any API call.
    let exec = get_tool("sprites_exec");
    let result = exec
        .execute_with_context(
            json!({"sprite_name": "foreign-sprite", "command": "ls"}),
            &context,
        )
        .await;
    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(
                msg.contains("was not created by this session"),
                "Got: {msg}"
            );
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }

    // Listing skips the forged entry.
    let list = get_tool("sprites_list_sprites");
    match list.execute_with_context(json!({}), &context).await {
        ToolExecutionResult::Success(output) => {
            assert_eq!(output["count"], 1, "Got: {output}");
            assert_eq!(output["sprites"][0]["sprite_name"], "own-sprite");
        }
        other => panic!("Expected Success, got: {other:?}"),
    }
}
