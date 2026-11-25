//! Integration tests for outbound MCP Events on `/mcp` (EVE-1121).
//!
//! Discovery and gating go through the JSON-RPC surface. The harness runs
//! `/mcp` anonymously, and anonymous MCP is not a user (TM-MCP-006), so
//! subscriptions for a real member go to the service directly. Callbacks land
//! in the harness's `WebhookReceiver` instead of the network, and events are
//! handed to the service the way the listener does. See
//! `knowledge/integrations/mcp-events.md`.
//!
//! Run with: cargo test -p everruns-server --test domain mcp_events_test::

use std::collections::HashMap;
use std::sync::atomic::Ordering;

use axum::http::Method;
use base64::Engine as _;
use everruns_core::DEFAULT_ORG_ID;
use everruns_core::events::deserialize_event_data;
use everruns_server::domains::mcp_servers::events::{
    EventsError, Subscribed, Subscriber, SubscriptionTarget, verify_signature,
};
use serde_json::{Value, json};

use crate::test_harness::TestServer;

const LATEST: &str = "2026-07-28";
const HOOK: &str = "https://8.8.8.8/hooks/everruns";

fn secret() -> String {
    format!(
        "whsec_{}",
        base64::engine::general_purpose::STANDARD.encode([42u8; 32])
    )
}

async fn rpc(server: &TestServer, method: &str, params: Value) -> Value {
    let mut headers = vec![
        ("content-type", "application/json"),
        ("MCP-Protocol-Version", LATEST),
    ];
    if let Some(name) = params.get("name").and_then(Value::as_str) {
        headers.push(("Mcp-Name", name));
    }
    let response: Value = server
        .request_raw(
            Method::POST,
            "/mcp",
            headers,
            serde_json::to_vec(&json!({
                "jsonrpc": "2.0", "id": 1, "method": method, "params": params
            }))
            .unwrap(),
        )
        .await
        .assert_success()
        .json();
    if method.starts_with("events/") && response.get("result").is_some() {
        assert_eq!(response["result"]["resultType"], "complete", "{method}");
    }
    response
}

async fn tool(server: &TestServer, name: &str, arguments: Value) -> Value {
    let resp = rpc(
        server,
        "tools/call",
        json!({ "name": name, "arguments": arguments }),
    )
    .await;
    assert_ne!(resp["result"]["isError"], true, "{name} failed: {resp}");
    resp["result"].clone()
}

/// A session to announce, with its agent.
async fn session(server: &TestServer) -> (String, String) {
    let name = format!("events-{}", uuid::Uuid::now_v7().simple());
    let created = tool(
        server,
        "execute",
        json!({ "commands": format!(
            "create_agent --name '{name}' --display_name 'Events' --system_prompt 'x'"
        ) }),
    )
    .await;
    let agent: Value =
        serde_json::from_str(created["content"][0]["text"].as_str().unwrap()).unwrap();
    let agent_id = agent["id"].as_str().unwrap().to_string();
    let run = tool(
        server,
        "agent_run",
        json!({ "agent_id": agent_id, "message": "go" }),
    )
    .await;
    let session_id = run["structuredContent"]["session_id"]
        .as_str()
        .unwrap()
        .to_string();
    (session_id, agent_id)
}

fn subscribe_params(name: &str, arguments: Value) -> Value {
    json!({
        "name": name,
        "arguments": arguments,
        "delivery": { "mode": "webhook", "url": HOOK, "secret": secret() },
    })
}

/// A real org member to subscribe as.
pub(super) async fn member(server: &TestServer) -> Subscriber {
    let user = server
        .db
        .create_user(everruns_server::storage::CreateUserRow {
            email: format!("{}@example.com", uuid::Uuid::now_v7().simple()),
            name: "Events Member".to_string(),
            avatar_url: None,
            roles: vec!["member".to_string()],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .unwrap();
    server
        .db
        .add_organization_member(DEFAULT_ORG_ID, user.id, "member")
        .await
        .unwrap();
    Subscriber {
        org_id: DEFAULT_ORG_ID,
        user_id: user.id,
    }
}

async fn subscribe(
    server: &TestServer,
    subscriber: &Subscriber,
    params: Value,
) -> Result<Subscribed, EventsError> {
    let target = SubscriptionTarget::from_params(&params)?;
    let secret = params["delivery"]["secret"].as_str().unwrap_or_default();
    server
        .mcp_events
        .subscribe(subscriber, &target, secret, None)
        .await
}

fn event(session_id: &str, event_type: &str, data: Value) -> everruns_core::Event {
    everruns_core::Event {
        id: everruns_contracts::typed_id::EventId::new(),
        event_type: event_type.to_string(),
        ts: chrono::Utc::now(),
        session_id: session_id.parse().expect("session id"),
        context: Default::default(),
        data: deserialize_event_data(event_type, data),
        metadata: None,
        tags: None,
        sequence: None,
    }
}

pub(super) fn turn_completed(session_id: &str) -> everruns_core::Event {
    event(
        session_id,
        "turn.completed",
        json!({ "turn_id": everruns_contracts::typed_id::TurnId::new().to_string(), "iterations": 1 }),
    )
}

/// Event deliveries (verification challenges left out), as parsed bodies.
fn deliveries(server: &TestServer) -> Vec<(everruns_core::EgressRequest, Value)> {
    server
        .webhooks
        .requests
        .lock()
        .iter()
        .map(|request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            (request.clone(), body)
        })
        .filter(|(_, body)| body["type"] != "verification")
        .collect()
}

fn assert_signed(request: &everruns_core::EgressRequest) {
    let header = |name: &str| request.headers.get(name).cloned().unwrap_or_default();
    assert!(
        verify_signature(
            &secret(),
            &header("webhook-id"),
            &header("webhook-timestamp"),
            &request.body,
            &header("webhook-signature"),
        )
        .unwrap(),
        "bad signature"
    );
}

struct CallbackFailure(Option<u16>, &'static str);

struct DelayedCallbackConsent;

#[async_trait::async_trait]
impl everruns_core::EgressService for DelayedCallbackConsent {
    async fn send(
        &self,
        request: everruns_core::EgressRequest,
    ) -> everruns_core::EgressResult<everruns_core::EgressResponse> {
        assert_signed(&request);
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        // Make both concurrent subscribes reach the shared initialization.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        Ok(everruns_core::EgressResponse {
            status: 200,
            headers: Default::default(),
            body: serde_json::to_vec(&json!({"challenge":body["challenge"]})).unwrap(),
        })
    }
    async fn send_stream(
        &self,
        _: everruns_core::EgressRequest,
    ) -> everruns_core::EgressResult<everruns_core::EgressStreamResponse> {
        Err(everruns_core::EgressError::invalid("unused test transport"))
    }
}

#[async_trait::async_trait]
impl everruns_core::EgressService for CallbackFailure {
    async fn send(
        &self,
        _: everruns_core::EgressRequest,
    ) -> everruns_core::EgressResult<everruns_core::EgressResponse> {
        match self.0 {
            Some(status) => Ok(everruns_core::EgressResponse {
                status,
                headers: Default::default(),
                body: b"private callback response".to_vec(),
            }),
            None => Err(everruns_core::EgressError::Transport(self.1.to_string())),
        }
    }

    async fn send_stream(
        &self,
        _: everruns_core::EgressRequest,
    ) -> everruns_core::EgressResult<everruns_core::EgressStreamResponse> {
        Err(everruns_core::EgressError::invalid("unused test transport"))
    }
}

#[tokio::test]
async fn callback_failures_use_categories_and_never_cache_failed_consent() {
    let server = TestServer::in_memory().await;
    let subscriber = member(&server).await;
    let params = subscribe_params("session.completed", json!({}));
    for (status, detail, reason) in [
        (Some(400), "", "http_4xx"),
        (Some(503), "", "http_5xx"),
        (Some(200), "", "challenge_failed"),
        (None, "timed out", "timeout"),
        (None, "TLS certificate failure", "tls_error"),
        (None, "connection refused", "connection_refused"),
    ] {
        *server.webhooks.forward.write() =
            Some(std::sync::Arc::new(CallbackFailure(status, detail)));
        let error = subscribe(&server, &subscriber, params.clone())
            .await
            .unwrap_err();
        assert_eq!(error.code, -32027);
        assert_eq!(error.data, Some(json!({"reason":reason})));
    }
    *server.webhooks.forward.write() = None;
    subscribe(&server, &subscriber, params).await.unwrap();
    assert_eq!(
        server.webhooks.requests.lock().len(),
        7,
        "failed consent is retried"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn events_are_available_despite_retired_org_override() {
    let server = TestServer::in_memory().await;
    let discover = rpc(&server, "server/discover", json!({})).await;
    assert_eq!(
        discover["result"]["capabilities"]["extensions"]["io.modelcontextprotocol/events"],
        json!({})
    );
    let list = rpc(&server, "events/list", json!({})).await;
    let names: Vec<&str> = list["result"]["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "session.completed",
            "session.failed",
            "session.input_required"
        ]
    );
    assert_eq!(list["result"]["events"][0]["delivery"], json!(["webhook"]));

    server
        .db
        .replace_org_feature_flags(
            DEFAULT_ORG_ID,
            &HashMap::from([("mcp_events".to_string(), false)]),
        )
        .await
        .unwrap();
    let discover = rpc(&server, "server/discover", json!({})).await;
    assert_eq!(
        discover["result"]["capabilities"]["extensions"]["io.modelcontextprotocol/events"],
        json!({})
    );
    let list = rpc(&server, "events/list", json!({})).await;
    assert_eq!(list["result"]["events"].as_array().unwrap().len(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn anonymous_mcp_cannot_subscribe() {
    let server = TestServer::in_memory().await;
    let params = subscribe_params("session.completed", json!({}));
    let resp = rpc(&server, "events/subscribe", params.clone()).await;
    assert_eq!(resp["error"]["code"], -32024);
    assert!(resp["error"]["message"].as_str().unwrap().contains("user"));
    let resp = rpc(&server, "events/unsubscribe", params).await;
    assert_eq!(resp["error"]["code"], -32024);
    assert!(
        server.webhooks.requests.lock().is_empty(),
        "no callback is contacted for a refused subscriber"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subscribe_verifies_the_callback_and_is_idempotent() {
    let server = TestServer::in_memory().await;
    let subscriber = member(&server).await;
    let params = subscribe_params("session.completed", json!({}));
    let first = subscribe(&server, &subscriber, params.clone())
        .await
        .unwrap();

    let challenge = server.webhooks.requests.lock()[0].clone();
    let body: Value = serde_json::from_slice(&challenge.body).unwrap();
    assert_eq!(body["type"], "verification");
    assert_eq!(challenge.headers["x-mcp-subscription-id"], first.id);
    assert_signed(&challenge);

    let again = subscribe(&server, &subscriber, params).await.unwrap();
    assert_eq!(again.id, first.id);
    assert_eq!(
        server.webhooks.requests.lock().len(),
        1,
        "refresh reuses consent"
    );
    let rows = server
        .db
        .list_active_mcp_event_subscriptions(
            DEFAULT_ORG_ID,
            "session.completed",
            chrono::Utc::now(),
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "a repeated subscribe refreshes, not stacks");
    assert!(!String::from_utf8_lossy(&rows[0].secret_encrypted).contains(&secret()));

    let other = subscribe(
        &server,
        &member(&server).await,
        subscribe_params("session.completed", json!({})),
    )
    .await
    .unwrap();
    assert_ne!(other.id, first.id, "subscriptions are per subscriber");
    assert_eq!(
        server.webhooks.requests.lock().len(),
        2,
        "another principal proves consent"
    );
}

#[tokio::test]
async fn callback_consent_covers_arguments_and_coalesces_concurrent_subscribes() {
    let server = TestServer::in_memory().await;
    *server.webhooks.forward.write() = Some(std::sync::Arc::new(DelayedCallbackConsent));
    let subscriber = member(&server).await;
    let (session_id, _) = session(&server).await;
    let all = subscribe_params("session.completed", json!({}));
    let filtered = subscribe_params("session.completed", json!({"session_id":session_id}));
    let (first, second) = tokio::join!(
        subscribe(&server, &subscriber, all.clone()),
        subscribe(&server, &subscriber, filtered),
    );
    assert_ne!(first.unwrap().id, second.unwrap().id);
    assert_eq!(server.webhooks.requests.lock().len(), 1);
    subscribe(&server, &subscriber, all.clone()).await.unwrap();
    assert_eq!(server.webhooks.requests.lock().len(), 1);
    let mut other_url = all;
    other_url["delivery"]["url"] = json!(format!("{HOOK}/other"));
    subscribe(&server, &subscriber, other_url).await.unwrap();
    assert_eq!(
        server.webhooks.requests.lock().len(),
        2,
        "consent is URL scoped"
    );
}

#[tokio::test]
async fn one_event_has_distinct_signed_message_ids_on_two_subscriptions() {
    let server = TestServer::in_memory().await;
    let subscriber = member(&server).await;
    let (session_id, _) = session(&server).await;
    for arguments in [json!({}), json!({"session_id":session_id})] {
        subscribe(
            &server,
            &subscriber,
            subscribe_params("session.completed", arguments),
        )
        .await
        .unwrap();
    }
    let event = turn_completed(&session_id);
    assert_eq!(server.mcp_events.dispatch(&event).await, 2);
    let messages = deliveries(&server);
    assert_eq!(messages.len(), 2);
    assert_ne!(
        messages[0].0.headers["webhook-id"],
        messages[1].0.headers["webhook-id"]
    );
    for (request, body) in messages {
        assert_signed(&request);
        assert_eq!(body["eventId"], event.id.to_string());
        let nonce = request.headers["webhook-id"].strip_prefix("msg_").unwrap();
        assert_eq!(uuid::Uuid::parse_str(nonce).unwrap().get_version_num(), 4);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subscribe_refuses_unsafe_callbacks_and_bad_secrets() {
    let server = TestServer::in_memory().await;
    let subscriber = member(&server).await;
    let with = |url: &str, secret: &str| {
        json!({
            "name": "session.failed",
            "delivery": { "mode": "webhook", "url": url, "secret": secret },
        })
    };
    for (url, secret) in [
        ("http://8.8.8.8/hook", secret()),
        ("https://127.0.0.1/hook", secret()),
        ("https://10.0.0.5/hook", secret()),
        (HOOK, "not-a-secret".to_string()),
        (HOOK, "whsec_c2hvcnQ=".to_string()),
    ] {
        let error = subscribe(&server, &subscriber, with(url, &secret))
            .await
            .unwrap_err();
        assert_eq!(error.code, -32602, "{url} {secret}: {error:?}");
    }
    assert!(server.webhooks.requests.lock().is_empty());

    server
        .webhooks
        .refuse_verification
        .store(true, Ordering::SeqCst);
    let error = subscribe(&server, &subscriber, with(HOOK, &secret()))
        .await
        .unwrap_err();
    assert_eq!(error.code, -32027);
    assert_eq!(error.data, Some(json!({ "reason": "challenge_failed" })));
    let rows = server
        .db
        .list_active_mcp_event_subscriptions(DEFAULT_ORG_ID, "session.failed", chrono::Utc::now())
        .await
        .unwrap();
    assert!(rows.is_empty(), "an unverified callback is never stored");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completed_session_is_delivered_signed_to_matching_subscriptions_only() {
    let server = TestServer::in_memory().await;
    let subscriber = member(&server).await;
    let (session_id, agent_id) = session(&server).await;
    let (other_session, _) = session(&server).await;
    for filtered_to in [&session_id, &other_session] {
        let params = subscribe_params("session.completed", json!({ "session_id": filtered_to }));
        subscribe(&server, &subscriber, params).await.unwrap();
    }

    let event = turn_completed(&session_id);
    assert_eq!(server.mcp_events.dispatch(&event).await, 1);
    let delivered = deliveries(&server);
    assert_eq!(delivered.len(), 1);
    let (request, body) = &delivered[0];
    assert_signed(request);
    assert_eq!(body["name"], "session.completed");
    assert_eq!(body["eventId"], event.id.to_string());
    assert_eq!(body["data"]["session_id"], session_id);
    assert_eq!(body["data"]["agent_id"], agent_id);
    assert!(
        body["data"]["link"]
            .as_str()
            .unwrap()
            .ends_with(&format!("/sessions/{session_id}/chat"))
    );
    // Identifiers and state only: no transcript text rides the webhook.
    assert!(body["data"].get("final_answer_preview").is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn questions_approvals_and_failures_are_announced() {
    let server = TestServer::in_memory().await;
    let subscriber = member(&server).await;
    let (session_id, _) = session(&server).await;
    for name in ["session.input_required", "session.failed"] {
        subscribe(&server, &subscriber, subscribe_params(name, json!({})))
            .await
            .unwrap();
    }

    let question = event(
        &session_id,
        "tool.call_requested",
        json!({ "tool_calls": [{ "id": "call_q", "name": "ask_user", "arguments": {} }] }),
    );
    let approval = event(
        &session_id,
        "tool.completed",
        json!({
            "tool_call_id": "call_a",
            "tool_name": "request_approval",
            "success": true,
            "status": "success",
            "result": [{ "type": "text", "text": json!({
                "awaiting_approval": true, "action": "Deploy", "question": "Go?"
            }).to_string() }],
        }),
    );
    let failed = event(
        &session_id,
        "turn.failed",
        json!({
            "turn_id": everruns_contracts::typed_id::TurnId::new().to_string(),
            "error": "provider said no: sk-secret",
            "error_code": "model_error",
        }),
    );
    let unrelated = event(
        &session_id,
        "tool.call_requested",
        json!({ "tool_calls": [{ "id": "call_x", "name": "bash", "arguments": {} }] }),
    );
    for event in [&question, &approval, &failed] {
        assert_eq!(
            server.mcp_events.dispatch(event).await,
            1,
            "{}",
            event.event_type
        );
    }
    assert_eq!(server.mcp_events.dispatch(&unrelated).await, 0);

    let bodies: Vec<Value> = deliveries(&server).into_iter().map(|(_, b)| b).collect();
    assert_eq!(bodies[0]["data"]["kind"], "question");
    assert_eq!(bodies[0]["data"]["tool_call_id"], "call_q");
    assert_eq!(bodies[1]["data"]["kind"], "approval");
    assert_eq!(bodies[2]["name"], "session.failed");
    assert_eq!(bodies[2]["data"]["error_code"], "model_error");
    assert!(!bodies[2].to_string().contains("sk-secret"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_deliveries_retry_and_gone_only_abandons_one_delivery() {
    let server = TestServer::in_memory().await;
    let subscriber = member(&server).await;
    let (session_id, _) = session(&server).await;
    subscribe(
        &server,
        &subscriber,
        subscribe_params("session.completed", json!({})),
    )
    .await
    .unwrap();

    server.webhooks.delivery_statuses.lock().extend([500, 503]);
    let completed = turn_completed(&session_id);
    assert_eq!(server.mcp_events.dispatch(&completed).await, 1);
    let attempts = deliveries(&server);
    assert_eq!(attempts.len(), 3, "two retries, then success");
    assert_eq!(
        attempts[0].0.headers["webhook-id"], attempts[2].0.headers["webhook-id"],
        "a retry is the same message"
    );

    server.webhooks.delivery_statuses.lock().push_back(410);
    assert_eq!(
        server
            .mcp_events
            .dispatch(&turn_completed(&session_id))
            .await,
        0
    );
    assert_eq!(deliveries(&server).len(), 4, "410 is not retried");
    assert_eq!(
        server
            .mcp_events
            .dispatch(&turn_completed(&session_id))
            .await,
        1,
        "410 must preserve the subscription"
    );
    assert_eq!(deliveries(&server).len(), 5);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsubscribe_and_leaving_the_org_stop_delivery() {
    let server = TestServer::in_memory().await;
    let subscriber = member(&server).await;
    let (session_id, _) = session(&server).await;
    let params = subscribe_params("session.completed", json!({}));
    let target = SubscriptionTarget::from_params(&params).unwrap();

    subscribe(&server, &subscriber, params.clone())
        .await
        .unwrap();
    server
        .mcp_events
        .unsubscribe(&subscriber, &target)
        .await
        .unwrap();
    let missing = server
        .mcp_events
        .unsubscribe(&subscriber, &target)
        .await
        .unwrap_err();
    assert_eq!(missing.code, -32023);
    assert_eq!(missing.data, Some(json!({"kind":"subscription"})));
    assert_eq!(
        server
            .mcp_events
            .dispatch(&turn_completed(&session_id))
            .await,
        0
    );

    // A stale opt-out cannot disable built-in events.
    subscribe(&server, &subscriber, params.clone())
        .await
        .unwrap();
    let flags = |on: bool| HashMap::from([("mcp_events".to_string(), on)]);
    server
        .db
        .replace_org_feature_flags(DEFAULT_ORG_ID, &flags(false))
        .await
        .unwrap();
    assert_eq!(
        server
            .mcp_events
            .dispatch(&turn_completed(&session_id))
            .await,
        1
    );
    server
        .db
        .replace_org_feature_flags(DEFAULT_ORG_ID, &flags(true))
        .await
        .unwrap();
    assert_eq!(
        server
            .mcp_events
            .dispatch(&turn_completed(&session_id))
            .await,
        1
    );

    server
        .db
        .remove_organization_member(DEFAULT_ORG_ID, subscriber.user_id)
        .await
        .unwrap();
    assert_eq!(
        server
            .mcp_events
            .dispatch(&turn_completed(&session_id))
            .await,
        0
    );
    let rows = server
        .db
        .list_active_mcp_event_subscriptions(
            DEFAULT_ORG_ID,
            "session.completed",
            chrono::Utc::now(),
        )
        .await
        .unwrap();
    assert!(rows.is_empty(), "a former member's subscription is dropped");
    let attempts = deliveries(&server);
    let (request, body) = attempts.last().unwrap();
    assert_signed(request);
    assert_eq!(body["type"], "terminated");
    assert_eq!(body["error"]["code"], -32024);
    assert!(body.get("data").is_none());
}

#[tokio::test]
async fn final_routing_headers_are_required_and_legacy_requests_remain_accepted() {
    let server = TestServer::in_memory().await;
    // Upstream discovery only runs these probes when poll is advertised.
    // Exercise the real endpoint's refusal paths without advertising a mode
    // this server does not implement.
    for method in ["events/poll", "events/stream"] {
        let missing = rpc(&server, method, json!({"name":"unknown.event"})).await;
        assert_eq!(missing["error"]["code"], -32023);
        assert_eq!(missing["error"]["data"]["kind"], "event");
        let unsupported = rpc(&server, method, json!({"name":"session.completed"})).await;
        assert_eq!(unsupported["error"]["code"], -32026);
        assert_eq!(unsupported["error"]["data"]["feature"], "deliveryMode");
    }
    for method in [
        "events/subscribe",
        "events/unsubscribe",
        "events/poll",
        "events/stream",
    ] {
        for name in [None, Some("wrong.event")] {
            let mut headers = vec![
                ("content-type", "application/json"),
                ("MCP-Protocol-Version", LATEST),
            ];
            if let Some(name) = name {
                headers.push(("Mcp-Name", name));
            }
            let response: Value = server.request_raw(Method::POST, "/mcp", headers, serde_json::to_vec(&json!({"jsonrpc":"2.0", "id":1, "method":method, "params":{"name":"session.completed"}})).unwrap()).await.assert_status(axum::http::StatusCode::BAD_REQUEST).json();
            assert_eq!(response["error"]["code"], -32020);
        }
    }
    let response: Value = server.request_raw(Method::POST, "/mcp", vec![("content-type", "application/json"), ("MCP-Protocol-Version", "2025-11-25")], serde_json::to_vec(&json!({"jsonrpc":"2.0", "id":1, "method":"events/subscribe", "params":subscribe_params("session.completed",json!({}))})).unwrap()).await.assert_success().json();
    assert_eq!(
        response["error"]["code"], -32024,
        "legacy reached authentication without a routing header"
    );
    assert!(server.webhooks.requests.lock().is_empty());
    let legacy: Value = server
        .request_raw(
            Method::POST,
            "/mcp",
            vec![
                ("content-type", "application/json"),
                ("MCP-Protocol-Version", "2025-11-25"),
            ],
            serde_json::to_vec(&json!({
                "jsonrpc": "2.0", "id": 2, "method": "events/list", "params": {}
            }))
            .unwrap(),
        )
        .await
        .assert_success()
        .json();
    assert!(legacy["result"]["events"].is_array());
    assert!(legacy["result"].get("resultType").is_none());
    let discovery = rpc(&server, "server/discover", json!({})).await;
    assert!(discovery["result"]["capabilities"].get("events").is_none());
    assert_eq!(
        discovery["result"]["capabilities"]["extensions"]["io.modelcontextprotocol/events"],
        json!({})
    );
}
