//! Integration tests for the ARD client tools against a wiremock registry.
//!
//! Covers: discover ranking, MCP attach -> `mcp_server` resource, A2A attach ->
//! `external_a2a_agent` resource, local-URL blocking (and the opt-in bypass),
//! trust-gate rejection, idempotency, and the `max_attachments` cap.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::sync::Mutex;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use everruns_core::error::Result;
use everruns_core::mcp_server::ScopedMcpServers;
use everruns_core::session::Session;
use everruns_core::session_resource::{
    RegisterSessionResource, SessionResourceEntry, SessionResourceFilter, SessionResourceStatus,
};
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::{SessionMutator, SessionResourceRegistry, ToolContext};
use everruns_core::typed_id::SessionId;

use everruns_integrations_ard::config::{RegistryConfig, ResourceDiscoveryConfig};

// ---------------------------------------------------------------------------
// Mock SessionMutator — records upsert_session_mcp_servers calls (EVE-593)
// ---------------------------------------------------------------------------

#[derive(Default)]
struct MockMutator {
    /// Each recorded upsert overlay, in call order.
    upserts: Mutex<Vec<ScopedMcpServers>>,
}

#[async_trait]
impl SessionMutator for MockMutator {
    async fn update_session_title(
        &self,
        _session_id: SessionId,
        _title: String,
    ) -> Result<Session> {
        unreachable!("attach_resource does not set titles")
    }

    async fn upsert_session_mcp_servers(
        &self,
        _session_id: SessionId,
        servers: ScopedMcpServers,
    ) -> Result<()> {
        self.upserts.lock().await.push(servers);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Mock SessionResourceRegistry
// ---------------------------------------------------------------------------

#[derive(Default)]
struct MockRegistry {
    entries: Mutex<HashMap<String, SessionResourceEntry>>,
}

#[async_trait]
impl SessionResourceRegistry for MockRegistry {
    async fn register(&self, entry: RegisterSessionResource) -> Result<SessionResourceEntry> {
        let now = chrono::Utc::now();
        let stored = SessionResourceEntry {
            resource_id: entry.resource_id.clone(),
            session_id: entry.session_id,
            kind: entry.kind,
            display_name: entry.display_name,
            status: entry.status,
            metadata: entry.metadata,
            created_at: now,
            updated_at: now,
        };
        self.entries
            .lock()
            .await
            .insert(entry.resource_id, stored.clone());
        Ok(stored)
    }

    async fn update_status(
        &self,
        _session_id: SessionId,
        resource_id: &str,
        status: SessionResourceStatus,
    ) -> Result<Option<SessionResourceEntry>> {
        let mut map = self.entries.lock().await;
        if let Some(e) = map.get_mut(resource_id) {
            e.status = status;
            return Ok(Some(e.clone()));
        }
        Ok(None)
    }

    async fn get(
        &self,
        _session_id: SessionId,
        resource_id: &str,
    ) -> Result<Option<SessionResourceEntry>> {
        Ok(self.entries.lock().await.get(resource_id).cloned())
    }

    async fn list(
        &self,
        _session_id: SessionId,
        filter: Option<&SessionResourceFilter>,
    ) -> Result<Vec<SessionResourceEntry>> {
        let map = self.entries.lock().await;
        Ok(map
            .values()
            .filter(|e| match filter {
                Some(f) => f.kind.as_deref().is_none_or(|k| k == e.kind),
                None => true,
            })
            .cloned()
            .collect())
    }

    async fn deregister(&self, _session_id: SessionId, resource_id: &str) -> Result<bool> {
        Ok(self.entries.lock().await.remove(resource_id).is_some())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn config_for(
    server_uri: &str,
    allow_local_urls: bool,
    require_trust: bool,
) -> ResourceDiscoveryConfig {
    ResourceDiscoveryConfig {
        registries: vec![RegistryConfig {
            id: "main".into(),
            url: server_uri.to_string(),
            federation: vec![],
        }],
        require_trust,
        allow_attach_types: vec![],
        max_attachments: 8,
        allow_local_urls,
    }
}

fn context_with(registry: Arc<MockRegistry>) -> ToolContext {
    ToolContext::new(SessionId::new()).with_session_resource_registry(registry)
}

fn context_with_mutator(registry: Arc<MockRegistry>, mutator: Arc<MockMutator>) -> ToolContext {
    ToolContext::new(SessionId::new())
        .with_session_resource_registry(registry)
        .with_session_mutator(mutator)
}

async fn mount_search(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [
                {
                    "urn": "urn:ard:example.com:weather-mcp",
                    "display_name": "Weather MCP",
                    "type": "application/mcp-server+json",
                    "score": 0.92,
                    "description": "Weather data via MCP"
                },
                {
                    "urn": "urn:ard:example.com:research-agent",
                    "display_name": "Research Agent",
                    "type": "application/a2a-agent-card+json",
                    "score": 0.71,
                    "description": "External research agent"
                }
            ]
        })))
        .mount(server)
        .await;
}

fn discover(cfg: ResourceDiscoveryConfig) -> impl Tool {
    everruns_integrations_ard::test_support::discover_tool(cfg)
}

fn attach(cfg: ResourceDiscoveryConfig) -> impl Tool {
    everruns_integrations_ard::test_support::attach_tool(cfg)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn discover_returns_ranked_results() {
    let server = MockServer::start().await;
    mount_search(&server).await;

    let cfg = config_for(&server.uri(), true, true);
    let tool = discover(cfg);
    let ctx = context_with(Arc::new(MockRegistry::default()));

    let result = tool
        .execute_with_context(json!({ "text": "weather" }), &ctx)
        .await;
    let value = expect_success(result);
    assert_eq!(value["count"], 2);
    let results = value["results"].as_array().unwrap();
    assert_eq!(results[0]["urn"], "urn:ard:example.com:weather-mcp");
    assert_eq!(results[0]["source"], "main");
    assert!(results[0]["score"].as_f64().unwrap() > results[1]["score"].as_f64().unwrap());
}

#[tokio::test]
async fn attach_mcp_creates_mcp_server_resource() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/resolve"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "urn": "urn:ard:example.com:weather-mcp",
            "display_name": "Weather MCP",
            "type": "application/mcp-server+json",
            "url": "https://mcp.example.com/v1/mcp",
            "trustManifest": { "identity": "example.com", "attestation": "sig" }
        })))
        .mount(&server)
        .await;

    let registry = Arc::new(MockRegistry::default());
    let mutator = Arc::new(MockMutator::default());
    let cfg = config_for(&server.uri(), true, true);
    let tool = attach(cfg);
    let ctx = context_with_mutator(registry.clone(), mutator.clone());

    let result = tool
        .execute_with_context(json!({ "urn": "urn:ard:example.com:weather-mcp" }), &ctx)
        .await;
    let value = expect_success(result);
    assert_eq!(value["status"], "attached");
    assert_eq!(value["kind"], "mcp_server");
    // EVE-593: an MCP attach makes the server callable next turn.
    assert_eq!(value["callable_next_turn"], true);

    let stored = registry
        .get(ctx.session_id, "urn:ard:example.com:weather-mcp")
        .await
        .unwrap()
        .expect("resource recorded");
    assert_eq!(stored.kind, "mcp_server");

    // Exactly one upsert, carrying the SSRF-validated URL and a sane name.
    let upserts = mutator.upserts.lock().await;
    assert_eq!(upserts.len(), 1, "exactly one upsert call");
    let overlay = &upserts[0];
    assert_eq!(overlay.len(), 1);
    let (name, scoped) = overlay.iter().next().unwrap();
    assert!(name.starts_with("ard_"), "logical name: {name}");
    assert!(
        !everruns_core::mcp_server::sanitize_mcp_server_name(name).contains("__"),
        "name {name} would be rejected by validation"
    );
    assert_eq!(scoped.url, "https://mcp.example.com/v1/mcp");
}

#[tokio::test]
async fn attach_a2a_creates_external_agent_resource() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/resolve"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "urn": "urn:ard:example.com:research-agent",
            "display_name": "Research Agent",
            "type": "application/a2a-agent-card+json",
            "data": {
                "name": "Research Agent",
                "supported_interfaces": [{ "url": "https://agent.example.com/jsonrpc", "type": "JSONRPC" }]
            },
            "trustManifest": { "identity": "example.com", "attestation": "sig" }
        })))
        .mount(&server)
        .await;

    let registry = Arc::new(MockRegistry::default());
    let mutator = Arc::new(MockMutator::default());
    let cfg = config_for(&server.uri(), true, true);
    let tool = attach(cfg);
    let ctx = context_with_mutator(registry.clone(), mutator.clone());

    let result = tool
        .execute_with_context(json!({ "urn": "urn:ard:example.com:research-agent" }), &ctx)
        .await;
    let value = expect_success(result);
    assert_eq!(value["kind"], "a2a_agent");
    // A2A attach is visibility-only for now — no MCP overlay upsert.
    assert_eq!(value["callable_next_turn"], false);
    assert!(
        mutator.upserts.lock().await.is_empty(),
        "A2A attach must not upsert MCP servers"
    );

    let stored = registry
        .get(ctx.session_id, "urn:ard:example.com:research-agent")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.kind, "external_a2a_agent");
}

#[tokio::test]
async fn attach_blocks_local_url_unless_allowed() {
    let make_server = || async {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/resolve"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "urn": "urn:ard:example.com:local-mcp",
                "type": "application/mcp-server+json",
                "url": "http://127.0.0.1:9000/mcp",
                "trustManifest": { "identity": "example.com", "attestation": "sig" }
            })))
            .mount(&server)
            .await;
        server
    };

    // Blocked by default. allow_local_urls must still be true for the *registry*
    // base (wiremock is on localhost); the resolved resource URL is what we test.
    let server = make_server().await;
    let cfg = config_for(&server.uri(), false, true);
    // Registry base on localhost would itself be blocked, so this exercises the
    // registry-side block. Verify it errors.
    let blocked = attach(cfg)
        .execute_with_context(
            json!({ "urn": "urn:ard:example.com:local-mcp" }),
            &context_with(Arc::new(MockRegistry::default())),
        )
        .await;
    assert!(matches!(blocked, ToolExecutionResult::ToolError(_)));

    // With allow_local_urls = true, registry + resolved local URL both pass.
    let server2 = make_server().await;
    let cfg2 = config_for(&server2.uri(), true, true);
    let ok = attach(cfg2)
        .execute_with_context(
            json!({ "urn": "urn:ard:example.com:local-mcp" }),
            &context_with_mutator(
                Arc::new(MockRegistry::default()),
                Arc::new(MockMutator::default()),
            ),
        )
        .await;
    let value = expect_success(ok);
    assert_eq!(value["status"], "attached");
}

#[tokio::test]
async fn attach_with_unsafe_url_does_not_upsert() {
    // SSRF write-time validation rejects a non-local-but-disallowed scheme
    // before any registry record or MCP overlay upsert happens.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/resolve"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "urn": "urn:ard:example.com:bad-scheme",
            "type": "application/mcp-server+json",
            "url": "file:///etc/passwd",
            "trustManifest": { "identity": "example.com", "attestation": "sig" }
        })))
        .mount(&server)
        .await;

    let registry = Arc::new(MockRegistry::default());
    let mutator = Arc::new(MockMutator::default());
    let cfg = config_for(&server.uri(), true, true);
    let result = attach(cfg)
        .execute_with_context(
            json!({ "urn": "urn:ard:example.com:bad-scheme" }),
            &context_with_mutator(registry.clone(), mutator.clone()),
        )
        .await;
    assert!(matches!(result, ToolExecutionResult::ToolError(_)));
    assert!(
        mutator.upserts.lock().await.is_empty(),
        "unsafe URL must not upsert"
    );
}

#[tokio::test]
async fn attach_mcp_without_mutator_errors_and_does_not_attach() {
    // When the host did not supply a SessionMutator, an MCP attach must fail
    // loudly rather than silently recording a non-callable resource.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/resolve"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "urn": "urn:ard:example.com:weather-mcp",
            "type": "application/mcp-server+json",
            "url": "https://mcp.example.com/v1/mcp",
            "trustManifest": { "identity": "example.com", "attestation": "sig" }
        })))
        .mount(&server)
        .await;

    let registry = Arc::new(MockRegistry::default());
    let cfg = config_for(&server.uri(), true, true);
    // context_with does NOT attach a session_mutator.
    let result = attach(cfg)
        .execute_with_context(
            json!({ "urn": "urn:ard:example.com:weather-mcp" }),
            &context_with(registry.clone()),
        )
        .await;
    assert!(
        matches!(result, ToolExecutionResult::InternalError(_)),
        "expected internal error when mutator is unavailable, got {result:?}"
    );
}

#[tokio::test]
async fn attach_rejected_when_trust_missing_and_required() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/resolve"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "urn": "urn:ard:example.com:untrusted",
            "type": "application/mcp-server+json",
            "url": "https://mcp.example.com/v1/mcp"
        })))
        .mount(&server)
        .await;

    let cfg = config_for(&server.uri(), true, true); // require_trust = true
    let result = attach(cfg)
        .execute_with_context(
            json!({ "urn": "urn:ard:example.com:untrusted" }),
            &context_with(Arc::new(MockRegistry::default())),
        )
        .await;
    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.to_lowercase().contains("trust"), "got: {msg}");
        }
        other => panic!("expected trust rejection, got {other:?}"),
    }
}

#[tokio::test]
async fn attach_is_idempotent_per_urn() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/resolve"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "urn": "urn:ard:example.com:weather-mcp",
            "type": "application/mcp-server+json",
            "url": "https://mcp.example.com/v1/mcp",
            "trustManifest": { "identity": "example.com", "attestation": "sig" }
        })))
        .mount(&server)
        .await;

    let registry = Arc::new(MockRegistry::default());
    let mutator = Arc::new(MockMutator::default());
    let cfg = config_for(&server.uri(), true, true);
    let ctx = context_with_mutator(registry.clone(), mutator.clone());

    let first = attach(cfg.clone())
        .execute_with_context(json!({ "urn": "urn:ard:example.com:weather-mcp" }), &ctx)
        .await;
    assert_eq!(expect_success(first)["status"], "attached");

    let second = attach(cfg)
        .execute_with_context(json!({ "urn": "urn:ard:example.com:weather-mcp" }), &ctx)
        .await;
    assert_eq!(expect_success(second)["status"], "already_attached");

    let all = registry.list(ctx.session_id, None).await.unwrap();
    assert_eq!(all.len(), 1);
}

#[tokio::test]
async fn attach_enforces_max_attachments() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/resolve"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "urn": "urn:ard:example.com:second-mcp",
            "type": "application/mcp-server+json",
            "url": "https://mcp.example.com/v1/mcp",
            "trustManifest": { "identity": "example.com", "attestation": "sig" }
        })))
        .mount(&server)
        .await;

    let registry = Arc::new(MockRegistry::default());
    // Pre-seed one attachment so the cap of 1 is already hit.
    registry
        .register(RegisterSessionResource {
            session_id: SessionId::new(),
            resource_id: "urn:ard:example.com:first-mcp".into(),
            kind: "mcp_server".into(),
            display_name: "First".into(),
            status: SessionResourceStatus::Active,
            metadata: json!({}),
        })
        .await
        .unwrap();

    let mut cfg = config_for(&server.uri(), true, true);
    cfg.max_attachments = 1;
    let ctx = context_with(registry.clone());

    let result = attach(cfg)
        .execute_with_context(json!({ "urn": "urn:ard:example.com:second-mcp" }), &ctx)
        .await;
    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("limit"), "got: {msg}");
        }
        other => panic!("expected cap rejection, got {other:?}"),
    }
}

fn expect_success(result: ToolExecutionResult) -> Value {
    match result {
        ToolExecutionResult::Success(v) => v,
        other => panic!("expected success, got {other:?}"),
    }
}
