//! Integration tests for `resource_discovery` tools against a wiremock ARD
//! registry. Exercises the full `execute_with_context` discover → attach flow,
//! the MCP and A2A attachment kinds, SSRF local-URL blocking, and the trust gate.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use everruns_core::ard_attachment::{ArdAttachment, ArdAttachmentTarget};
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::{
    session_services::KeyInfo, session_services::SecretInfo, session_services::SessionStorageStore,
    tool_context::ToolContext,
};
use everruns_provider::error::Result;
use everruns_provider::typed_id::SessionId;
use serde_json::{Value, json};
use tokio::sync::Mutex;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use everruns_ard::config::ArdConfig;
use everruns_ard::tools::{AttachResourceTool, DiscoverResourcesTool};

// --- in-memory SessionStorageStore mock ---------------------------------------

#[derive(Default)]
struct MockStorage {
    values: Mutex<HashMap<String, String>>,
    secrets: Mutex<HashMap<String, String>>,
}

fn k(session_id: SessionId, key: &str) -> String {
    format!("{session_id}:{key}")
}

#[async_trait]
impl SessionStorageStore for MockStorage {
    async fn set_value(&self, session_id: SessionId, key: &str, value: &str) -> Result<()> {
        self.values
            .lock()
            .await
            .insert(k(session_id, key), value.to_string());
        Ok(())
    }
    async fn get_value(&self, session_id: SessionId, key: &str) -> Result<Option<String>> {
        Ok(self.values.lock().await.get(&k(session_id, key)).cloned())
    }
    async fn delete_value(&self, session_id: SessionId, key: &str) -> Result<bool> {
        Ok(self
            .values
            .lock()
            .await
            .remove(&k(session_id, key))
            .is_some())
    }
    async fn list_keys(&self, session_id: SessionId) -> Result<Vec<KeyInfo>> {
        let prefix = format!("{session_id}:");
        Ok(self
            .values
            .lock()
            .await
            .keys()
            .filter_map(|full| full.strip_prefix(&prefix).map(ToString::to_string))
            .map(|key| KeyInfo {
                key,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
            .collect())
    }
    async fn set_secret(&self, session_id: SessionId, name: &str, value: &str) -> Result<()> {
        self.secrets
            .lock()
            .await
            .insert(k(session_id, name), value.to_string());
        Ok(())
    }
    async fn get_secret(&self, session_id: SessionId, name: &str) -> Result<Option<String>> {
        Ok(self.secrets.lock().await.get(&k(session_id, name)).cloned())
    }
    async fn delete_secret(&self, session_id: SessionId, name: &str) -> Result<bool> {
        Ok(self
            .secrets
            .lock()
            .await
            .remove(&k(session_id, name))
            .is_some())
    }
    async fn list_secrets(&self, _session_id: SessionId) -> Result<Vec<SecretInfo>> {
        Ok(vec![])
    }
}

fn ctx(session_id: SessionId, store: Arc<MockStorage>) -> ToolContext {
    let mut context = ToolContext::with_storage_store(session_id, store);
    context.egress_service = Some(Arc::new(everruns_host::DirectEgressService::new()));
    context
}

fn config_for(registry_url: &str, allow_local: bool, require_trust: Vec<String>) -> ArdConfig {
    ArdConfig::from_value(&json!({
        "registries": [{ "id": "test", "url": registry_url, "federation": "none" }],
        "allow_local_urls": allow_local,
        "require_trust": require_trust,
    }))
    .unwrap()
}

/// Mount a `POST /search` returning the given results, plus artifact docs.
async fn mount_registry(server: &MockServer, results: Value) {
    Mock::given(method("POST"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "results": results })))
        .mount(server)
        .await;
}

fn loaded_attachment(raw: &str) -> ArdAttachment {
    serde_json::from_str(raw).expect("attachment json")
}

#[tokio::test]
async fn discover_then_attach_mcp_server() {
    let server = MockServer::start().await;
    let base = server.uri();

    // MCP catalog entry whose artifact is fetched from the registry.
    mount_registry(
        &server,
        json!([{
            "identifier": "urn:ai:acme.com:mcp:docs",
            "displayName": "Acme Docs MCP",
            "type": "application/mcp-server+json",
            "description": "Documentation search",
            "score": 91,
            "source": base,
            "url": format!("{base}/artifacts/docs-mcp.json"),
        }]),
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/artifacts/docs-mcp.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "url": format!("{base}/mcp"),
            "headers": { "X-Tenant": "acme" }
        })))
        .mount(&server)
        .await;

    let session_id = SessionId::new();
    let store = Arc::new(MockStorage::default());
    let context = ctx(session_id, store.clone());
    let cfg = config_for(&base, true, vec![]);

    let discover = DiscoverResourcesTool::new(cfg.clone());
    let res = discover
        .execute_with_context(json!({ "text": "documentation search" }), &context)
        .await;
    assert!(matches!(res, ToolExecutionResult::Success(_)), "{res:?}");

    let attach = AttachResourceTool::new(cfg);
    let res = attach
        .execute_with_context(json!({ "urn": "urn:ai:acme.com:mcp:docs" }), &context)
        .await;
    let ToolExecutionResult::Success(v) = res else {
        panic!("attach failed: {res:?}");
    };
    assert_eq!(v["status"], "attached");
    assert_eq!(v["kind"], "mcp_server");

    // Assert a session-scoped mcpServers attachment was persisted.
    let keys = store.list_keys(session_id).await.unwrap();
    let attach_key = keys
        .iter()
        .find(|k| k.key.starts_with("ard_attach:"))
        .expect("attachment persisted");
    let raw = store
        .get_value(session_id, &attach_key.key)
        .await
        .unwrap()
        .unwrap();
    let attachment = loaded_attachment(&raw);
    match attachment.target {
        ArdAttachmentTarget::McpServer { name, server } => {
            assert_eq!(name, "docs");
            assert_eq!(server.url, format!("{base}/mcp"));
            assert_eq!(
                server.headers.get("X-Tenant").map(String::as_str),
                Some("acme")
            );
        }
        other => panic!("expected mcp server attachment, got {other:?}"),
    }

    // Idempotent: re-attaching the same URN reports already_attached.
    let res = attach
        .execute_with_context(json!({ "urn": "urn:ai:acme.com:mcp:docs" }), &context)
        .await;
    let ToolExecutionResult::Success(v) = res else {
        panic!("re-attach failed: {res:?}");
    };
    assert_eq!(v["status"], "already_attached");
}

#[tokio::test]
async fn attach_a2a_agent_produces_external_agent() {
    let server = MockServer::start().await;
    let base = server.uri();
    mount_registry(
        &server,
        json!([{
            "identifier": "urn:ai:acme.com:agent:concierge",
            "displayName": "Concierge",
            "type": "application/a2a-agent-card+json",
            "source": base,
            "data": { "name": "Concierge", "url": format!("{base}/a2a") }
        }]),
    )
    .await;

    let session_id = SessionId::new();
    let store = Arc::new(MockStorage::default());
    let context = ctx(session_id, store.clone());
    let cfg = config_for(&base, true, vec![]);

    DiscoverResourcesTool::new(cfg.clone())
        .execute_with_context(json!({ "text": "concierge" }), &context)
        .await;
    let res = AttachResourceTool::new(cfg)
        .execute_with_context(
            json!({ "urn": "urn:ai:acme.com:agent:concierge" }),
            &context,
        )
        .await;
    let ToolExecutionResult::Success(v) = res else {
        panic!("attach failed: {res:?}");
    };
    assert_eq!(v["kind"], "a2a_agent");

    let keys = store.list_keys(session_id).await.unwrap();
    let key = keys
        .iter()
        .find(|k| k.key.starts_with("ard_attach:"))
        .unwrap();
    let raw = store
        .get_value(session_id, &key.key)
        .await
        .unwrap()
        .unwrap();
    match loaded_attachment(&raw).target {
        ArdAttachmentTarget::ExternalAgent { agent } => {
            assert_eq!(agent["id"], "concierge");
            assert_eq!(agent["base_url"], format!("{base}/a2a"));
        }
        other => panic!("expected external agent, got {other:?}"),
    }
}

#[tokio::test]
async fn blocks_local_urls_unless_allowed() {
    let server = MockServer::start().await;
    let base = server.uri();
    mount_registry(
        &server,
        json!([{
            "identifier": "urn:ai:acme.com:mcp:internal",
            "displayName": "Internal",
            "type": "application/mcp-server+json",
            "source": base,
            // Inline data so resolution doesn't need a fetch; endpoint is loopback.
            "data": { "url": "http://127.0.0.1:9/mcp" }
        }]),
    )
    .await;

    let session_id = SessionId::new();
    let store = Arc::new(MockStorage::default());
    let context = ctx(session_id, store.clone());
    // Discover through the local test registry with an explicit local-dev opt-in,
    // then attach with local URLs disabled so the loopback endpoint is rejected.
    let discover_cfg = config_for(&base, true, vec![]);
    let attach_cfg = config_for("https://registry.example.test", false, vec![]);

    DiscoverResourcesTool::new(discover_cfg)
        .execute_with_context(json!({ "text": "internal" }), &context)
        .await;
    let res = AttachResourceTool::new(attach_cfg)
        .execute_with_context(json!({ "urn": "urn:ai:acme.com:mcp:internal" }), &context)
        .await;
    match res {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("SSRF"), "expected SSRF rejection, got: {msg}")
        }
        other => panic!("expected SSRF tool error, got {other:?}"),
    }
    // Nothing was attached.
    let keys = store.list_keys(session_id).await.unwrap();
    assert!(!keys.iter().any(|k| k.key.starts_with("ard_attach:")));
}

#[tokio::test]
async fn trust_gate_rejects_domain_mismatch() {
    let server = MockServer::start().await;
    let base = server.uri();
    mount_registry(
        &server,
        json!([{
            "identifier": "urn:ai:acme.com:mcp:docs",
            "displayName": "Spoofed",
            "type": "application/mcp-server+json",
            "source": base,
            "data": { "url": format!("{base}/mcp") },
            "trustManifest": {
                "identity": "spiffe://evil.com/docs",
                "attestations": [{ "type": "SOC2-Type2" }]
            }
        }]),
    )
    .await;

    let session_id = SessionId::new();
    let store = Arc::new(MockStorage::default());
    let context = ctx(session_id, store.clone());
    let cfg = config_for(&base, true, vec!["soc2".to_string()]);

    DiscoverResourcesTool::new(cfg.clone())
        .execute_with_context(json!({ "text": "docs" }), &context)
        .await;
    let res = AttachResourceTool::new(cfg)
        .execute_with_context(json!({ "urn": "urn:ai:acme.com:mcp:docs" }), &context)
        .await;
    match res {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("Trust verification failed"), "got: {msg}")
        }
        other => panic!("expected trust rejection, got {other:?}"),
    }
}

#[tokio::test]
async fn attach_requires_prior_discovery() {
    let server = MockServer::start().await;
    let cfg = config_for(&server.uri(), true, vec![]);
    let session_id = SessionId::new();
    let store = Arc::new(MockStorage::default());
    let context = ctx(session_id, store);

    let res = AttachResourceTool::new(cfg)
        .execute_with_context(json!({ "urn": "urn:ai:acme.com:mcp:never-seen" }), &context)
        .await;
    match res {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("discover_resources first"), "got: {msg}")
        }
        other => panic!("expected discovery-required error, got {other:?}"),
    }
}
