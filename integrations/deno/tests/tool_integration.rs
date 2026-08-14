//! Integration tests: tool execute_with_context for Deno sandbox tools.
//!
//! These tests exercise the full tool execution flow through the tool layer:
//! MockStorageStore → tool.execute_with_context() → parameter/state/credential validation.
//!
//! Unlike the unit tests in src/tools.rs (which test schemas and parsing), these tests
//! verify the orchestration between credential resolution, sandbox state persistence,
//! and tool parameter validation through execute_with_context.
//!
//! NOTE: Deno uses websocket-based sandbox communication, so we cannot mock the
//! sandbox API with wiremock (HTTP). These tests cover everything up to the actual
//! websocket connection: credential resolution, state management, and parameter
//! validation — the same surface Daytona's tool_integration.rs covers.

use async_trait::async_trait;
use everruns_core::leased_resource::{LeasedResource, LeasedResourceStatus, UpsertLeasedResource};
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::{
    connection_services::UserConnectionResolver, session_services::KeyInfo,
    session_services::LeasedResourceStore, session_services::SecretInfo,
    session_services::SessionStorageStore, tool_context::ToolContext,
};
use everruns_provider::error::Result;
use everruns_provider::typed_id::{LeasedResourceId, SessionId};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

// Force linker to include the integration crate.
use everruns_integrations_deno as _;

use everruns_integrations_deno::state::SandboxState;

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

struct MockLeasedResourceStore {
    resources: Mutex<Vec<LeasedResource>>,
}

impl MockLeasedResourceStore {
    fn new() -> Self {
        Self {
            resources: Mutex::new(Vec::new()),
        }
    }

    async fn seed_active(
        &self,
        session_id: SessionId,
        provider: &str,
        resource_type: &str,
        external_id: &str,
    ) {
        let now = chrono::Utc::now();
        self.resources.lock().await.push(LeasedResource {
            id: LeasedResourceId::new(),
            session_id: Some(session_id),
            provider: provider.to_string(),
            resource_type: resource_type.to_string(),
            external_id: external_id.to_string(),
            display_name: Some(external_id.to_string()),
            status: LeasedResourceStatus::Active,
            owner_user_id: None,
            lease_duration_seconds: 1200,
            last_touched_at: now,
            lease_expires_at: now + chrono::TimeDelta::seconds(1200),
            cleanup_started_at: None,
            cleanup_completed_at: None,
            cleanup_attempts: 0,
            last_cleanup_error: None,
            metadata: json!({}),
            created_at: now,
            updated_at: now,
        });
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

/// Create a mock connection resolver that returns a fixed Deno access token.
fn deno_resolver() -> Arc<dyn UserConnectionResolver> {
    Arc::new(MockConnectionResolver {
        token: Some("ddo_test_token".to_string()),
    })
}

// ============================================================================
// Helpers
// ============================================================================

/// Seed sandbox state into the store.
async fn setup_context_with_sandbox(
    session_id: SessionId,
    store: &Arc<MockStorageStore>,
    sandbox_id: &str,
) {
    let state = SandboxState {
        sandbox_id: sandbox_id.to_string(),
        region: "ord".to_string(),
        org: None,
        workspace_path: "/home/app".to_string(),
        started_at: "2026-03-22T10:00:00Z".to_string(),
    };
    store
        .seed_secret(
            session_id,
            &format!("deno_sandbox:{sandbox_id}"),
            &serde_json::to_string(&state).unwrap(),
        )
        .await;
}

fn get_tool(name: &str) -> Box<dyn Tool> {
    let cap = everruns_integrations_deno::DenoCapability;
    use everruns_core::capabilities::Capability;
    cap.tools()
        .into_iter()
        .find(|t| t.name() == name)
        .unwrap_or_else(|| panic!("Tool {name} not found"))
}

// ============================================================================
// Credential resolution tests
// ============================================================================

#[tokio::test]
async fn test_exec_tool_missing_api_key() {
    let tool = get_tool("deno_exec");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let context = ToolContext::with_storage_store(session_id, store);

    let result = tool
        .execute_with_context(json!({"sandbox_id": "sb_test", "command": "ls"}), &context)
        .await;

    match result {
        ToolExecutionResult::ConnectionRequired { provider } => {
            assert_eq!(provider, "deno");
        }
        other => panic!("Expected ConnectionRequired, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_create_tool_missing_api_key() {
    let tool = get_tool("deno_create_sandbox");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let context = ToolContext::with_storage_store(session_id, store);

    let result = tool.execute_with_context(json!({}), &context).await;

    match result {
        ToolExecutionResult::ConnectionRequired { provider } => {
            assert_eq!(provider, "deno");
        }
        other => panic!("Expected ConnectionRequired, got: {other:?}"),
    }
}

// ============================================================================
// Sandbox state tests
// ============================================================================

#[tokio::test]
async fn test_exec_tool_missing_sandbox_state() {
    let tool = get_tool("deno_exec");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let context = ToolContext::with_storage_store(session_id, store)
        .with_connection_resolver(deno_resolver());

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
async fn test_exec_tool_rejects_cross_session_sandbox_id() {
    let tool = get_tool("deno_exec");
    let owner_session = SessionId::new();
    let attacker_session = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let leased_resources = Arc::new(MockLeasedResourceStore::new());
    leased_resources
        .seed_active(owner_session, "deno", "sandbox", "sb_foreign")
        .await;

    let context = ToolContext::with_storage_store(attacker_session, store)
        .with_connection_resolver(deno_resolver())
        .with_leased_resource_store(leased_resources);

    let result = tool
        .execute_with_context(
            json!({"sandbox_id": "sb_foreign", "command": "ls"}),
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
}

#[tokio::test]
async fn test_read_file_tool_missing_sandbox_state() {
    let tool = get_tool("deno_read_file");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let context = ToolContext::with_storage_store(session_id, store)
        .with_connection_resolver(deno_resolver());

    let result = tool
        .execute_with_context(
            json!({"sandbox_id": "sb_missing", "path": "/test.txt"}),
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

// ============================================================================
// Parameter validation tests
// ============================================================================

#[tokio::test]
async fn test_exec_tool_missing_command_param() {
    let tool = get_tool("deno_exec");
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
async fn test_exec_tool_missing_sandbox_id_param() {
    let tool = get_tool("deno_exec");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
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
    let tool = get_tool("deno_read_file");
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
    let tool = get_tool("deno_write_file");
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
async fn test_manage_sandbox_unsupported_action() {
    let tool = get_tool("deno_manage_sandbox");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    setup_context_with_sandbox(session_id, &store, "sb_test").await;
    let context = ToolContext::with_storage_store(session_id, store)
        .with_connection_resolver(deno_resolver());

    let result = tool
        .execute_with_context(json!({"sandbox_id": "sb_test", "action": "stop"}), &context)
        .await;

    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("Unsupported action"), "Got: {msg}");
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_create_sandbox_rejects_session_timeout() {
    let tool = get_tool("deno_create_sandbox");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let context = ToolContext::with_storage_store(session_id, store)
        .with_connection_resolver(deno_resolver());

    let result = tool
        .execute_with_context(json!({"timeout": "session"}), &context)
        .await;

    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("session"), "Got: {msg}");
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_create_sandbox_rejects_invalid_memory() {
    let tool = get_tool("deno_create_sandbox");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let context = ToolContext::with_storage_store(session_id, store)
        .with_connection_resolver(deno_resolver());

    let result = tool
        .execute_with_context(json!({"memory_mb": 0}), &context)
        .await;

    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("memory_mb"), "Got: {msg}");
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_create_sandbox_rejects_string_memory() {
    let tool = get_tool("deno_create_sandbox");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());
    let context = ToolContext::with_storage_store(session_id, store)
        .with_connection_resolver(deno_resolver());

    let result = tool
        .execute_with_context(json!({"memory_mb": "big"}), &context)
        .await;

    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("memory_mb"), "Got: {msg}");
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

// ============================================================================
// List sandboxes state tests
// ============================================================================

#[tokio::test]
async fn test_list_sandboxes_empty() {
    let tool = get_tool("deno_list_sandboxes");
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
    let tool = get_tool("deno_list_sandboxes");
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());

    // Seed two sandbox states
    setup_context_with_sandbox(session_id, &store, "sb_one").await;
    let state2 = SandboxState {
        sandbox_id: "sb_two".to_string(),
        region: "ams".to_string(),
        org: Some("test-org".to_string()),
        workspace_path: "/home/app".to_string(),
        started_at: "2026-03-22T11:00:00Z".to_string(),
    };
    store
        .seed_secret(
            session_id,
            "deno_sandbox:sb_two",
            &serde_json::to_string(&state2).unwrap(),
        )
        .await;

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
// State persistence roundtrip test
// ============================================================================

#[tokio::test]
async fn test_sandbox_state_persistence_roundtrip() {
    let session_id = SessionId::new();
    let store = Arc::new(MockStorageStore::new());

    setup_context_with_sandbox(session_id, &store, "sb_persist").await;

    let context = ToolContext::with_storage_store(session_id, store.clone());

    let tool = get_tool("deno_list_sandboxes");
    let result = tool.execute_with_context(json!({}), &context).await;

    match result {
        ToolExecutionResult::Success(output) => {
            assert_eq!(output["count"], 1);
            let sandbox = &output["sandboxes"][0];
            assert_eq!(sandbox["sandbox_id"], "sb_persist");
            assert_eq!(sandbox["region"], "ord");
            assert_eq!(sandbox["workspace_path"], "/home/app");
        }
        other => panic!("Expected Success, got: {other:?}"),
    }
}
