#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Integration tests: `daytona_api_call`'s ownership-label injection and
//! leased-resource tracking, driven through `execute_with_context` (not the
//! `DaytonaClient` directly), against a wiremock Daytona API.
//!
//! Split out of `tool_integration.rs` (which is on the file-size allowlist)
//! rather than grown in place.

use async_trait::async_trait;
use everruns_core::leased_resource::{LeasedResource, LeasedResourceStatus, UpsertLeasedResource};
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::{
    capabilities::Capability, connection_services::UserConnectionResolver,
    session_services::KeyInfo, session_services::LeasedResourceStore, session_services::SecretInfo,
    session_services::SessionStorageStore, tool_context::ToolContext,
};
use everruns_provider::error::Result;
use everruns_provider::typed_id::{LeasedResourceId, SessionId};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// Force linker to include the integration crate.
use everruns_integrations_daytona as _;

use everruns_integrations_daytona::client_test_override::DaytonaBaseUrlOverride;
use everruns_integrations_daytona::state::SandboxState;

// ============================================================================
// Mocks (mirrors tool_integration.rs's; kept self-contained per test binary)
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
    async fn list_secrets(&self, _session_id: SessionId) -> Result<Vec<SecretInfo>> {
        Ok(vec![])
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
        _session_id: SessionId,
        _provider: &str,
        _resource_type: &str,
        _external_id: &str,
    ) -> Result<Option<LeasedResource>> {
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

fn daytona_resolver() -> Arc<dyn UserConnectionResolver> {
    Arc::new(MockConnectionResolver {
        token: Some("test_api_key".to_string()),
    })
}

fn get_tool(name: &str) -> Box<dyn Tool> {
    let cap = everruns_integrations_daytona::DaytonaCapability;
    // Use tools_with_config to include opt-in tools like daytona_api_call.
    cap.tools_with_config(&json!({"enable_api_calling": true}))
        .into_iter()
        .find(|t| t.name() == name)
        .unwrap_or_else(|| panic!("Tool {name} not found"))
}

/// Build a `daytona_api_call` context wired to `mock_server` (via
/// `DaytonaBaseUrlOverride`), with storage + leased-resource stores so the
/// tool's label-injection and lease-tracking side effects are observable.
fn api_call_context(
    session_id: SessionId,
    storage: Arc<MockStorageStore>,
    leases: Arc<MockLeasedResourceStore>,
    mock_server: &MockServer,
) -> ToolContext {
    ToolContext::with_storage_store(session_id, storage)
        .with_connection_resolver(daytona_resolver())
        .with_leased_resource_store(leases)
        .with_extension(Arc::new(DaytonaBaseUrlOverride {
            api_base: mock_server.uri(),
            toolbox_base: mock_server.uri(),
        }))
}

#[tokio::test]
async fn test_api_call_tool_injects_labels_on_sandbox_create() {
    let mock_server = MockServer::start().await;
    let session_id = SessionId::new();

    // The tool must inject "everruns" ownership labels (merged with any
    // user-supplied labels) into the outgoing POST /sandbox body — asserted
    // via body_partial_json against the actual request the tool sends.
    Mock::given(method("POST"))
        .and(path("/sandbox"))
        .and(wiremock::matchers::body_partial_json(json!({
            "name": "API Created",
            "labels": {
                "everruns": "true",
                "everruns.session_id": session_id.to_string()
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

    let tool = get_tool("daytona_api_call");
    let context = api_call_context(
        session_id,
        Arc::new(MockStorageStore::new()),
        Arc::new(MockLeasedResourceStore::new()),
        &mock_server,
    );

    let result = tool
        .execute_with_context(
            json!({
                "method": "POST",
                "path": "/sandbox",
                "body": {"name": "API Created", "snapshot": "daytona-small"}
            }),
            &context,
        )
        .await;

    match result {
        ToolExecutionResult::Success(response) => {
            assert_eq!(response["id"], "sb_api_created");
        }
        other => panic!("Expected Success, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_api_call_tool_tracks_sandbox_state_on_create() {
    let mock_server = MockServer::start().await;
    let session_id = SessionId::new();

    Mock::given(method("POST"))
        .and(path("/sandbox"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "sb_tracked",
            "name": "Tracked Sandbox",
            "state": "started"
        })))
        .mount(&mock_server)
        .await;

    let tool = get_tool("daytona_api_call");
    let storage = Arc::new(MockStorageStore::new());
    let leases = Arc::new(MockLeasedResourceStore::new());
    let context = api_call_context(session_id, storage.clone(), leases.clone(), &mock_server);

    let result = tool
        .execute_with_context(
            json!({
                "method": "POST",
                "path": "/sandbox",
                "body": {"name": "Tracked Sandbox"}
            }),
            &context,
        )
        .await;

    match result {
        ToolExecutionResult::Success(response) => {
            assert_eq!(response["id"], "sb_tracked");
            assert_eq!(response["state"], "started");
        }
        other => panic!("Expected Success, got: {other:?}"),
    }

    // Sandbox state was persisted (so daytona_exec etc. can find it)...
    let saved = storage
        .get_secret(session_id, "daytona_sandbox:sb_tracked")
        .await
        .unwrap()
        .expect("sandbox state should be saved after api_call create");
    let state: SandboxState = serde_json::from_str(&saved).unwrap();
    assert_eq!(state.sandbox_id, "sb_tracked");

    // ...and a leased-resource entry was registered for cleanup.
    let resources = leases.resources.lock().await;
    assert!(resources.iter().any(|r| r.provider == "daytona"
        && r.resource_type == "sandbox"
        && r.external_id == "sb_tracked"));
}
