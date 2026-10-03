//! Integration tests for inbound MCP Events as an agent trigger (EVE-1121).
//!
//! `FakeEventsServer` is an in-process MCP server behind the harness's
//! `mcp_servers` egress slot: it answers `events/list`, `events/subscribe` and
//! `events/unsubscribe` and records each call. Deliveries are signed with the
//! secret the trigger handed it on subscribe and posted to the callback route,
//! as the server would. See `knowledge/integrations/mcp-events.md`.
//!
//! Run with: cargo test -p everruns-server --test domain mcp_event_triggers_test::

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::records::SessionSource;
use axum::http::{Method, StatusCode};
use everruns_contracts::typed_id::{SessionId, TriggerId};
use everruns_core::DEFAULT_ORG_ID;
use everruns_server::services::standard_webhooks;
use serde_json::{Value, json};

use crate::test_harness::{TestResponse, TestServer};

const MCP_URL: &str = "https://8.8.8.8/mcp";
const EVENT: &str = "issue.created";

/// One JSON-RPC call the fake server received.
#[derive(Clone, Debug)]
struct Call {
    method: String,
    params: Value,
    authorization: Option<String>,
}

#[derive(Default)]
struct FakeEventsServer {
    calls: parking_lot::Mutex<Vec<Call>>,
    next_id: AtomicUsize,
    /// `refreshBefore` to answer subscribe with; one hour out when unset.
    refresh_before: parking_lot::Mutex<Option<String>>,
    reject_subscribe: AtomicBool,
}

impl FakeEventsServer {
    fn install(server: &TestServer) -> Arc<Self> {
        let fake = Arc::new(Self::default());
        *server.mcp_servers.0.write() = Some(fake.clone());
        fake
    }

    fn calls(&self, method: &str) -> Vec<Call> {
        self.calls
            .lock()
            .iter()
            .filter(|call| call.method == method)
            .cloned()
            .collect()
    }

    /// The last subscribe: what a real server would deliver with.
    fn subscription(&self) -> Subscription {
        let call = self.calls("events/subscribe").pop().expect("subscribed");
        let delivery = &call.params["delivery"];
        let url = delivery["url"].as_str().unwrap();
        Subscription {
            path: url[url.find("/v1/e/").expect("callback path")..].to_string(),
            secret: delivery["secret"].as_str().unwrap().to_string(),
            id: format!("sub_{}", self.next_id.load(Ordering::SeqCst)),
        }
    }

    fn result(&self, method: &str, params: &Value) -> Result<Value, (i64, &'static str)> {
        match method {
            "events/list" => Ok(json!({
                "events": [{
                    "name": EVENT,
                    "inputSchema": { "type": "object", "properties": { "team": { "type": "string" } } },
                }]
            })),
            "events/subscribe" => {
                if self.reject_subscribe.load(Ordering::SeqCst) {
                    return Err((-32602, "unknown team"));
                }
                if params["name"] != EVENT {
                    return Err((-32602, "unknown event"));
                }
                let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
                let refresh_before = self.refresh_before.lock().clone().unwrap_or_else(|| {
                    (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339()
                });
                Ok(json!({
                    "id": format!("sub_{id}"),
                    "refreshBefore": refresh_before,
                    "cursor": format!("cursor_{id}"),
                    "truncated": false,
                }))
            }
            "events/unsubscribe" => Ok(json!({})),
            _ => Err((-32601, "method not found")),
        }
    }
}

#[async_trait::async_trait]
impl everruns_core::EgressService for FakeEventsServer {
    async fn send(
        &self,
        request: everruns_core::EgressRequest,
    ) -> everruns_core::EgressResult<everruns_core::EgressResponse> {
        assert_eq!(request.url, MCP_URL);
        let body: Value = serde_json::from_slice(&request.body).unwrap_or_default();
        let method = body["method"].as_str().unwrap_or_default().to_string();
        let params = body["params"].clone();
        self.calls.lock().push(Call {
            method: method.clone(),
            params: params.clone(),
            authorization: request
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                .map(|(_, value)| value.clone()),
        });
        let reply = match self.result(&method, &params) {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": body["id"], "result": result }),
            Err((code, message)) => json!({
                "jsonrpc": "2.0", "id": body["id"], "error": { "code": code, "message": message }
            }),
        };
        Ok(everruns_core::EgressResponse {
            status: 200,
            headers: [("content-type".to_string(), "application/json".to_string())].into(),
            body: serde_json::to_vec(&reply).unwrap(),
        })
    }

    async fn send_stream(
        &self,
        _request: everruns_core::EgressRequest,
    ) -> everruns_core::EgressResult<everruns_core::EgressStreamResponse> {
        Err(everruns_core::EgressError::Transport("no streaming".into()))
    }
}

/// Where and how the fake server delivers for one subscription.
struct Subscription {
    path: String,
    secret: String,
    id: String,
}

impl Subscription {
    /// POST a body signed as the server would, at `timestamp`.
    async fn post_at(
        &self,
        server: &TestServer,
        webhook_id: &str,
        timestamp: i64,
        body: &[u8],
    ) -> TestResponse {
        let key = standard_webhooks::decode_secret(&self.secret).unwrap();
        let timestamp = timestamp.to_string();
        let signature = standard_webhooks::sign(&key, webhook_id, &timestamp, body);
        server
            .request_raw(
                Method::POST,
                &self.path,
                vec![
                    ("content-type", "application/json"),
                    (standard_webhooks::HEADER_ID, webhook_id),
                    (standard_webhooks::HEADER_TIMESTAMP, &timestamp),
                    (standard_webhooks::HEADER_SIGNATURE, &signature),
                    (standard_webhooks::HEADER_SUBSCRIPTION_ID, &self.id),
                ],
                body.to_vec(),
            )
            .await
    }

    async fn post(&self, server: &TestServer, webhook_id: &str, body: &Value) -> TestResponse {
        let body = serde_json::to_vec(body).unwrap();
        self.post_at(server, webhook_id, chrono::Utc::now().timestamp(), &body)
            .await
    }
}

fn event(title: &str, cursor: &str) -> Value {
    json!({
        "eventId": format!("evt_{title}"),
        "name": EVENT,
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "data": { "issue": { "id": "ISS-1", "title": title } },
        "cursor": cursor,
    })
}

async fn create_agent(server: &TestServer) -> String {
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": format!("mcp-events-{}", uuid::Uuid::now_v7().simple()),
                "system_prompt": "triage issues",
                "harness_id": server.seed_generic_harness_id.clone(),
                "mcpServers": {
                    "tracker": {
                        "url": MCP_URL,
                        "headers": { "Authorization": "Bearer tracker-token" },
                    }
                },
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    agent["id"].as_str().unwrap().to_string()
}

fn trigger_request(server_name: &str) -> Value {
    json!({
        "trigger_type": "mcp_event",
        "mcp_server": server_name,
        "mcp_event": EVENT,
        "mcp_event_arguments": { "team": "eng" },
        "session_mode": "session_per_invocation",
        "message": "New issue {{payload.issue.title}} from {{mcp.server}}",
        "enabled": true,
    })
}

async fn create_trigger(server: &TestServer, agent_id: &str) -> Value {
    server
        .post(
            &format!("/v1/agents/{agent_id}/triggers"),
            trigger_request("tracker"),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json()
}

fn trigger_id(trigger: &Value) -> TriggerId {
    trigger["id"].as_str().unwrap().parse().unwrap()
}

#[tokio::test]
async fn create_subscribes_and_a_signed_event_starts_a_run() {
    let server = TestServer::in_memory().await;
    let fake = FakeEventsServer::install(&server);
    let agent_id = create_agent(&server).await;
    let trigger = create_trigger(&server, &agent_id).await;
    assert_eq!(trigger["trigger_type"], "mcp_event");
    assert_eq!(trigger["config"]["server"], "tracker");
    assert!(
        trigger["config"].get("secret").is_none(),
        "the secret never leaves the subscription row"
    );

    // Subscribed once, as the agent's attachment: same URL and credential.
    let subscribes = fake.calls("events/subscribe");
    assert_eq!(subscribes.len(), 1);
    let call = &subscribes[0];
    assert_eq!(call.authorization.as_deref(), Some("Bearer tracker-token"));
    assert_eq!(call.params["name"], EVENT);
    assert_eq!(call.params["arguments"], json!({ "team": "eng" }));
    assert_eq!(call.params["delivery"]["mode"], "webhook");
    let ingress_id = trigger["ingress_id"].as_str().unwrap();
    assert!(
        call.params["delivery"]["url"]
            .as_str()
            .unwrap()
            .ends_with(&format!("/v1/e/{ingress_id}/mcp-events"))
    );
    let secret = call.params["delivery"]["secret"].as_str().unwrap();
    assert!(standard_webhooks::decode_secret(secret).is_ok());

    let stored = server
        .db
        .get_agent_trigger_mcp_subscription(trigger_id(&trigger))
        .await
        .unwrap()
        .expect("subscription stored");
    assert_eq!(stored.status, "active");
    assert_eq!(stored.remote_subscription_id.as_deref(), Some("sub_1"));
    assert_eq!(stored.cursor.as_deref(), Some("cursor_1"));
    assert!(
        !String::from_utf8_lossy(&stored.secret_encrypted).contains(secret),
        "the secret is stored encrypted"
    );

    let subscription = fake.subscription();
    // The verification handshake echoes the challenge.
    let challenge: Value = subscription
        .post(
            &server,
            "msg_challenge",
            &json!({ "type": "verification", "challenge": "abc123" }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(challenge, json!({ "challenge": "abc123" }));

    let delivered: Value = subscription
        .post(&server, "msg_1", &event("Broken login", "cursor_2"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(delivered["delivery"], "dispatched");
    let session_id: SessionId = delivered["session_id"].as_str().unwrap().parse().unwrap();
    let session = server
        .db
        .get_session(DEFAULT_ORG_ID, session_id)
        .await
        .unwrap()
        .expect("session started");
    assert_eq!(
        SessionSource::from(session.source.as_str()),
        SessionSource::Webhook
    );
    let messages = server
        .get(&format!("/v1/sessions/{session_id}/messages"))
        .await
        .assert_success()
        .json_value()
        .to_string();
    assert!(
        messages.contains("New issue Broken login from tracker"),
        "rendered message: {messages}"
    );

    // The cursor advances with each delivery, for the next refresh.
    let stored = server
        .db
        .get_agent_trigger_mcp_subscription(trigger_id(&trigger))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.cursor.as_deref(), Some("cursor_2"));
}

#[tokio::test]
async fn deliveries_failing_verification_are_refused() {
    let server = TestServer::in_memory().await;
    let fake = FakeEventsServer::install(&server);
    let agent_id = create_agent(&server).await;
    create_trigger(&server, &agent_id).await;
    let subscription = fake.subscription();
    let body = serde_json::to_vec(&event("x", "c")).unwrap();
    let now = chrono::Utc::now().timestamp();

    // Signed with another secret.
    let forged = Subscription {
        secret: standard_webhooks::generate_secret(),
        path: subscription.path.clone(),
        id: subscription.id.clone(),
    };
    forged
        .post_at(&server, "msg_forged", now, &body)
        .await
        .assert_status(StatusCode::UNAUTHORIZED);

    // Correctly signed, but outside the timestamp window either way.
    subscription
        .post_at(&server, "msg_stale", now - 600, &body)
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
    subscription
        .post_at(&server, "msg_future", now + 600, &body)
        .await
        .assert_status(StatusCode::UNAUTHORIZED);

    // No signature at all.
    server
        .request_raw(
            Method::POST,
            &subscription.path,
            vec![("content-type", "application/json")],
            body.clone(),
        )
        .await
        .assert_status(StatusCode::UNAUTHORIZED);

    // Over the 256 KiB cap: refused before anything is checked.
    let oversize = vec![b' '; standard_webhooks::MAX_BODY_BYTES + 1];
    subscription
        .post_at(&server, "msg_big", now, &oversize)
        .await
        .assert_status(StatusCode::PAYLOAD_TOO_LARGE);

    // Signed, but for an event this trigger did not subscribe to.
    let mut other = event("x", "c");
    other["name"] = json!("issue.deleted");
    subscription
        .post(&server, "msg_other", &other)
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_replayed_delivery_is_a_duplicate() {
    let server = TestServer::in_memory().await;
    let fake = FakeEventsServer::install(&server);
    let agent_id = create_agent(&server).await;
    create_trigger(&server, &agent_id).await;
    let subscription = fake.subscription();
    let body = event("Once", "c1");

    let first: Value = subscription
        .post(&server, "msg_replay", &body)
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(first["delivery"], "dispatched");
    let replay: Value = subscription
        .post(&server, "msg_replay", &body)
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(replay["delivery"], "duplicate");
    assert!(replay["session_id"].is_null());
}

#[tokio::test]
async fn unknown_and_superseded_subscriptions_are_gone() {
    let server = TestServer::in_memory().await;
    let fake = FakeEventsServer::install(&server);
    let agent_id = create_agent(&server).await;
    create_trigger(&server, &agent_id).await;
    let subscription = fake.subscription();

    let unknown = Subscription {
        path: format!("/v1/e/appchan_{}/mcp-events", uuid::Uuid::now_v7().simple()),
        secret: subscription.secret.clone(),
        id: subscription.id.clone(),
    };
    unknown
        .post(&server, "msg_unknown", &event("x", "c"))
        .await
        .assert_status(StatusCode::GONE);

    let superseded = Subscription {
        id: "sub_old".to_string(),
        path: subscription.path.clone(),
        secret: subscription.secret.clone(),
    };
    superseded
        .post(&server, "msg_superseded", &event("x", "c"))
        .await
        .assert_status(StatusCode::GONE);
}

#[tokio::test]
async fn refresh_resubscribes_with_the_same_secret_and_cursor() {
    refresh_resubscribes(TestServer::in_memory().await).await;
}

/// The same lifecycle against PostgreSQL, for the SQL storage path.
#[tokio::test]
async fn refresh_resubscribes_on_postgres() {
    refresh_resubscribes(TestServer::new().await).await;
}

async fn refresh_resubscribes(server: TestServer) {
    let fake = FakeEventsServer::install(&server);
    // Due inside the refresh margin from the start.
    *fake.refresh_before.lock() =
        Some((chrono::Utc::now() + chrono::Duration::minutes(1)).to_rfc3339());
    let agent_id = create_agent(&server).await;
    let trigger = create_trigger(&server, &agent_id).await;
    let first = fake.subscription();
    first
        .post(&server, "msg_before_refresh", &event("x", "cursor_seen"))
        .await
        .assert_status(StatusCode::OK);

    *fake.refresh_before.lock() =
        Some((chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339());
    // A shared PostgreSQL database may hold other tests' due rows too, so
    // only this trigger's calls are counted.
    let callback = format!("{}/mcp-events", trigger["ingress_id"].as_str().unwrap());
    let ours = || {
        fake.calls("events/subscribe")
            .into_iter()
            .filter(|call| {
                call.params["delivery"]["url"]
                    .as_str()
                    .is_some_and(|url| url.ends_with(&callback))
            })
            .collect::<Vec<_>>()
    };
    assert!(server.mcp_event_triggers.refresh_due().await >= 1);

    let subscribes = ours();
    assert_eq!(subscribes.len(), 2);
    let refresh = &subscribes[1].params;
    assert_eq!(
        refresh["delivery"]["secret"].as_str(),
        Some(first.secret.as_str())
    );
    assert_eq!(refresh["cursor"], "cursor_seen");
    let stored = server
        .db
        .get_agent_trigger_mcp_subscription(trigger_id(&trigger))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, "active");
    assert_ne!(stored.remote_subscription_id, Some(first.id.clone()));
    assert!(stored.refresh_before.unwrap() > chrono::Utc::now() + chrono::Duration::hours(1));

    // Nothing of ours is due any more.
    server.mcp_event_triggers.refresh_due().await;
    assert_eq!(ours().len(), 2);

    // Deleting drops the row, so it is never refreshed again.
    let path = format!(
        "/v1/agents/{agent_id}/triggers/{}",
        trigger["id"].as_str().unwrap()
    );
    server.delete(&path).await.assert_success();
    assert!(
        server
            .db
            .get_agent_trigger_mcp_subscription(trigger_id(&trigger))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn disable_enable_and_delete_follow_the_subscription() {
    let server = TestServer::in_memory().await;
    let fake = FakeEventsServer::install(&server);
    let agent_id = create_agent(&server).await;
    let trigger = create_trigger(&server, &agent_id).await;
    let path = format!(
        "/v1/agents/{agent_id}/triggers/{}",
        trigger["id"].as_str().unwrap()
    );
    let first = fake.subscription();

    server
        .patch(&path, json!({ "enabled": false }))
        .await
        .assert_success();
    let unsubscribes = fake.calls("events/unsubscribe");
    assert_eq!(unsubscribes.len(), 1);
    assert_eq!(unsubscribes[0].params["id"], "sub_1");
    assert!(
        server
            .db
            .get_agent_trigger_mcp_subscription(trigger_id(&trigger))
            .await
            .unwrap()
            .is_none()
    );
    // The server's next delivery learns the subscription is over.
    first
        .post(&server, "msg_after_disable", &event("x", "c"))
        .await
        .assert_status(StatusCode::GONE);

    // Re-enabling subscribes afresh, with a new secret.
    server
        .patch(&path, json!({ "enabled": true }))
        .await
        .assert_success();
    let second = fake.subscription();
    assert_ne!(second.secret, first.secret);
    first
        .post(&server, "msg_old_secret", &event("x", "c"))
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
    second
        .post(&server, "msg_new_secret", &event("x", "c"))
        .await
        .assert_status(StatusCode::OK);

    // Changing the arguments replaces the subscription.
    server
        .patch(&path, json!({ "mcp_event_arguments": { "team": "ops" } }))
        .await
        .assert_success();
    assert_eq!(fake.calls("events/unsubscribe").len(), 2);
    let subscribes = fake.calls("events/subscribe");
    assert_eq!(subscribes.len(), 3);
    assert_eq!(subscribes[2].params["arguments"], json!({ "team": "ops" }));

    server.delete(&path).await.assert_success();
    assert_eq!(fake.calls("events/unsubscribe").len(), 3);
    fake.subscription()
        .post(&server, "msg_after_delete", &event("x", "c"))
        .await
        .assert_status(StatusCode::GONE);
}

#[tokio::test]
async fn a_rejected_subscribe_leaves_no_trigger_behind() {
    let server = TestServer::in_memory().await;
    let fake = FakeEventsServer::install(&server);
    fake.reject_subscribe.store(true, Ordering::SeqCst);
    let agent_id = create_agent(&server).await;

    let refused = server
        .post(
            &format!("/v1/agents/{agent_id}/triggers"),
            trigger_request("tracker"),
        )
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        refused.text().contains("unknown team"),
        "{}",
        refused.text()
    );
    let listed: Value = server
        .get(&format!("/v1/agents/{agent_id}/triggers"))
        .await
        .assert_success()
        .json();
    assert_eq!(listed.as_array().map(Vec::len), Some(0), "{listed}");
}

#[tokio::test]
async fn create_validates_the_server_attachment_and_flag() {
    let server = TestServer::in_memory().await;
    let fake = FakeEventsServer::install(&server);
    let agent_id = create_agent(&server).await;
    let triggers = format!("/v1/agents/{agent_id}/triggers");

    let missing = server
        .post(&triggers, trigger_request("nope"))
        .await
        .assert_status(StatusCode::BAD_REQUEST);
    assert!(missing.text().contains("no MCP server named 'nope'"));
    let mut no_event = trigger_request("tracker");
    no_event["mcp_event"] = Value::Null;
    server
        .post(&triggers, no_event)
        .await
        .assert_status(StatusCode::BAD_REQUEST);
    let mut bad_arguments = trigger_request("tracker");
    bad_arguments["mcp_event_arguments"] = json!(["eng"]);
    server
        .post(&triggers, bad_arguments)
        .await
        .assert_status(StatusCode::BAD_REQUEST);
    assert!(fake.calls("events/subscribe").is_empty());

    // With the org's `mcp_events` flag off, nothing subscribes.
    let flags: HashMap<String, bool> = [("mcp_events".to_string(), false)].into();
    server
        .db
        .replace_org_feature_flags(DEFAULT_ORG_ID, &flags)
        .await
        .unwrap();
    server
        .post(&triggers, trigger_request("tracker"))
        .await
        .assert_status(StatusCode::NOT_FOUND);
    assert!(fake.calls("events/subscribe").is_empty());
}

#[tokio::test]
async fn deliveries_stop_when_the_flag_is_turned_off() {
    let server = TestServer::in_memory().await;
    let fake = FakeEventsServer::install(&server);
    let agent_id = create_agent(&server).await;
    create_trigger(&server, &agent_id).await;
    let subscription = fake.subscription();

    let flags: HashMap<String, bool> = [("mcp_events".to_string(), false)].into();
    server
        .db
        .replace_org_feature_flags(DEFAULT_ORG_ID, &flags)
        .await
        .unwrap();
    subscription
        .post(&server, "msg_flag_off", &event("x", "c"))
        .await
        .assert_status(StatusCode::GONE);
}
