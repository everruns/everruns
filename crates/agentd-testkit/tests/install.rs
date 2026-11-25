//! agentd as a backend you push to (`agentd serve`, `install.rs`): one
//! process, agents and a model key pushed over the public API, sessions
//! created by agent id with labels, and the stream read back with a
//! cursor. The model is a stand-in for an OpenAI-compatible API, so the
//! test proves the pushed key is what a turn sends.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::http::HeaderMap;
use axum::routing::post;
use everruns_agentd::install_auth::Auth;
use everruns_agentd::serve::{ServeConfig, Served, start};
use parking_lot::Mutex;
use serde_json::{Value, json};

/// What the stand-in model API saw: the authorization header per call.
type Seen = Arc<Mutex<Vec<String>>>;

/// An OpenAI-compatible chat API that answers "pong".
async fn model_api() -> (String, Seen) {
    let seen: Seen = Arc::default();
    let log = seen.clone();
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |headers: HeaderMap| {
            let log = log.clone();
            async move {
                let auth = headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default()
                    .to_owned();
                log.lock().push(auth);
                let sse = [
                    r#"data: {"id":"1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"pong"},"finish_reason":null}]}"#,
                    "",
                    r#"data: {"id":"1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4}}"#,
                    "",
                    "data: [DONE]",
                    "",
                    "",
                ]
                .join("\n");
                (
                    [
                        ("content-type", "text/event-stream"),
                        ("connection", "close"),
                    ],
                    sse,
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await });
    (url, seen)
}

fn config(data: &std::path::Path, dev: bool, auth: Auth) -> ServeConfig {
    ServeConfig {
        data: data.to_owned(),
        listen: "127.0.0.1:0".parse().unwrap(),
        store: None,
        group_label: Some("org".into()),
        dev,
        env_secrets: false,
        auth,
        slots: 8,
        memory_budget_mb: 256,
        tls: None,
    }
}

struct Client {
    base: String,
    key: Option<String>,
    http: reqwest::Client,
}

impl Client {
    fn new(served: &Served, key: Option<&str>) -> Self {
        Self {
            base: format!("http://{}", served.addr),
            key: key.map(str::to_owned),
            http: reqwest::Client::new(),
        }
    }

    async fn call(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> (u16, Value) {
        let mut request = self.http.request(method, format!("{}{path}", self.base));
        if let Some(key) = &self.key {
            request = request.bearer_auth(key);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .timeout(Duration::from_secs(60))
            .send()
            .await
            .unwrap();
        let status = response.status().as_u16();
        let text = response.text().await.unwrap();
        (
            status,
            serde_json::from_str(&text).unwrap_or(Value::String(text)),
        )
    }

    /// Retry while the install is still electing its shard.
    async fn ready(&self, method: reqwest::Method, path: &str, body: Value) -> Value {
        for _ in 0..120 {
            let (status, reply) = self.call(method.clone(), path, Some(body.clone())).await;
            if (200..300).contains(&status) {
                return reply;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        panic!("{method} {path} never succeeded");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pushed_agent_runs_with_a_pushed_key_and_the_stream_reports_it() {
    let (model_url, seen) = model_api().await;
    let data = tempfile::tempdir().unwrap();
    let served = start(
        config(data.path(), true, Auth::open()),
        Arc::new(everruns_agentd::StandardHost::new()),
    )
    .await
    .unwrap();
    let client = Client::new(&served, None);
    use reqwest::Method;

    // The model key, bound to the provider the agent names.
    client
        .ready(
            Method::PUT,
            "/v1/secrets/acme-key",
            json!({
                "value": "key-of-the-install",
                "bind": { "model": "Acme", "provider_type": "openai", "base_url": model_url, "api": "chat_completions" }
            }),
        )
        .await;
    let agent = json!({
        "name": "helper",
        "instructions": "Answer.",
        "model": { "provider": "Acme", "model": "acme-large" }
    });
    let pushed = client
        .ready(Method::PUT, "/v1/agents/helper", agent.clone())
        .await;
    assert_eq!(pushed["revision"], 1, "{pushed}");
    // The same definition again changes nothing.
    let again = client.ready(Method::PUT, "/v1/agents/helper", agent).await;
    assert_eq!(
        (again["revision"].clone(), again["changed"].clone()),
        (json!(1), json!(false))
    );
    let (_, listed) = client.call(Method::GET, "/v1/agents", None).await;
    assert_eq!(listed["data"][0]["id"], "helper", "{listed}");
    // A malformed one is refused at push.
    let (status, _) = client
        .call(Method::PUT, "/v1/agents/broken", Some(json!({ "name": 3 })))
        .await;
    assert_eq!(status, 400);

    // A session of group o1 by agent id; the group label is required.
    let (status, _) = client
        .call(
            Method::POST,
            "/v1/sessions",
            Some(json!({ "agent_id": "helper" })),
        )
        .await;
    assert_eq!(status, 400, "no group label");
    let created = client
        .ready(
            Method::POST,
            "/v1/sessions",
            json!({ "agent_id": "helper", "labels": { "org": "o1", "team": "support" } }),
        )
        .await;
    let session = created["id"].as_str().unwrap().to_owned();
    let (status, waited) = client
        .call(
            Method::POST,
            &format!("/v1/sessions/{session}/messages?wait=true"),
            Some(json!({ "text": "ping" })),
        )
        .await;
    assert_eq!(status, 200, "{waited}");
    assert_eq!(waited["status"], "completed", "{waited}");
    assert!(waited.to_string().contains("pong"), "{waited}");
    assert!(
        seen.lock()
            .iter()
            .all(|auth| auth == "Bearer key-of-the-install"),
        "{:?}",
        seen.lock()
    );

    // Listed by label, across shards; agentd names the pushed agent.
    let (_, mine) = client
        .call(Method::GET, "/v1/sessions?label=org=o1", None)
        .await;
    assert_eq!(mine["data"].as_array().unwrap().len(), 1, "{mine}");
    assert_eq!(mine["data"][0]["labels"]["team"], "support", "{mine}");
    assert_eq!(
        mine["data"][0]["labels"]["agentd.agent"], "helper",
        "{mine}"
    );
    let (_, other) = client
        .call(Method::GET, "/v1/sessions?label=org=o2", None)
        .await;
    assert!(other["data"].as_array().unwrap().is_empty(), "{other}");

    // The stream: the turn's usage and its session summary, with labels.
    let mut cursor = String::new();
    let mut kinds = Vec::new();
    let mut positions = std::collections::HashSet::new();
    let position = |r: &Value| {
        (
            r["shard"].to_string(),
            r["incarnation"].as_u64(),
            r["index"].as_u64(),
            r["ordinal"].as_u64(),
        )
    };
    for _ in 0..20 {
        let (status, page) = client
            .call(
                Method::GET,
                &format!("/v1/stream?label=org=o1&wait=5&after={cursor}"),
                None,
            )
            .await;
        assert_eq!(status, 200, "{page}");
        for record in page["data"].as_array().unwrap() {
            assert_eq!(record["labels"]["org"], "o1", "{record}");
            assert_eq!(record["group"], "o1", "{record}");
            kinds.push(record["type"].as_str().unwrap().to_owned());
            // A pointer to tapped events comes with the events it names.
            if record["type"] == "Events" {
                let events = record["events"]
                    .as_array()
                    .expect("events read from the session");
                assert!(!events.is_empty(), "{record}");
                kinds.extend(events.iter().map(|_| "Event".to_owned()));
            }
            assert!(positions.insert(position(record)), "read twice: {record}");
        }
        cursor = page["cursor"].as_str().unwrap().to_owned();
        // The archive arrives in batches: read until every kind is in.
        if ["Usage", "Session", "Event"]
            .iter()
            .all(|want| kinds.iter().any(|k| k == want))
        {
            break;
        }
    }
    assert!(kinds.iter().any(|k| k == "Usage"), "{kinds:?}");
    assert!(kinds.iter().any(|k| k == "Session"), "{kinds:?}");
    assert!(
        kinds.iter().any(|k| k == "Event"),
        "every session event: {kinds:?}"
    );
    // Read on from the cursor: whatever arrived since, nothing repeated.
    let (_, after) = client
        .call(
            Method::GET,
            &format!("/v1/stream?label=org=o1&after={cursor}"),
            None,
        )
        .await;
    for record in after["data"].as_array().unwrap() {
        assert!(!positions.contains(&position(record)), "repeated: {record}");
    }
    // A filter by the session event's own type.
    let (_, turns) = client
        .call(Method::GET, "/v1/stream?type=turn.completed", None)
        .await;
    let turns = turns["data"].as_array().unwrap();
    assert!(
        !turns.is_empty() && turns.iter().all(|r| r["event"]["type"] == "turn.completed"),
        "{turns:?}"
    );

    let (status, _) = client
        .call(
            Method::POST,
            "/v1/sessions",
            Some(json!({ "agent_id": "helper", "labels": { "org": "o1", "agentd.agent": "x" } })),
        )
        .await;
    assert_eq!(status, 400, "agentd. labels are agentd's");

    // Narrower keys: the agent's beats the install's, the session's beats
    // both. Provider material is cached for a short while (`providers.rs`
    // KEEP), per group, so each check runs in a group of its own.
    let bind = json!({ "model": "Acme", "provider_type": "openai", "base_url": model_url, "api": "chat_completions" });
    let scoped = client
        .ready(
            Method::PUT,
            "/v1/agents/helper/secrets/acme-key",
            json!({ "value": "key-of-the-agent", "bind": bind }),
        )
        .await;
    assert_eq!(scoped["scope"], "agent.helper", "{scoped}");
    let start = |group: &'static str| {
        let client = &client;
        async move {
            let created = client
                .ready(
                    Method::POST,
                    "/v1/sessions",
                    json!({ "agent_id": "helper", "labels": { "org": group } }),
                )
                .await;
            created["id"].as_str().unwrap().to_owned()
        }
    };
    let turn = |session: String| {
        let client = &client;
        async move {
            let (status, waited) = client
                .call(
                    Method::POST,
                    &format!("/v1/sessions/{session}/messages?wait=true"),
                    Some(json!({ "text": "ping" })),
                )
                .await;
            assert_eq!(
                (status, waited["status"].clone()),
                (200, json!("completed")),
                "{waited}"
            );
        }
    };
    turn(start("o3").await).await;
    assert_eq!(seen.lock().last().unwrap(), "Bearer key-of-the-agent");
    let own = start("o4").await;
    client
        .ready(
            Method::PUT,
            &format!("/v1/sessions/{own}/secrets/acme-key"),
            json!({ "value": "key-of-the-session", "bind": bind }),
        )
        .await;
    turn(own.clone()).await;
    assert_eq!(seen.lock().last().unwrap(), "Bearer key-of-the-session");
    // Deleting the narrower keys falls back to the install's.
    for path in [
        format!("/v1/sessions/{own}/secrets/acme-key"),
        "/v1/agents/helper/secrets/acme-key".to_owned(),
    ] {
        let (status, _) = client.call(Method::DELETE, &path, None).await;
        assert_eq!(status, 204, "{path}");
    }
    turn(start("o5").await).await;
    assert_eq!(seen.lock().last().unwrap(), "Bearer key-of-the-install");

    // The group: a spend limit of zero ends the next turn exhausted.
    let (status, group) = client
        .call(
            Method::PUT,
            "/v1/groups/o2",
            Some(json!({ "spend_usd": 0.0 })),
        )
        .await;
    assert_eq!(status, 200, "{group}");
    assert_eq!(group["spend_usd"], 0.0, "{group}");
    let broke = client
        .ready(
            Method::POST,
            "/v1/sessions",
            json!({ "agent_id": "helper", "labels": { "org": "o2" } }),
        )
        .await;
    let broke = broke["id"].as_str().unwrap().to_owned();
    let calls = seen.lock().len();
    let (_, waited) = client
        .call(
            Method::POST,
            &format!("/v1/sessions/{broke}/messages?wait=true"),
            Some(json!({ "text": "ping" })),
        )
        .await;
    assert_ne!(waited["status"], "completed", "{waited}");
    assert_eq!(seen.lock().len(), calls, "no model call without money");
    let (_, o1) = client.call(Method::GET, "/v1/groups/o1", None).await;
    assert_eq!(o1["spend_usd"], Value::Null, "{o1}");
    drop(served);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_install_takes_only_its_keys_and_refuses_to_start_without_any() {
    let data = tempfile::tempdir().unwrap();
    let refused = start(
        config(data.path(), false, Auth::default()),
        Arc::new(everruns_agentd::StandardHost::new()),
    )
    .await;
    assert!(refused.is_err(), "no credential, no install");
    let mut wide = config(data.path(), true, Auth::open());
    wide.listen = "0.0.0.0:0".parse().unwrap();
    assert!(
        start(wide, Arc::new(everruns_agentd::StandardHost::new()))
            .await
            .is_err(),
        "--dev only on loopback"
    );

    let auth = Auth::default()
        .with_keys("app:0123456789abcdef0123456789")
        .unwrap();
    let served = start(
        config(data.path(), false, auth),
        Arc::new(everruns_agentd::StandardHost::new()),
    )
    .await
    .unwrap();
    use reqwest::Method;
    let anonymous = Client::new(&served, None);
    let (status, _) = anonymous.call(Method::GET, "/v1/agents", None).await;
    assert_eq!(status, 401);
    let (status, _) = anonymous.call(Method::GET, "/health", None).await;
    assert_eq!(status, 200, "health stays open");
    let wrong = Client::new(&served, Some("not-the-key-at-all-not-at-all"));
    let (status, _) = wrong.call(Method::GET, "/v1/agents", None).await;
    assert_eq!(status, 401);
    let app = Client::new(&served, Some("0123456789abcdef0123456789"));
    let agent =
        json!({ "name": "a", "instructions": "x", "model": { "provider": "p", "model": "m" } });
    let pushed = app.ready(Method::PUT, "/v1/agents/a", agent).await;
    assert_eq!(pushed["revision"], 1, "{pushed}");
}
