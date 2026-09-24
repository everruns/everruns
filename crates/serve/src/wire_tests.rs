//! End-to-end tests of the `/v1` wire API over a real socket, against an app
//! registered with serve's own macros. Offline: the agent runs the simulator.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use crate::app::Mode;
use crate::host::Host;
use crate::prelude::*;
use crate::sim;

/// The test agent: a scripted conversation that loops
/// (1) call `shout`, reply; (2) call `guarded`, reply.
#[agent]
fn tester() -> Agent {
    Agent::builder()
        .model("sim")
        .instructions("Test agent.")
        .offline(sim::script([
            sim::call("shout", json!({ "text": "hi" })),
            sim::reply("shouted"),
            sim::call("guarded", json!({})),
            sim::reply("guarded done"),
        ]))
        .build()
}

/// Upper-case the text.
#[tool]
async fn shout(cx: &Cx, text: String) -> Result<String> {
    cx.progress("shouting");
    Ok(text.to_uppercase())
}

/// A tool that always needs a person's approval.
#[tool(needs_approval)]
async fn guarded() -> Result<&'static str> {
    Ok("ran")
}

/// Replies are logged, not posted.
#[channel]
fn hook() -> Webhook {
    Webhook::new()
}

struct Server {
    base: String,
    client: reqwest::Client,
    _task: tokio::task::JoinHandle<()>,
}

async fn serve(host: Arc<Host>) -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, crate::server::router(host))
            .await
            .unwrap();
    });
    Server {
        base: format!("http://{addr}"),
        client: reqwest::Client::new(),
        _task: task,
    }
}

fn app() -> App {
    let app = App::builder().discover().build();
    assert!(app.errors().is_empty(), "{:?}", app.errors());
    app
}

impl Server {
    async fn post(&self, path: &str, body: Value) -> reqwest::Response {
        self.client
            .post(format!("{}{path}", self.base))
            .json(&body)
            .send()
            .await
            .unwrap()
    }

    /// Replay the log (no follow) and parse the SSE blocks.
    async fn events(&self, id: &str, cursor: Option<i64>) -> Vec<Value> {
        let mut request = self.client.get(format!(
            "{}/v1/sessions/{id}/events?follow=false",
            self.base
        ));
        if let Some(cursor) = cursor {
            request = request.header("last-event-id", cursor.to_string());
        }
        let text = request.send().await.unwrap().text().await.unwrap();
        text.split("\n\n")
            .filter_map(|block| {
                let data = block.lines().find_map(|line| line.strip_prefix("data: "))?;
                serde_json::from_str(data).ok()
            })
            .collect()
    }

    /// Poll until an event of `kind` appears after `cursor`.
    async fn wait_for(&self, id: &str, kind: &str, cursor: i64) -> Vec<Value> {
        for _ in 0..200 {
            let events = self.events(id, Some(cursor)).await;
            if events.iter().any(|event| event["type"] == kind) {
                return events;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("no {kind} event on {id}");
    }
}

fn kinds(events: &[Value]) -> Vec<&str> {
    events.iter().filter_map(|e| e["type"].as_str()).collect()
}

#[tokio::test]
async fn a_session_runs_a_turn_and_its_log_resumes_from_a_cursor() {
    let host = Host::new(app(), Mode::Eval, None).unwrap();
    let server = serve(host).await;

    let response = server.post("/v1/sessions", json!({ "input": "go" })).await;
    assert_eq!(response.status(), 201);
    let location = response.headers()["location"].to_str().unwrap().to_string();
    let body: Value = response.json().await.unwrap();
    let id = body["id"].as_str().unwrap();
    assert_eq!(location, format!("/v1/sessions/{id}"));
    assert_eq!(body["agent"], "tester");

    let events = server.wait_for(id, "turn.result", 0).await;
    let kinds = kinds(&events);
    assert_eq!(kinds[0], "session.created");
    assert!(kinds.contains(&"tool.progress"), "{kinds:?}");
    let started = events.iter().find(|e| e["type"] == "tool.started").unwrap();
    assert_eq!(started["data"]["tool_name"], "shout");
    assert_eq!(started["data"]["arguments"], json!({ "text": "hi" }));
    let result = events.iter().find(|e| e["type"] == "turn.result").unwrap();
    assert_eq!(result["data"]["response"], "shouted");

    // Sequence numbers are dense, and a cursor replays strictly after it.
    let seqs: Vec<i64> = events.iter().map(|e| e["seq"].as_i64().unwrap()).collect();
    assert_eq!(seqs, (1..=seqs.len() as i64).collect::<Vec<_>>());
    let tail = server.events(id, Some(3)).await;
    assert_eq!(tail[0]["seq"], 4);
    assert_eq!(tail.len(), events.len() - 3);
}

#[tokio::test]
async fn an_approval_gates_the_tool_until_a_person_decides() {
    let host = Host::new(app(), Mode::Eval, None).unwrap();
    let server = serve(host).await;
    let body: Value = server
        .post("/v1/sessions", json!({ "input": "first" }))
        .await
        .json()
        .await
        .unwrap();
    let id = body["id"].as_str().unwrap().to_string();
    let first = server.wait_for(&id, "turn.result", 0).await;
    let cursor = first.last().unwrap()["seq"].as_i64().unwrap();

    let accepted = server
        .post(
            &format!("/v1/sessions/{id}/messages"),
            json!({ "input": "second" }),
        )
        .await;
    assert_eq!(accepted.status(), 202);
    let pending = server.wait_for(&id, "approval.requested", cursor).await;
    assert!(
        !kinds(&pending).contains(&"tool.completed"),
        "ran before approval"
    );
    let approval = pending
        .iter()
        .find(|e| e["type"] == "approval.requested")
        .unwrap()["data"]["approval_id"]
        .as_str()
        .unwrap()
        .to_string();

    let unknown = server
        .post(
            &format!("/v1/sessions/{id}/approvals/apr_nope"),
            json!({ "decision": "approve" }),
        )
        .await;
    assert_eq!(unknown.status(), 404);
    let resolved = server
        .post(
            &format!("/v1/sessions/{id}/approvals/{approval}"),
            json!({ "decision": "approve" }),
        )
        .await;
    assert_eq!(resolved.status(), 200);

    let done = server.wait_for(&id, "turn.result", cursor).await;
    let completed = done.iter().find(|e| e["type"] == "tool.completed").unwrap();
    assert_eq!(completed["data"]["tool_name"], "guarded");
    assert_eq!(completed["data"]["success"], true);
}

#[tokio::test]
async fn a_denied_call_reports_the_decision_to_the_model() {
    let host = Host::new(app(), Mode::Eval, None).unwrap();
    let id = host.create_session(None, Value::Null, None).await.unwrap();
    host.send(&id, "first".into())
        .await
        .unwrap()
        .wait()
        .await
        .unwrap();
    let mut live = host.events.subscribe();
    let turn = host.send(&id, "second".into()).await.unwrap();
    let approval = loop {
        let event = live.recv().await.unwrap();
        if event.kind == "approval.requested" {
            break event.data["approval_id"].as_str().unwrap().to_string();
        }
    };
    assert!(
        host.resolve_approval(&id, &approval, false, Some("too big".into()))
            .unwrap()
    );
    assert!(turn.wait().await.unwrap().success);
    // The turn future can resolve before the event pump has logged the turn.
    let mut events = host.events_after(&id, 0).unwrap();
    for _ in 0..200 {
        if events.iter().filter(|e| e.kind == "turn.completed").count() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        events = host.events_after(&id, 0).unwrap();
    }
    let completed = events
        .iter()
        .rev()
        .find(|e| e.kind == "tool.completed")
        .unwrap();
    let result = completed.data.to_string();
    assert!(
        result.contains("declined") && result.contains("too big"),
        "{result}"
    );
}

#[tokio::test]
async fn unknown_sessions_and_channels_are_404() {
    let host = Host::new(app(), Mode::Eval, None).unwrap();
    let server = serve(host).await;
    let missing = server
        .client
        .get(format!("{}/v1/sessions/session_nope/events", server.base))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);
    let channel = server.post("/v1/channels/nope", json!({})).await;
    assert_eq!(channel.status(), 404);
}

#[tokio::test]
async fn a_channel_thread_maps_to_one_session_and_gets_the_reply() {
    let host = Host::new(app(), Mode::Eval, None).unwrap();
    let server = serve(host).await;
    let first: Value = server
        .post(
            "/v1/channels/hook",
            json!({ "thread": "t1", "text": "hello" }),
        )
        .await
        .json()
        .await
        .unwrap();
    let id = first["session_id"].as_str().unwrap().to_string();
    server.wait_for(&id, "delivery.completed", 0).await;

    let again: Value = server
        .post(
            "/v1/channels/hook",
            json!({ "thread": "t1", "text": "more" }),
        )
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(
        again["session_id"],
        id.as_str(),
        "same thread, same session"
    );
    let bad = server
        .post("/v1/channels/hook", json!({ "thread": "t1" }))
        .await;
    assert_eq!(bad.status(), 400);
}

#[tokio::test]
async fn a_session_survives_a_restart_of_the_host() {
    let dir = tempfile::tempdir().unwrap();
    let id = {
        let host = Host::new(app(), Mode::Dev, Some(dir.path().to_path_buf())).unwrap();
        let id = host.create_session(None, Value::Null, None).await.unwrap();
        assert!(
            host.send(&id, "before".into())
                .await
                .unwrap()
                .wait()
                .await
                .unwrap()
                .success
        );
        id
    };
    // A new process: same data dir, nothing in memory.
    let host = Host::new(app(), Mode::Dev, Some(dir.path().to_path_buf())).unwrap();
    assert!(
        host.send(&id, "after".into())
            .await
            .unwrap()
            .wait()
            .await
            .unwrap()
            .success
    );
    let kinds: Vec<String> = host
        .events_after(&id, 0)
        .unwrap()
        .into_iter()
        .map(|e| e.kind)
        .collect();
    assert!(kinds.contains(&"session.resumed".to_string()), "{kinds:?}");
}

#[tokio::test]
async fn production_refuses_a_session_pinned_to_another_build() {
    let dir = tempfile::tempdir().unwrap();
    let id = {
        let host = Host::new(app(), Mode::Dev, Some(dir.path().to_path_buf())).unwrap();
        host.create_session(None, Value::Null, None).await.unwrap()
    };
    let store = crate::store::Store::open(&dir.path().join("serve.db")).unwrap();
    store.repin_for_test(&id, "b-older");
    drop(store);
    let host = Host::new(app(), Mode::Start, Some(dir.path().to_path_buf())).unwrap();
    let server = serve(host).await;
    let response = server
        .post(
            &format!("/v1/sessions/{id}/messages"),
            json!({ "input": "hi" }),
        )
        .await;
    assert_eq!(response.status(), 409);
    assert_eq!(response.headers()["x-serve-build"], "b-older");
}

#[tokio::test]
async fn the_manifest_lists_what_the_host_must_provide() {
    let manifest = app().manifest();
    assert_eq!(manifest.schema, crate::manifest::SCHEMA);
    assert!(manifest.experimental);
    let guarded = manifest.tools.iter().find(|t| t.name == "guarded").unwrap();
    assert_eq!(guarded.needs_approval, "always");
    let shout = manifest.tools.iter().find(|t| t.name == "shout").unwrap();
    assert_eq!(shout.parameters["required"], json!(["text"]));
    assert!(
        manifest
            .routes
            .contains(&"POST /v1/channels/hook".to_string())
    );
    assert_eq!(manifest.models, vec!["sim"]);
    assert_eq!(
        manifest.build_id,
        app().manifest().build_id,
        "deterministic"
    );
}

#[tokio::test]
async fn in_process_evals_see_tools_and_approvals() {
    let host = Host::new(app(), Mode::Eval, None).unwrap();
    let mut t = crate::eval::EvalCx::local_for_test(host);
    t.send("one").await.unwrap();
    t.completed()
        .unwrap()
        .called_tool("shout")
        .unwrap()
        .reply_contains("SHOUTED")
        .unwrap();
    t.send("two").await.unwrap();
    t.completed().unwrap().asked_approval("guarded").unwrap();
}
