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
    server
        .request_raw(
            Method::POST,
            "/mcp",
            vec![
                ("content-type", "application/json"),
                ("MCP-Protocol-Version", LATEST),
            ],
            serde_json::to_vec(&json!({
                "jsonrpc": "2.0", "id": 1, "method": method, "params": params
            }))
            .unwrap(),
        )
        .await
        .assert_success()
        .json()
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
async fn member(server: &TestServer) -> Subscriber {
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

fn turn_completed(session_id: &str) -> everruns_core::Event {
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn events_exist_only_for_orgs_that_opted_in() {
    let server = TestServer::in_memory().await;
    let discover = rpc(&server, "server/discover", json!({})).await;
    assert_eq!(discover["result"]["capabilities"]["events"], json!({}));
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
    assert!(discover["result"]["capabilities"].get("events").is_none());
    let list = rpc(&server, "events/list", json!({})).await;
    assert_eq!(list["error"]["code"], -32601);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn anonymous_mcp_cannot_subscribe() {
    let server = TestServer::in_memory().await;
    let params = subscribe_params("session.completed", json!({}));
    let resp = rpc(&server, "events/subscribe", params.clone()).await;
    assert_eq!(resp["error"]["code"], -32602);
    assert!(resp["error"]["message"].as_str().unwrap().contains("user"));
    let resp = rpc(&server, "events/unsubscribe", params).await;
    assert_eq!(resp["error"]["code"], -32602);
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
    assert_eq!(error.code, -32015);
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
async fn failed_deliveries_retry_and_gone_ends_the_subscription() {
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
        0,
        "410 removed the subscription"
    );
    assert_eq!(deliveries(&server).len(), 4);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsubscribe_leaving_the_org_and_opting_out_stop_delivery() {
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
    assert_eq!(
        server
            .mcp_events
            .dispatch(&turn_completed(&session_id))
            .await,
        0
    );

    // The org turning the flag off silences existing subscriptions.
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
        0
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
}
