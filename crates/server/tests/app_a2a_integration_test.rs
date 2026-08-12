//! Integration tests for the App A2A (Agent2Agent) channel.

mod test_harness;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use axum::{
    Json, Router,
    http::{Method, StatusCode},
    routing::get,
};
use everruns_core::DEFAULT_ORG_ID;
use everruns_core::capabilities::Capability;
use everruns_core::tools::ToolExecutionResult;
use everruns_core::traits::{KeyInfo, SecretInfo, SessionStorageStore, ToolContext};
use everruns_core::typed_id::SessionId;
use everruns_platform::capabilities::A2aAgentDelegationCapability;
use everruns_server::storage::models::{AuditLogQuery, AuditLogRow};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use test_harness::TestServer;
use tokio::time::{Duration, sleep};

/// Compute the A2A request signature for tests.
///
/// Basestring is `v0:{ts_secs}:{channel_scope}:{body}` (Slack-derived but
/// scope-bound — see `crates/server/src/api/a2a_signing.rs` for the full
/// rationale). `channel_scope` is the same `{app_id}:{channel_id}` value
/// the server uses to bind the signature to the target endpoint.
fn a2a_sign(secret: &str, ts_secs: i64, channel_scope: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(format!("v0:{ts_secs}:{channel_scope}:").as_bytes());
    mac.update(body);
    format!("v0={}", hex::encode(mac.finalize().into_bytes()))
}

fn a2a_now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

async fn create_app_with_a2a(server: &TestServer, name: &str, message: &str) -> (Value, String) {
    create_app_with_a2a_mode(server, name, message, "shared_session").await
}

async fn create_app_with_a2a_mode(
    server: &TestServer,
    name: &str,
    message: &str,
    session_mode: &str,
) -> (Value, String) {
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": format!("{name}-agent"),
                "display_name": format!("{name} agent"),
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let app: Value = server
        .post(
            "/v1/apps",
            json!({
                "name": name,
                "harness_id": server.seed_generic_harness_id.clone(),
                "agent_id": agent["id"],
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let app_id = app["id"].as_str().unwrap().to_string();
    let response: Value = server
        .post(
            &format!("/v1/apps/{app_id}/a2a-channels"),
            json!({
                "session_mode": session_mode,
                "message": message,
                "agent_card_name": "Inbox triage",
                "agent_card_description": "Triages inbound A2A traffic",
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let api_key = response["api_key"].as_str().unwrap().to_string();
    let app_after: Value = server
        .get(&format!("/v1/apps/{app_id}"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    (app_after, api_key)
}

async fn publish_app(server: &TestServer, app_id: &str) {
    server
        .post(&format!("/v1/apps/{app_id}/publish"), json!({}))
        .await
        .assert_status(StatusCode::OK);
}

async fn list_user_message_texts(server: &TestServer, session_id: &str) -> Vec<String> {
    let body: Value = server
        .get(&format!("/v1/sessions/{session_id}/messages"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    body["data"]
        .as_array()
        .expect("messages array")
        .iter()
        .filter(|m| m["role"].as_str() == Some("user"))
        .filter_map(|m| {
            m["content"].as_array().and_then(|parts| {
                parts.iter().find_map(|p| {
                    (p["type"].as_str() == Some("text"))
                        .then(|| p["text"].as_str().map(str::to_owned))
                        .flatten()
                })
            })
        })
        .collect()
}

async fn wait_for_app_invocation_audit_log(
    server: &TestServer,
    channel_id: &str,
    session_id: &str,
) -> AuditLogRow {
    for _ in 0..20 {
        let rows = server
            .db
            .list_audit_logs(AuditLogQuery {
                org_id: DEFAULT_ORG_ID,
                limit: 50,
                action: Some("agent.app_invocation.started"),
                ..Default::default()
            })
            .await
            .expect("list audit logs");
        if let Some(row) = rows.into_iter().find(|row| {
            row.target_type.as_deref() == Some("app_channel")
                && row.target_id.as_deref() == Some(channel_id)
                && row.metadata.get("session_id").and_then(Value::as_str) == Some(session_id)
        }) {
            return row;
        }
        sleep(Duration::from_millis(25)).await;
    }

    panic!("expected app invocation audit log for channel {channel_id} session {session_id}");
}

#[derive(Default)]
struct TestStorageStore {
    values: Mutex<HashMap<String, String>>,
}

impl TestStorageStore {
    /// Parse the stored agent-run record for a run_id, if present.
    fn run_record(&self, run_id: &str) -> Option<Value> {
        self.values
            .lock()
            .unwrap()
            .get(&format!("agent_run:{run_id}"))
            .and_then(|raw| serde_json::from_str(raw).ok())
    }
}

#[async_trait]
impl SessionStorageStore for TestStorageStore {
    async fn set_value(
        &self,
        _session_id: SessionId,
        key: &str,
        value: &str,
    ) -> everruns_core::Result<()> {
        self.values
            .lock()
            .unwrap()
            .insert(key.to_string(), value.to_string());
        Ok(())
    }

    async fn get_value(
        &self,
        _session_id: SessionId,
        key: &str,
    ) -> everruns_core::Result<Option<String>> {
        Ok(self.values.lock().unwrap().get(key).cloned())
    }

    async fn delete_value(&self, _session_id: SessionId, key: &str) -> everruns_core::Result<bool> {
        Ok(self.values.lock().unwrap().remove(key).is_some())
    }

    async fn list_keys(&self, _session_id: SessionId) -> everruns_core::Result<Vec<KeyInfo>> {
        let now = chrono::Utc::now();
        Ok(self
            .values
            .lock()
            .unwrap()
            .keys()
            .map(|key| KeyInfo {
                key: key.clone(),
                created_at: now,
                updated_at: now,
            })
            .collect())
    }

    async fn set_secret(
        &self,
        _session_id: SessionId,
        _name: &str,
        _value: &str,
    ) -> everruns_core::Result<()> {
        Ok(())
    }

    async fn get_secret(
        &self,
        _session_id: SessionId,
        _name: &str,
    ) -> everruns_core::Result<Option<String>> {
        Ok(None)
    }

    async fn delete_secret(
        &self,
        _session_id: SessionId,
        _name: &str,
    ) -> everruns_core::Result<bool> {
        Ok(false)
    }

    async fn list_secrets(&self, _session_id: SessionId) -> everruns_core::Result<Vec<SecretInfo>> {
        Ok(Vec::new())
    }
}

fn a2a_agent_card(endpoint: &str) -> Value {
    json!({
        "name": "Local Everruns A2A app",
        "description": "Local A2A app endpoint used by outbound delegation tests",
        "version": "0.1",
        "protocolVersion": "0.3.0",
        "preferredTransport": "JSONRPC",
        "supportedInterfaces": [
            {
                "url": endpoint,
                "protocolBinding": "JSONRPC",
                "protocolVersion": "1.0"
            }
        ],
        "capabilities": {
            "streaming": false,
            "pushNotifications": false,
            "stateTransitionHistory": false
        },
        "defaultInputModes": ["text/plain"],
        "defaultOutputModes": ["text/plain"],
        "skills": []
    })
}

async fn spawn_agent_card_server(card: Value) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind agent-card server");
    let addr = listener.local_addr().expect("agent-card server addr");
    let app = Router::new().route(
        "/.well-known/agent-card.json",
        get(move || {
            let card = card.clone();
            async move { Json(card) }
        }),
    );
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve agent-card server");
    });
    format!("http://{addr}")
}

async fn create_published_served_a2a_app() -> (TestServer, String, String, String, String) {
    let (server, base_url) = TestServer::serving_in_memory().await;
    let (app, api_key) = create_app_with_a2a(&server, "a2a-outbound-local", "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap().to_string();
    let channel_id = app["channels"][0]["id"].as_str().unwrap().to_string();
    publish_app(&server, &app_id).await;
    let endpoint = format!("{base_url}/api/v1/apps/{app_id}/a2a/{channel_id}");
    (server, endpoint, api_key, app_id, channel_id)
}

fn outbound_delegation_config(
    endpoint: &str,
    api_key: &str,
    discovery_base_url: Option<&str>,
) -> Value {
    let mut agent = json!({
        "id": "local_app",
        "name": "Local App",
        "description": "Local Everruns A2A app endpoint",
        "headers": {
            "authorization": format!("Bearer {api_key}")
        },
        "preferred_binding": "JSONRPC",
        "poll_interval_ms": 100,
        "allow_local_urls": true
    });
    if let Some(base_url) = discovery_base_url {
        agent["base_url"] = json!(base_url);
    } else {
        agent["agent_card"] = a2a_agent_card(endpoint);
    }
    json!({ "agents": [agent] })
}

async fn spawn_background_against_local_a2a(config: Value) -> (Arc<TestStorageStore>, Value) {
    let capability = A2aAgentDelegationCapability;
    // EVE-885: delegation providers expose their tool through the neutral
    // router seam. Collection merges every provider into one model-facing
    // `spawn_agent`; this test drives the A2A provider tool directly.
    let spawn_tool = capability
        .delegation_target_with_config(&config)
        .expect("a2a delegation target provider")
        .tool;
    let storage = Arc::new(TestStorageStore::default());
    // Background spawns are now required to be task-backed (so they can be
    // controlled via wait_task/message_task/cancel_task), so the ToolContext
    // must carry a session-task registry. An in-memory one is enough here.
    let task_registry: Arc<dyn everruns_core::session_task::SessionTaskRegistry> =
        Arc::new(everruns_server::storage::DbSessionTaskRegistry::new(
            Arc::new(everruns_server::storage::StorageBackend::in_memory()),
        ));
    let ctx = ToolContext::with_storage_store(SessionId::new(), storage.clone())
        .with_session_task_registry(task_registry);

    let result = spawn_tool
        .execute_with_context(
            json!({
                "instructions": "hello from outbound delegation",
                "target": {"type": "external_a2a", "id": "local_app"},
                "mode": "background",
                "wait_timeout_secs": 5,
                "wake_on_completion": false
            }),
            &ctx,
        )
        .await;

    let ToolExecutionResult::Success(value) = result else {
        panic!("expected successful spawn_agent result: {result:?}");
    };
    (storage, value)
}

async fn wait_for_remote_task_snapshot(storage: &TestStorageStore, run_id: &str) -> Value {
    for _ in 0..30 {
        if let Some(metadata) = storage.run_record(run_id) {
            assert_ne!(
                metadata["status"], "failed",
                "agent_run failed before remote task snapshot was observed: {metadata:?}",
            );
            if metadata["last_remote_task_snapshot"].is_object() {
                return metadata;
            }
        }
        sleep(Duration::from_millis(50)).await;
    }
    panic!("expected background monitor to record a remote task snapshot for {run_id}");
}

#[tokio::test]
async fn a2a_message_send_creates_session_and_returns_submitted_task() {
    let server = TestServer::in_memory().await;
    let (app, api_key) = create_app_with_a2a(&server, "a2a-shared", "from a2a: {{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "req-1",
        "method": "message/send",
        "params": {
            "message": {
                "role": "user",
                "messageId": "msg-1",
                "parts": [{ "kind": "text", "text": "hello" }]
            }
        }
    }))
    .unwrap();

    let response: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            body.clone(),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], "req-1");
    // Tasks are async; the dispatch returns immediately with state=submitted
    // and task_id == contextId == session_id. Subsequent `tasks/get` calls
    // observe transitions to working/completed/failed/canceled as the
    // durable runtime emits turn lifecycle events.
    assert_eq!(response["result"]["status"]["state"], "submitted");
    assert_eq!(response["result"]["kind"], "task");
    let session_id = response["result"]["contextId"].as_str().unwrap();
    assert_eq!(response["result"]["id"].as_str().unwrap(), session_id);
    let texts = list_user_message_texts(&server, session_id).await;
    assert!(texts.iter().any(|t| t == "from a2a: hello"));

    // Second invocation in shared_session reuses the same session.
    let body2 = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "req-2",
        "method": "message/send",
        "params": {
            "message": {
                "role": "user",
                "messageId": "msg-2",
                "parts": [{ "kind": "text", "text": "again" }]
            }
        }
    }))
    .unwrap();
    let response2: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            body2,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    // Second invocation must land in the same shared session — `contextId`
    // echoes the Everruns SessionId and shared_session mode reuses one
    // session for the channel.
    assert_eq!(
        response2["result"]["contextId"].as_str().unwrap(),
        session_id,
        "shared_session A2A invocations must reuse the same session",
    );
    let texts = list_user_message_texts(&server, session_id).await;
    assert!(texts.iter().any(|t| t == "from a2a: hello"));
    assert!(texts.iter().any(|t| t == "from a2a: again"));
}

#[tokio::test]
async fn a2a_message_send_emits_app_invocation_audit_log() {
    let server = TestServer::in_memory().await;
    let (app, api_key) = create_app_with_a2a(&server, "a2a-audit", "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "req-audit",
        "method": "message/send",
        "params": {
            "message": {
                "role": "user",
                "messageId": "msg-audit",
                "parts": [{ "kind": "text", "text": "audit me" }]
            }
        }
    }))
    .unwrap();

    let response: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();

    let session_id = response["result"]["contextId"].as_str().unwrap();
    let audit_log = wait_for_app_invocation_audit_log(&server, channel_id, session_id).await;

    assert_eq!(audit_log.domain, "agent");
    assert_eq!(audit_log.action, "agent.app_invocation.started");
    assert_eq!(audit_log.event_type, "agent.app_invocation.started");
    assert_eq!(audit_log.actor_id, None);
    assert_eq!(audit_log.metadata["source"], "app_a2a");
    assert_eq!(audit_log.metadata["app_id"], app_id);
    assert_eq!(audit_log.metadata["app_channel_id"], channel_id);
    assert_eq!(audit_log.metadata["app_channel_type"], "a2a");
    assert_eq!(audit_log.metadata["session_id"], session_id);
    assert_eq!(audit_log.metadata["created_session"], true);
}

#[tokio::test]
async fn a2a_rejects_missing_or_invalid_api_key() {
    let server = TestServer::in_memory().await;
    let (app, api_key) = create_app_with_a2a(&server, "a2a-auth", "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "message/send",
        "params": {
            "message": { "role": "user", "parts": [{ "kind": "text", "text": "hi" }] }
        }
    }))
    .unwrap();

    // No auth header.
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![("content-type", "application/json")],
            body.clone(),
        )
        .await
        .assert_status(StatusCode::UNAUTHORIZED);

    // Wrong key.
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", "Bearer evra2a_wrong"),
            ],
            body.clone(),
        )
        .await
        .assert_status(StatusCode::UNAUTHORIZED);

    // Right key still works.
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::OK);
}

#[tokio::test]
async fn a2a_rejects_unpublished_or_disabled() {
    let server = TestServer::in_memory().await;
    let (app, api_key) = create_app_with_a2a(&server, "a2a-pub", "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();

    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "message/send",
        "params": {
            "message": { "role": "user", "parts": [{ "kind": "text", "text": "hi" }] }
        }
    }))
    .unwrap();

    // Unpublished: generic 404 (EVE-632 / TM-TENANT-002). An unauthenticated
    // caller must not be able to distinguish "unpublished" from "unknown app".
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            body.clone(),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);

    publish_app(&server, app_id).await;

    server
        .patch(
            &format!("/v1/apps/{app_id}/channels/{channel_id}"),
            json!({ "enabled": false }),
        )
        .await
        .assert_status(StatusCode::OK);

    // Disabled channel: also a generic 404, not a 403 that confirms the app
    // exists and is published (EVE-632 / TM-AUTHZ-006).
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a2a_rejects_unsupported_methods_and_empty_text() {
    let server = TestServer::in_memory().await;
    let (app, api_key) = create_app_with_a2a(&server, "a2a-method", "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    // tasks/resubscribe is not supported (sentinel for unhandled method).
    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "x",
        "method": "tasks/resubscribe",
        "params": { "id": "task-1" }
    }))
    .unwrap();
    let response: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(response["error"]["code"], -32601);

    // Empty parts.
    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "y",
        "method": "message/send",
        "params": {
            "message": { "role": "user", "parts": [] }
        }
    }))
    .unwrap();
    let response: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(response["error"]["code"], -32602);

    // Present-but-malformed discriminators are invalid rather than treated as
    // legacy discriminator-free text parts.
    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "z",
        "method": "message/send",
        "params": {
            "message": {
                "role": "user",
                "parts": [{ "kind": 123, "text": "malformed discriminator" }]
            }
        }
    }))
    .unwrap();
    let response: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(response["error"]["code"], -32602);
}

/// `message/stream` is rejected for shared-session channels because stream
/// events cannot be safely correlated across concurrent callers.
#[tokio::test]
async fn a2a_message_stream_rejects_shared_session_channels() {
    let server = TestServer::in_memory().await;
    let (app, api_key) = create_app_with_a2a(&server, "a2a-stream", "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "stream-empty",
        "method": "message/stream",
        "params": {
            "message": { "role": "user", "parts": [] }
        }
    }))
    .unwrap();
    let response: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(response["error"]["code"], -32600);
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("session_mode=session_per_invocation"),
        "expected stream-mode rejection for shared sessions: {response:?}",
    );
}

/// `message/stream` is dispatched on `session_per_invocation` channels: the
/// session-mode gate must not fire, so an empty `parts` body should reach
/// `parse_message_params` and surface as `-32602 Invalid params`.
#[tokio::test]
async fn a2a_message_stream_dispatches_on_session_per_invocation_channels() {
    let server = TestServer::in_memory().await;
    let (app, api_key) = create_app_with_a2a_mode(
        &server,
        "a2a-stream-spi",
        "{{a2a.text}}",
        "session_per_invocation",
    )
    .await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "stream-empty",
        "method": "message/stream",
        "params": {
            "message": {
                "role": "user",
                "parts": []
            }
        }
    }))
    .unwrap();

    let response: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{}/a2a/{}", app_id, channel_id),
            vec![
                ("authorization", &format!("Bearer {api_key}")),
                ("content-type", "application/json"),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(
        response["error"]["code"], -32602,
        "expected parse_message_params to surface -32602, not the session-mode gate: {response:?}"
    );
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("non-empty text part"),
        "unexpected error message: {response:?}"
    );
}

/// `tasks/get` returns the task derived from the underlying session lifecycle.
/// Right after `message/send` the task is non-terminal; an unknown task id
/// returns the documented `-32001 Task not found` JSON-RPC error envelope.
#[tokio::test]
async fn a2a_tasks_get_returns_non_terminal_for_freshly_dispatched_task() {
    let server = TestServer::in_memory().await;
    let (app, api_key) = create_app_with_a2a(&server, "a2a-tasks-get", "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    let send_body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "send-1",
        "method": "message/send",
        "params": {
            "message": {
                "role": "user",
                "parts": [{ "kind": "text", "text": "hello" }]
            }
        }
    }))
    .unwrap();
    let send_response: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            send_body,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    let task_id = send_response["result"]["id"].as_str().unwrap().to_string();

    // tasks/get on the freshly dispatched task — turn lifecycle hasn't moved
    // yet in the in-memory test harness, so state is `submitted` (or
    // `working` if the runtime has emitted turn.started by now). Either is
    // a valid non-terminal state.
    let get_body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "get-1",
        "method": "tasks/get",
        "params": { "id": task_id }
    }))
    .unwrap();
    let get_response: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            get_body,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(get_response["result"]["id"], task_id);
    assert_eq!(get_response["result"]["contextId"], task_id);
    let state = get_response["result"]["status"]["state"]
        .as_str()
        .unwrap_or("");
    assert!(
        state == "submitted" || state == "working",
        "expected non-terminal state, got {state:?}",
    );
    assert_eq!(get_response["result"]["kind"], "task");

    // Unknown task id (well-formed but not associated with any session)
    // surfaces -32001 rather than leaking session existence.
    let unknown_body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "get-unknown",
        "method": "tasks/get",
        "params": { "id": "session_01999999999979998888aaaaaaaaaaaa" }
    }))
    .unwrap();
    let unknown_response: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            unknown_body,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(unknown_response["error"]["code"], -32001);

    // Malformed task id surfaces -32602 Invalid params.
    let malformed_body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "get-bad",
        "method": "tasks/get",
        "params": { "id": "not-a-uuid" }
    }))
    .unwrap();
    let malformed_response: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            malformed_body,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(malformed_response["error"]["code"], -32602);
}

/// Send a JSON-RPC request to an A2A channel and return the parsed envelope.
async fn a2a_rpc(
    server: &TestServer,
    app_id: &str,
    channel_id: &str,
    api_key: &str,
    method: &str,
    params: Value,
) -> Value {
    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "rpc-1",
        "method": method,
        "params": params,
    }))
    .unwrap();
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::OK)
        .json()
}

/// Seed a deterministic structured result (EVE-678) on `session_id`: create a
/// `result_schema`-bound task owned by the session, point it at a result file,
/// and write that file into the session's workspace VFS. Returns the JSON we
/// stored so callers can assert round-trip fidelity.
async fn seed_structured_result(server: &TestServer, session_id: &str, result: Value) {
    use everruns_core::session_task::{
        CreateSessionTask, SessionTaskState, SessionTaskUpdate, new_session_task, task_result_path,
    };
    use everruns_server::storage::models::CreateSessionFileRow;

    let sid = session_id.parse::<SessionId>().expect("valid session id");
    let session = server
        .db
        .get_session(DEFAULT_ORG_ID, sid)
        .await
        .expect("get_session")
        .expect("session exists");

    // Create the owning task in a non-terminal state, then set `result_path`
    // via an update — mirroring how `report_result` records the pointer.
    let task = new_session_task(
        CreateSessionTask {
            session_id: sid,
            id: None,
            kind: "subagent".to_string(),
            display_name: "structured result".to_string(),
            spec: json!({ "result_schema": { "type": "object" } }),
            state: SessionTaskState::Running,
            links: Default::default(),
            wake_policy: Default::default(),
        },
        chrono::Utc::now(),
    );
    server
        .db
        .create_session_task(&task)
        .await
        .expect("create task");

    let path = task_result_path(&task.id);
    server
        .db
        .update_session_task(
            sid,
            &task.id,
            SessionTaskUpdate {
                state: Some(SessionTaskState::Succeeded),
                result_path: Some(path.clone()),
                ..Default::default()
            },
        )
        .await
        .expect("set result_path");

    // The VFS is keyed by the owning session's workspace id (that's the key
    // `report_result` writes with, and the key the reader reads back with).
    server
        .db
        .create_session_file(CreateSessionFileRow {
            session_id: SessionId::from_uuid(session.workspace_id),
            path,
            content: Some(serde_json::to_vec(&result).unwrap()),
            is_directory: false,
            is_readonly: false,
        })
        .await
        .expect("write result file");
}

async fn seed_non_schema_result_path(server: &TestServer, session_id: &str, result: Value) {
    use everruns_core::session_task::{
        CreateSessionTask, SessionTaskState, SessionTaskUpdate, new_session_task,
    };
    use everruns_server::storage::models::CreateSessionFileRow;

    let sid = session_id.parse::<SessionId>().expect("valid session id");
    let session = server
        .db
        .get_session(DEFAULT_ORG_ID, sid)
        .await
        .expect("get_session")
        .expect("session exists");

    let task = new_session_task(
        CreateSessionTask {
            session_id: sid,
            id: None,
            kind: "background_tool".to_string(),
            display_name: "background result".to_string(),
            spec: json!({ "tool": "internal_scan" }),
            state: SessionTaskState::Running,
            links: Default::default(),
            wake_policy: Default::default(),
        },
        chrono::Utc::now(),
    );
    server
        .db
        .create_session_task(&task)
        .await
        .expect("create non-schema task");

    let path = format!("/.background/{}/result.json", task.id);
    server
        .db
        .update_session_task(
            sid,
            &task.id,
            SessionTaskUpdate {
                state: Some(SessionTaskState::Succeeded),
                result_path: Some(path.clone()),
                ..Default::default()
            },
        )
        .await
        .expect("set non-schema result_path");

    server
        .db
        .create_session_file(CreateSessionFileRow {
            session_id: SessionId::from_uuid(session.workspace_id),
            path,
            content: Some(serde_json::to_vec(&result).unwrap()),
            is_directory: false,
            is_readonly: false,
        })
        .await
        .expect("write non-schema result file");
}

/// A task that reported a schema-bound `result.json` (EVE-678) exposes it as an
/// A2A `tasks/get` artifact `DataPart`, not just last-message / status text.
#[tokio::test]
async fn a2a_tasks_get_surfaces_structured_result_artifact() {
    let server = TestServer::in_memory().await;
    let (app, api_key) = create_app_with_a2a(&server, "a2a-result-artifact", "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    let send = a2a_rpc(
        &server,
        app_id,
        channel_id,
        &api_key,
        "message/send",
        json!({ "message": { "role": "user", "parts": [{ "kind": "text", "text": "triage" }] } }),
    )
    .await;
    let task_id = send["result"]["id"].as_str().unwrap().to_string();

    // No structured result yet → no artifacts on the task.
    let before = a2a_rpc(
        &server,
        app_id,
        channel_id,
        &api_key,
        "tasks/get",
        json!({ "id": task_id }),
    )
    .await;
    assert!(
        before["result"].get("artifacts").is_none(),
        "no artifacts before a result is reported, got {before}"
    );

    let expected = json!({ "verdict": "spam", "confidence": 0.97 });
    seed_structured_result(&server, &task_id, expected.clone()).await;

    let after = a2a_rpc(
        &server,
        app_id,
        channel_id,
        &api_key,
        "tasks/get",
        json!({ "id": task_id }),
    )
    .await;
    let artifacts = after["result"]["artifacts"]
        .as_array()
        .expect("artifacts present after reporting a result");
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0]["name"], "result");
    let parts = artifacts[0]["parts"].as_array().expect("artifact parts");
    assert_eq!(parts[0]["kind"], "data");
    assert_eq!(parts[0]["data"], expected);
}

#[tokio::test]
async fn a2a_tasks_get_ignores_non_schema_result_path_artifact() {
    let server = TestServer::in_memory().await;
    let (app, api_key) =
        create_app_with_a2a(&server, "a2a-non-schema-result", "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    let send = a2a_rpc(
        &server,
        app_id,
        channel_id,
        &api_key,
        "message/send",
        json!({ "message": { "role": "user", "parts": [{ "kind": "text", "text": "triage" }] } }),
    )
    .await;
    let task_id = send["result"]["id"].as_str().unwrap().to_string();

    seed_non_schema_result_path(&server, &task_id, json!({ "internal": "tool-output" })).await;

    let resp = a2a_rpc(
        &server,
        app_id,
        channel_id,
        &api_key,
        "tasks/get",
        json!({ "id": task_id }),
    )
    .await;
    assert!(
        resp["result"].get("artifacts").is_none(),
        "non-schema result_path leaked as artifact: {resp}"
    );
}

/// Tenant isolation: a second channel's API key must not receive another
/// channel's structured-result artifact — the cross-channel lookup collapses to
/// `-32001 Task not found` with no artifact leak.
#[tokio::test]
async fn a2a_tasks_get_structured_result_not_leaked_cross_channel() {
    let server = TestServer::in_memory().await;
    let (app_a, key_a) = create_app_with_a2a(&server, "a2a-result-owner", "{{a2a.text}}").await;
    let app_a_id = app_a["id"].as_str().unwrap();
    let chan_a = app_a["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_a_id).await;

    let (app_b, key_b) = create_app_with_a2a(&server, "a2a-result-snoop", "{{a2a.text}}").await;
    let app_b_id = app_b["id"].as_str().unwrap();
    let chan_b = app_b["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_b_id).await;

    let send = a2a_rpc(
        &server,
        app_a_id,
        chan_a,
        &key_a,
        "message/send",
        json!({ "message": { "role": "user", "parts": [{ "kind": "text", "text": "secret" }] } }),
    )
    .await;
    let task_id = send["result"]["id"].as_str().unwrap().to_string();
    seed_structured_result(&server, &task_id, json!({ "secret": "value" })).await;

    // Channel B (a different app/channel in the same org) cannot read the task
    // or its artifact.
    let snoop = a2a_rpc(
        &server,
        app_b_id,
        chan_b,
        &key_b,
        "tasks/get",
        json!({ "id": task_id }),
    )
    .await;
    assert_eq!(snoop["error"]["code"], -32001);
    assert!(snoop.get("result").is_none() || snoop["result"].is_null());

    // The owning channel still sees the artifact.
    let owner = a2a_rpc(
        &server,
        app_a_id,
        chan_a,
        &key_a,
        "tasks/get",
        json!({ "id": task_id }),
    )
    .await;
    assert_eq!(
        owner["result"]["artifacts"][0]["parts"][0]["data"]["secret"],
        "value"
    );
}

/// `tasks/cancel` returns the task with state=canceled and is idempotent — a
/// second cancel returns the already-canceled task without a new state
/// transition.
#[tokio::test]
async fn a2a_tasks_cancel_terminates_task_idempotently() {
    let server = TestServer::in_memory().await;
    let (app, api_key) = create_app_with_a2a(&server, "a2a-tasks-cancel", "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    let send_body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "send-1",
        "method": "message/send",
        "params": {
            "message": {
                "role": "user",
                "parts": [{ "kind": "text", "text": "hi" }]
            }
        }
    }))
    .unwrap();
    let send_response: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            send_body,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    let task_id = send_response["result"]["id"].as_str().unwrap().to_string();

    let cancel_body = |id: &str| {
        serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": "cancel",
            "method": "tasks/cancel",
            "params": { "id": id }
        }))
        .unwrap()
    };

    let first: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            cancel_body(&task_id),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(first["result"]["status"]["state"], "canceled");
    assert_eq!(first["result"]["id"], task_id);

    // Idempotence: a second cancel sees a terminal state and returns the
    // same task shape without re-cancelling. tasks/get also reports
    // canceled.
    let second: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            cancel_body(&task_id),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(second["result"]["status"]["state"], "canceled");

    let get_body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "get-after-cancel",
        "method": "tasks/get",
        "params": { "id": task_id }
    }))
    .unwrap();
    let get_response: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            get_body,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(get_response["result"]["status"]["state"], "canceled");
}

/// Channel binding (TM-A2A-012): a task created on one A2A channel must not
/// be readable or cancellable by an API key authenticated against a
/// different channel — even when both channels live in the same org. The
/// out-of-channel lookup must surface `-32001 Task not found` rather than
/// leaking session existence.
#[tokio::test]
async fn a2a_tasks_get_rejects_cross_channel_lookup() {
    let server = TestServer::in_memory().await;
    let (app_a, key_a) = create_app_with_a2a(&server, "a2a-cross-a", "{{a2a.text}}").await;
    let (app_b, key_b) = create_app_with_a2a(&server, "a2a-cross-b", "{{a2a.text}}").await;
    let a_id = app_a["id"].as_str().unwrap();
    let a_channel = app_a["channels"][0]["id"].as_str().unwrap();
    let b_id = app_b["id"].as_str().unwrap();
    let b_channel = app_b["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, a_id).await;
    publish_app(&server, b_id).await;

    // Submit a task on channel A.
    let send_body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "send-a",
        "method": "message/send",
        "params": {
            "message": { "role": "user", "parts": [{ "kind": "text", "text": "hi" }] }
        }
    }))
    .unwrap();
    let send_response: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{a_id}/a2a/{a_channel}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {key_a}")),
            ],
            send_body,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    let task_id = send_response["result"]["id"].as_str().unwrap().to_string();

    // Channel A's own key reads the task fine.
    let get_body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "get-a",
        "method": "tasks/get",
        "params": { "id": task_id }
    }))
    .unwrap();
    let own: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{a_id}/a2a/{a_channel}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {key_a}")),
            ],
            get_body.clone(),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(own["result"]["id"], task_id);

    // Channel B's key on its own endpoint must not see channel A's task.
    let cross: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{b_id}/a2a/{b_channel}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {key_b}")),
            ],
            get_body,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(cross["error"]["code"], -32001);

    // tasks/cancel must also refuse the cross-channel attempt.
    let cancel_body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "cancel-cross",
        "method": "tasks/cancel",
        "params": { "id": task_id }
    }))
    .unwrap();
    let cross_cancel: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{b_id}/a2a/{b_channel}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {key_b}")),
            ],
            cancel_body,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(cross_cancel["error"]["code"], -32001);
}

#[tokio::test]
async fn a2a_agent_card_published_only_when_live() {
    let server = TestServer::in_memory().await;
    let (app, _api_key) = create_app_with_a2a(&server, "a2a-card", "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    let card_path = format!("/v1/apps/{app_id}/a2a/{channel_id}/.well-known/agent-card.json");

    // Draft -> 404.
    server
        .request_raw(Method::GET, &card_path, vec![], vec![])
        .await
        .assert_status(StatusCode::NOT_FOUND);

    publish_app(&server, app_id).await;

    let card: Value = server
        .request_raw(
            Method::GET,
            &card_path,
            vec![("host", "example.test"), ("x-forwarded-proto", "http")],
            vec![],
        )
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(card["name"], "Inbox triage");
    assert!(card.get("protocolVersion").is_none());
    assert!(card.get("preferredTransport").is_none());
    assert!(card.get("url").is_none());
    let interfaces = card["supportedInterfaces"].as_array().unwrap();
    assert_eq!(interfaces.len(), 1);
    assert_eq!(interfaces[0]["protocolBinding"], "JSONRPC");
    assert_eq!(interfaces[0]["protocolVersion"], "1.0");
    assert!(
        interfaces[0]["url"]
            .as_str()
            .unwrap()
            .ends_with(&format!("/v1/apps/{app_id}/a2a/{channel_id}"))
    );
    let parsed_card: a2a::AgentCard = serde_json::from_value(card.clone()).unwrap();
    assert_eq!(parsed_card.supported_interfaces.len(), 1);
    assert_eq!(
        parsed_card.supported_interfaces[0].protocol_binding,
        "JSONRPC"
    );
    assert_eq!(parsed_card.supported_interfaces[0].protocol_version, "1.0");
    a2a_client::A2AClientFactory::builder()
        .build()
        .create_from_card(&parsed_card)
        .await
        .expect("served AgentCard should negotiate with the linked A2AClientFactory");
    // Streaming is only advertised for session_per_invocation channels.
    // The helper builds a shared_session channel by default, so the card
    // must report streaming=false to stay consistent with the runtime gate.
    assert_eq!(card["capabilities"]["streaming"], false);
    assert_eq!(card["capabilities"]["pushNotifications"], false);
    // Card never echoes secrets.
    let serialized = serde_json::to_string(&card).unwrap();
    assert!(!serialized.contains("api_key"));
    assert!(!serialized.contains("api_key_hash"));

    // Disable channel -> 404.
    server
        .patch(
            &format!("/v1/apps/{app_id}/channels/{channel_id}"),
            json!({ "enabled": false }),
        )
        .await
        .assert_status(StatusCode::OK);

    server
        .request_raw(Method::GET, &card_path, vec![], vec![])
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn outbound_a2a_delegation_reaches_local_app_with_discovery_card() {
    let (_server, endpoint, api_key, _app_id, _channel_id) =
        create_published_served_a2a_app().await;
    let discovery_base_url = spawn_agent_card_server(a2a_agent_card(&endpoint)).await;
    let config = outbound_delegation_config(&endpoint, &api_key, Some(&discovery_base_url));

    let (storage, spawn_result) = spawn_background_against_local_a2a(config).await;

    assert_ne!(
        spawn_result["status"], "failed",
        "spawn_agent failed: {spawn_result:?}",
    );
    let run_id = spawn_result["agent_run_id"].as_str().unwrap();
    let remote_task_id = spawn_result["remote_task_id"].as_str().unwrap();
    assert_eq!(
        spawn_result["remote_context_id"].as_str().unwrap(),
        remote_task_id
    );

    let metadata = wait_for_remote_task_snapshot(&storage, run_id).await;
    assert_eq!(metadata["remote_task_id"], remote_task_id);
    assert_eq!(metadata["last_remote_task_snapshot"]["id"], remote_task_id);
    assert!(
        matches!(
            metadata["last_remote_task_snapshot"]["state"].as_str(),
            Some(
                "submitted"
                    | "working"
                    | "completed"
                    | "TASK_STATE_SUBMITTED"
                    | "TASK_STATE_WORKING"
                    | "TASK_STATE_COMPLETED"
            )
        ),
        "expected non-error remote task state, got {metadata:?}",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn outbound_a2a_delegation_reaches_local_app_with_inline_card() {
    let (_server, endpoint, api_key, _app_id, _channel_id) =
        create_published_served_a2a_app().await;
    let config = outbound_delegation_config(&endpoint, &api_key, None);

    let (storage, spawn_result) = spawn_background_against_local_a2a(config).await;

    assert_ne!(
        spawn_result["status"], "failed",
        "spawn_agent failed: {spawn_result:?}",
    );
    let run_id = spawn_result["agent_run_id"].as_str().unwrap();
    let remote_task_id = spawn_result["remote_task_id"].as_str().unwrap();
    assert_eq!(
        spawn_result["remote_context_id"].as_str().unwrap(),
        remote_task_id
    );

    let metadata = wait_for_remote_task_snapshot(&storage, run_id).await;
    assert_eq!(metadata["remote_task_id"], remote_task_id);
    assert_eq!(metadata["last_remote_task_snapshot"]["id"], remote_task_id);
    assert!(
        matches!(
            metadata["last_remote_task_snapshot"]["state"].as_str(),
            Some(
                "submitted"
                    | "working"
                    | "completed"
                    | "TASK_STATE_SUBMITTED"
                    | "TASK_STATE_WORKING"
                    | "TASK_STATE_COMPLETED"
            )
        ),
        "expected non-error remote task state, got {metadata:?}",
    );
}

#[tokio::test]
async fn a2a_patch_preserves_api_key_when_omitted() {
    // PATCH on an A2A channel must preserve the server-managed api_key_hash
    // and api_key_prefix even when the client only sends the user-editable
    // fields (message, session_mode, agent card metadata). Otherwise an edit
    // would silently break authentication for previously issued keys.
    let server = TestServer::in_memory().await;
    let (app, api_key) = create_app_with_a2a(&server, "a2a-preserve", "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    server
        .patch(
            &format!("/v1/apps/{app_id}/channels/{channel_id}"),
            json!({
                "channel_config": {
                    "session_mode": "session_per_invocation",
                    "message": "edited {{a2a.text}}",
                    "agent_card_name": "Edited",
                }
            }),
        )
        .await
        .assert_status(StatusCode::OK);

    // The originally issued API key still works after the edit.
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            serde_json::to_vec(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "message/send",
                "params": {
                    "message": { "role": "user", "parts": [{ "kind": "text", "text": "post-edit" }] }
                }
            }))
            .unwrap(),
        )
        .await
        .assert_status(StatusCode::OK);
}

#[tokio::test]
async fn a2a_regenerate_key_invalidates_previous_key() {
    let server = TestServer::in_memory().await;
    let (app, original_key) = create_app_with_a2a(&server, "a2a-rotate", "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    // Original key works.
    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "message/send",
        "params": {
            "message": { "role": "user", "parts": [{ "kind": "text", "text": "hi" }] }
        }
    }))
    .unwrap();
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {original_key}")),
            ],
            body.clone(),
        )
        .await
        .assert_status(StatusCode::OK);

    // Rotate.
    let rotated: Value = server
        .post(
            &format!("/v1/apps/{app_id}/a2a-channels/{channel_id}/regenerate-key"),
            json!({}),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    let new_key = rotated["api_key"].as_str().unwrap().to_string();
    assert_ne!(new_key, original_key);

    // Original key fails.
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {original_key}")),
            ],
            body.clone(),
        )
        .await
        .assert_status(StatusCode::UNAUTHORIZED);

    // New key works.
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {new_key}")),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a2a_per_channel_rate_limit_returns_429() {
    // TM-A2A-013: configurable per-app, per-IP cap on the A2A ingress endpoint
    // protects an app's quota and LLM budget from a runaway counterparty agent
    // even when the global API limit would still allow more traffic.
    let server = TestServer::in_memory().await;
    let (app, api_key) =
        create_app_with_a2a(&server, "a2a-rate-limit", "from a2a: {{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    server
        .patch(
            &format!("/v1/apps/{app_id}/channels/{channel_id}"),
            json!({
                "channel_config": {
                    "session_mode": "shared_session",
                    "message": "from a2a: {{a2a.text}}",
                    "rate_limit_per_minute": 2
                }
            }),
        )
        .await
        .assert_status(StatusCode::OK);

    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "message/send",
        "params": {
            "message": { "role": "user", "parts": [{ "kind": "text", "text": "hi" }] }
        }
    }))
    .unwrap();

    for _ in 0..2 {
        server
            .request_raw(
                Method::POST,
                &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
                vec![
                    ("content-type", "application/json"),
                    ("authorization", &format!("Bearer {api_key}")),
                ],
                body.clone(),
            )
            .await
            .assert_status(StatusCode::OK);
    }

    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a2a_rate_limit_zero_disables_per_channel_cap() {
    let server = TestServer::in_memory().await;
    let (app, api_key) =
        create_app_with_a2a(&server, "a2a-rate-zero", "from a2a: {{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    server
        .patch(
            &format!("/v1/apps/{app_id}/channels/{channel_id}"),
            json!({
                "channel_config": {
                    "session_mode": "shared_session",
                    "message": "from a2a: {{a2a.text}}",
                    "rate_limit_per_minute": 0
                }
            }),
        )
        .await
        .assert_status(StatusCode::OK);

    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "message/send",
        "params": {
            "message": { "role": "user", "parts": [{ "kind": "text", "text": "hi" }] }
        }
    }))
    .unwrap();

    // With rate_limit_per_minute=0 the per-channel cap is disabled; five back-
    // to-back requests must each succeed (or at least never hit 429).
    for _ in 0..5 {
        let response = server
            .request_raw(
                Method::POST,
                &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
                vec![
                    ("content-type", "application/json"),
                    ("authorization", &format!("Bearer {api_key}")),
                ],
                body.clone(),
            )
            .await;
        assert_ne!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a2a_rate_limit_is_per_channel_not_per_app() {
    // TM-A2A-013 regression: an app with two A2A channels that have
    // different `rate_limit_per_minute` settings must keep independent
    // buckets. If the limiter were keyed on `app_id` only, alternating
    // requests across the two channels would replace the cached limiter
    // (replace-on-limit-change) and effectively bypass the stricter cap.
    let server = TestServer::in_memory().await;
    let (app, api_key_a) =
        create_app_with_a2a(&server, "a2a-multi-channel", "from a2a: {{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id_a = app["channels"][0]["id"].as_str().unwrap().to_string();

    // Create a second A2A channel on the same app, with a much higher cap.
    let second_channel_response: Value = server
        .post(
            &format!("/v1/apps/{app_id}/a2a-channels"),
            json!({
                "session_mode": "shared_session",
                "message": "from a2a (b): {{a2a.text}}",
                "agent_card_name": "Inbox triage (b)",
                "agent_card_description": "Triages inbound A2A traffic (b)",
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let api_key_b = second_channel_response["api_key"]
        .as_str()
        .unwrap()
        .to_string();
    let app_after: Value = server
        .get(&format!("/v1/apps/{app_id}"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let channels = app_after["channels"].as_array().unwrap();
    let channel_id_b = channels
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_string())
        .find(|id| id != &channel_id_a)
        .expect("second a2a channel id");

    publish_app(&server, app_id).await;

    // Channel A: rate_limit_per_minute = 2 (strict).
    server
        .patch(
            &format!("/v1/apps/{app_id}/channels/{channel_id_a}"),
            json!({
                "channel_config": {
                    "session_mode": "shared_session",
                    "message": "from a2a: {{a2a.text}}",
                    "rate_limit_per_minute": 2
                }
            }),
        )
        .await
        .assert_status(StatusCode::OK);

    // Channel B: rate_limit_per_minute = 100 (permissive — different limit
    // value, which is the trigger for the replace-on-limit-change strategy
    // that previously flushed the per-IP buckets when both channels shared
    // the same `app_id` cache key).
    server
        .patch(
            &format!("/v1/apps/{app_id}/channels/{channel_id_b}"),
            json!({
                "channel_config": {
                    "session_mode": "shared_session",
                    "message": "from a2a (b): {{a2a.text}}",
                    "rate_limit_per_minute": 100
                }
            }),
        )
        .await
        .assert_status(StatusCode::OK);

    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "message/send",
        "params": {
            "message": { "role": "user", "parts": [{ "kind": "text", "text": "hi" }] }
        }
    }))
    .unwrap();

    // Burn channel A's quota.
    for _ in 0..2 {
        server
            .request_raw(
                Method::POST,
                &format!("/v1/apps/{app_id}/a2a/{channel_id_a}"),
                vec![
                    ("content-type", "application/json"),
                    ("authorization", &format!("Bearer {api_key_a}")),
                ],
                body.clone(),
            )
            .await
            .assert_status(StatusCode::OK);
    }

    // A request to channel B with its own (higher) limit must succeed and
    // must NOT reset channel A's bucket.
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id_b}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key_b}")),
            ],
            body.clone(),
        )
        .await
        .assert_status(StatusCode::OK);

    // The next request on channel A must still be rate-limited — its bucket
    // was not flushed by the channel B request.
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id_a}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key_a}")),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn a2a_rate_limit_rejects_absurd_values() {
    let server = TestServer::in_memory().await;
    let (app, _api_key) = create_app_with_a2a(&server, "a2a-rate-bad", "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();

    // Mirror the AG-UI cap so a typo can't silently disable the per-channel
    // limit by overflowing reasonable expectations.
    server
        .patch(
            &format!("/v1/apps/{app_id}/channels/{channel_id}"),
            json!({
                "channel_config": {
                    "session_mode": "shared_session",
                    "message": "{{a2a.text}}",
                    "rate_limit_per_minute": 9_999_999_u32
                }
            }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}
// ---------------------------------------------------------------------------
// A2A request replay protection (TM-A2A-010)
//
// `signing_secret` is opt-in. When set, every request must carry a
// timestamp + HMAC-SHA256 signature over `v0:{ts}:{body}`. The server
// verifies the headers, enforces a 5-minute window, and dedups against
// the signature itself so an attacker who captures one request cannot
// replay it within the window.
// ---------------------------------------------------------------------------

const A2A_SIGNING_SECRET: &str = "shared-a2a-signing-secret-1234567890";

async fn enable_a2a_signing(server: &TestServer, app_id: &str, channel_id: &str, secret: &str) {
    server
        .patch(
            &format!("/v1/apps/{app_id}/channels/{channel_id}"),
            json!({
                "channel_config": {
                    "session_mode": "shared_session",
                    "message": "from a2a: {{a2a.text}}",
                    "signing_secret": secret
                }
            }),
        )
        .await
        .assert_status(StatusCode::OK);
}

#[tokio::test]
async fn a2a_signed_channel_accepts_valid_signature() {
    let server = TestServer::in_memory().await;
    let (app, api_key) = create_app_with_a2a(&server, "a2a-signed", "from a2a: {{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;
    enable_a2a_signing(&server, app_id, channel_id, A2A_SIGNING_SECRET).await;

    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "message/send",
        "params": {
            "message": { "role": "user", "parts": [{ "kind": "text", "text": "hi" }] }
        }
    }))
    .unwrap();
    let ts = a2a_now_secs();
    let sig = a2a_sign(
        A2A_SIGNING_SECRET,
        ts,
        &format!("{app_id}:{channel_id}"),
        &body,
    );

    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
                ("x-everruns-a2a-timestamp", &ts.to_string()),
                ("x-everruns-a2a-signature", &sig),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::OK);
}

#[tokio::test]
async fn a2a_signed_channel_rejects_missing_signature_headers() {
    let server = TestServer::in_memory().await;
    let (app, api_key) =
        create_app_with_a2a(&server, "a2a-signed-missing", "from a2a: {{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;
    enable_a2a_signing(&server, app_id, channel_id, A2A_SIGNING_SECRET).await;

    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "message/send",
        "params": {
            "message": { "role": "user", "parts": [{ "kind": "text", "text": "hi" }] }
        }
    }))
    .unwrap();

    // No signing headers at all — the signed channel must reject with 401
    // because authentication-equivalent freshness proof is missing.
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a2a_signed_channel_rejects_bad_signature() {
    let server = TestServer::in_memory().await;
    let (app, api_key) =
        create_app_with_a2a(&server, "a2a-signed-badsig", "from a2a: {{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;
    enable_a2a_signing(&server, app_id, channel_id, A2A_SIGNING_SECRET).await;

    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "message/send",
        "params": {
            "message": { "role": "user", "parts": [{ "kind": "text", "text": "hi" }] }
        }
    }))
    .unwrap();
    let ts = a2a_now_secs();
    let sig = a2a_sign(
        "not-the-secret",
        ts,
        &format!("{app_id}:{channel_id}"),
        &body,
    );

    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
                ("x-everruns-a2a-timestamp", &ts.to_string()),
                ("x-everruns-a2a-signature", &sig),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a2a_signed_channel_rejects_stale_timestamp() {
    let server = TestServer::in_memory().await;
    let (app, api_key) =
        create_app_with_a2a(&server, "a2a-signed-stale", "from a2a: {{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;
    enable_a2a_signing(&server, app_id, channel_id, A2A_SIGNING_SECRET).await;

    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "message/send",
        "params": {
            "message": { "role": "user", "parts": [{ "kind": "text", "text": "hi" }] }
        }
    }))
    .unwrap();
    // 6 minutes in the past — outside the 5-minute window. Signature is
    // valid for that timestamp; the window check rejects it anyway.
    let ts = a2a_now_secs() - 360;
    let sig = a2a_sign(
        A2A_SIGNING_SECRET,
        ts,
        &format!("{app_id}:{channel_id}"),
        &body,
    );

    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
                ("x-everruns-a2a-timestamp", &ts.to_string()),
                ("x-everruns-a2a-signature", &sig),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a2a_signed_channel_rejects_replay_within_window() {
    // TM-A2A-010 dedup: a captured request that survives the timestamp
    // window check is rejected because its (deterministic) signature is
    // already in the per-channel replay store.
    let server = TestServer::in_memory().await;
    let (app, api_key) =
        create_app_with_a2a(&server, "a2a-signed-replay", "from a2a: {{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;
    enable_a2a_signing(&server, app_id, channel_id, A2A_SIGNING_SECRET).await;

    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "message/send",
        "params": {
            "message": { "role": "user", "parts": [{ "kind": "text", "text": "hi" }] }
        }
    }))
    .unwrap();
    let ts = a2a_now_secs();
    let sig = a2a_sign(
        A2A_SIGNING_SECRET,
        ts,
        &format!("{app_id}:{channel_id}"),
        &body,
    );

    // First sighting succeeds.
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
                ("x-everruns-a2a-timestamp", &ts.to_string()),
                ("x-everruns-a2a-signature", &sig),
            ],
            body.clone(),
        )
        .await
        .assert_status(StatusCode::OK);

    // Replaying the exact same envelope + signature within the window is
    // rejected as a replay.
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
                ("x-everruns-a2a-timestamp", &ts.to_string()),
                ("x-everruns-a2a-signature", &sig),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a2a_signed_channel_rejects_cross_channel_replay() {
    // TM-A2A-010 cross-channel binding: when operators reuse the same
    // `signing_secret` across two A2A channels on the same app, a
    // signature computed for channel A must NOT be accepted on channel B.
    // The signed basestring includes `{app_id}:{channel_id}` so the
    // signature is bound to its target; verification on a different
    // channel must fail with 401.
    let server = TestServer::in_memory().await;
    let (app, api_key_a) =
        create_app_with_a2a(&server, "a2a-cross-channel", "from a2a: {{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id_a = app["channels"][0]["id"].as_str().unwrap().to_string();

    let second_channel_response: Value = server
        .post(
            &format!("/v1/apps/{app_id}/a2a-channels"),
            json!({
                "session_mode": "shared_session",
                "message": "from a2a (b): {{a2a.text}}",
                "agent_card_name": "Inbox triage (b)",
                "agent_card_description": "Triages inbound A2A traffic (b)",
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let api_key_b = second_channel_response["api_key"]
        .as_str()
        .unwrap()
        .to_string();
    let app_after: Value = server
        .get(&format!("/v1/apps/{app_id}"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let channel_id_b = app_after["channels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_string())
        .find(|id| id != &channel_id_a)
        .expect("second a2a channel id");

    publish_app(&server, app_id).await;

    // Both channels share the SAME signing secret — this is exactly the
    // misconfiguration the scope binding is intended to defend against.
    enable_a2a_signing(&server, app_id, &channel_id_a, A2A_SIGNING_SECRET).await;
    enable_a2a_signing(&server, app_id, &channel_id_b, A2A_SIGNING_SECRET).await;

    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "message/send",
        "params": {
            "message": { "role": "user", "parts": [{ "kind": "text", "text": "hi" }] }
        }
    }))
    .unwrap();
    let ts = a2a_now_secs();
    // Sign with channel A's scope.
    let sig_for_a = a2a_sign(
        A2A_SIGNING_SECRET,
        ts,
        &format!("{app_id}:{channel_id_a}"),
        &body,
    );

    // Sanity check: the signature is accepted on channel A (its target).
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id_a}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key_a}")),
                ("x-everruns-a2a-timestamp", &ts.to_string()),
                ("x-everruns-a2a-signature", &sig_for_a),
            ],
            body.clone(),
        )
        .await
        .assert_status(StatusCode::OK);

    // The same signed envelope replayed against channel B with channel
    // B's API key must be rejected, even though both channels share the
    // signing secret and the timestamp is still fresh. Without the
    // scope binding the per-channel replay store would not catch this.
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id_b}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key_b}")),
                ("x-everruns-a2a-timestamp", &ts.to_string()),
                ("x-everruns-a2a-signature", &sig_for_a),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a2a_unsigned_channel_keeps_api_key_only_behavior() {
    // Backward compatibility: channels without a `signing_secret` must
    // continue to accept plain bearer-only requests with no signing
    // headers. Existing deployments can opt in later without breaking
    // already-deployed clients.
    let server = TestServer::in_memory().await;
    let (app, api_key) =
        create_app_with_a2a(&server, "a2a-unsigned", "from a2a: {{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "message/send",
        "params": {
            "message": { "role": "user", "parts": [{ "kind": "text", "text": "hi" }] }
        }
    }))
    .unwrap();

    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::OK);
}

#[tokio::test]
async fn a2a_agent_card_advertises_signing_scheme_when_enabled() {
    let server = TestServer::in_memory().await;
    let (app, _api_key) =
        create_app_with_a2a(&server, "a2a-card-signed", "from a2a: {{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;
    enable_a2a_signing(&server, app_id, channel_id, A2A_SIGNING_SECRET).await;

    let card: Value = server
        .get(&format!(
            "/v1/apps/{app_id}/a2a/{channel_id}/.well-known/agent-card.json"
        ))
        .await
        .assert_status(StatusCode::OK)
        .json();

    let schemes = card["securitySchemes"].as_object().unwrap();
    assert!(
        schemes.contains_key("apiKey"),
        "apiKey scheme must always be advertised"
    );
    assert!(
        schemes.contains_key("everrunsHmacSignature"),
        "signing scheme must be advertised when signing_secret is configured"
    );
    assert_eq!(
        schemes["everrunsHmacSignature"]["apiKeySecurityScheme"]["description"],
        "HMAC-SHA256 over v0:{timestamp}:{channel_scope}:{body}; pair with X-Everruns-A2A-Timestamp"
    );

    // Card never echoes the secret.
    let serialized = serde_json::to_string(&card).unwrap();
    assert!(!serialized.contains(A2A_SIGNING_SECRET));
    assert!(!serialized.contains("signing_secret"));
}

#[tokio::test]
async fn a2a_channel_read_redacts_signing_secret_with_configured_flag() {
    // The plaintext signing secret must never be returned by the API. After
    // configuring it via PATCH, a subsequent GET surfaces only a
    // `signing_secret_configured: true` flag.
    let server = TestServer::in_memory().await;
    let (app, _api_key) =
        create_app_with_a2a(&server, "a2a-redact", "from a2a: {{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    enable_a2a_signing(&server, app_id, channel_id, A2A_SIGNING_SECRET).await;

    let app_after: Value = server
        .get(&format!("/v1/apps/{app_id}"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let channel = app_after["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"].as_str() == Some(channel_id))
        .unwrap();
    let cfg = channel["channel_config"].as_object().unwrap();
    assert!(
        !cfg.contains_key("signing_secret"),
        "plaintext signing_secret must be redacted on read"
    );
    assert_eq!(cfg["signing_secret_configured"], json!(true));
    let serialized = serde_json::to_string(&app_after).unwrap();
    assert!(!serialized.contains(A2A_SIGNING_SECRET));
}

#[tokio::test]
async fn a2a_patch_preserves_signing_secret_when_omitted() {
    // PATCHing a signed channel without resending `signing_secret` must
    // keep the channel's signing requirement intact — otherwise editing
    // an unrelated field would silently turn off replay protection.
    let server = TestServer::in_memory().await;
    let (app, api_key) =
        create_app_with_a2a(&server, "a2a-patch-keep", "from a2a: {{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;
    enable_a2a_signing(&server, app_id, channel_id, A2A_SIGNING_SECRET).await;

    // Edit an unrelated field (the agent card name) without sending the
    // signing secret. Channel must remain in signed mode.
    server
        .patch(
            &format!("/v1/apps/{app_id}/channels/{channel_id}"),
            json!({
                "channel_config": {
                    "session_mode": "shared_session",
                    "message": "from a2a: {{a2a.text}}",
                    "agent_card_name": "Renamed Agent"
                }
            }),
        )
        .await
        .assert_status(StatusCode::OK);

    // Unsigned request must still 401 because signing is preserved.
    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "message/send",
        "params": {
            "message": { "role": "user", "parts": [{ "kind": "text", "text": "hi" }] }
        }
    }))
    .unwrap();
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            body.clone(),
        )
        .await
        .assert_status(StatusCode::UNAUTHORIZED);

    // A correctly-signed request still works against the preserved secret.
    let ts = a2a_now_secs();
    let sig = a2a_sign(
        A2A_SIGNING_SECRET,
        ts,
        &format!("{app_id}:{channel_id}"),
        &body,
    );
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
                ("x-everruns-a2a-timestamp", &ts.to_string()),
                ("x-everruns-a2a-signature", &sig),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::OK);
}
