//! End-to-end tests of the `/v1` wire API over a real socket, against an app
//! registered with serve's own macros. Offline: agents run the simulator.
//! One test drives the host through the official everruns Rust SDK, which
//! is the compatibility bar for the wire API.

use std::sync::Arc;
use std::time::Duration;

use everruns_sdk::sse::StreamOptions;
use everruns_sdk::{CreateSessionRequest, Everruns, SessionStatus};
use futures::StreamExt;
use serde_json::{Value, json};

use crate::app::Mode;
use crate::host::{Host, NewSession, Notice};
use crate::prelude::*;
use crate::sim;

/// The default test agent: a scripted conversation that loops
/// (1) call `shout`, reply; (2) call `guarded`, reply.
#[agent(default)]
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

/// Asks the person where to deploy, through the built-in `ask_user`.
#[agent]
fn asker() -> Agent {
    Agent::builder()
        .model("sim")
        .instructions("Ask before deploying.")
        .tools(Vec::<String>::new())
        .offline(sim::script([
            sim::call(
                "ask_user",
                json!({ "questions": [{
                    "header": "Target",
                    "question": "Where should I deploy?",
                    "options": [
                        { "label": "Staging", "description": "Safe", "default": true },
                        { "label": "Production", "description": "Live" }
                    ]
                }] }),
            ),
            sim::reply("deploying"),
        ]))
        .build()
}

/// Upper-case the text.
#[tool]
async fn shout(cx: &Cx, text: String) -> Result<String> {
    cx.progress("shouting").await;
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

fn sdk(server: &Server) -> Everruns {
    Everruns::builder()
        .api_key("serve-local")
        .base_url(server.base.clone())
        .build()
        .unwrap()
}

fn text_message(text: &str) -> Value {
    json!({ "message": { "role": "user", "content": [{ "type": "text", "text": text }] } })
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

    async fn get(&self, path: &str) -> reqwest::Response {
        self.client
            .get(format!("{}{path}", self.base))
            .send()
            .await
            .unwrap()
    }

    async fn session(&self, id: &str) -> Value {
        self.get(&format!("/v1/sessions/{id}"))
            .await
            .json()
            .await
            .unwrap()
    }

    async fn events(&self, id: &str, query: &str) -> Vec<Value> {
        let body: Value = self
            .get(&format!("/v1/sessions/{id}/events?{query}"))
            .await
            .json()
            .await
            .unwrap();
        body["data"].as_array().cloned().unwrap()
    }

    /// Poll the session until `check` holds.
    async fn wait_until(&self, id: &str, check: impl Fn(&Value) -> bool) -> Value {
        for _ in 0..400 {
            let session = self.session(id).await;
            if check(&session) {
                return session;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("session {id} never reached the expected state");
    }

    /// Poll the log until `count` terminal turn events are in it.
    async fn wait_turns(&self, id: &str, count: usize) -> Vec<Value> {
        for _ in 0..400 {
            let events = self.events(id, "").await;
            if events.iter().filter(|e| terminal(e)).count() >= count {
                return events;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("session {id} never finished {count} turn(s)");
    }
}

fn terminal(event: &Value) -> bool {
    matches!(
        event["type"].as_str(),
        Some("turn.completed" | "turn.failed" | "turn.cancelled")
    )
}

fn kinds(events: &[Value]) -> Vec<&str> {
    events.iter().filter_map(|e| e["type"].as_str()).collect()
}

/// The everruns SDK creates a session by agent name, sends messages, replays
/// the canonical events with `since_id`, follows a live turn through an
/// approval, and reads the session back.
#[tokio::test]
async fn the_everruns_sdk_drives_a_serve_app() {
    let host = Host::new(app(), Mode::Eval, None).unwrap();
    let server = serve(host).await;
    let client = sdk(&server);

    let session = client
        .sessions()
        .create_with_options(
            CreateSessionRequest::new()
                .agent_name("tester")
                .title("sdk test"),
        )
        .await
        .unwrap();
    assert_eq!(session.status, SessionStatus::Idle);
    assert!(session.id.starts_with("session_"), "{}", session.id);
    assert_eq!(session.agent_id.as_deref(), Some("tester"));
    assert_eq!(session.title.as_deref(), Some("sdk test"));

    let message = client.messages().create(&session.id, "go").await.unwrap();
    assert!(!message.id.is_empty());
    server.wait_turns(&session.id, 1).await;
    let listed = client.events().list(&session.id).await.unwrap().data;
    let types: Vec<&str> = listed.iter().map(|e| e.event_type.as_str()).collect();
    assert!(types.contains(&"input.message"), "{types:?}");
    assert!(types.contains(&"tool.progress"), "{types:?}");
    assert!(listed.iter().all(|e| e.session_id == session.id));

    // Resume after the third event: exactly the rest, in order.
    let mut resumed = client.events().stream_with_options(
        &session.id,
        StreamOptions::exclude_deltas().with_since_id(listed[2].id.clone()),
    );
    let mut replayed = Vec::new();
    while replayed.len() < listed.len() - 3 {
        let event = tokio::time::timeout(Duration::from_secs(10), resumed.next())
            .await
            .expect("replay arrives")
            .unwrap()
            .unwrap();
        replayed.push(event.id);
    }
    let expected: Vec<String> = listed[3..].iter().map(|e| e.id.clone()).collect();
    assert_eq!(replayed, expected);
    resumed.stop();

    // Live: the SDK stream starts at "now" (no cursor), like on the server.
    // It connects on first poll, so poll it on its own task first.
    let (tx, mut live) = tokio::sync::mpsc::unbounded_channel();
    let mut stream = client
        .events()
        .stream_with_options(&session.id, StreamOptions::exclude_deltas());
    let follower = tokio::spawn(async move {
        while let Some(Ok(event)) = stream.next().await {
            if tx.send(event).is_err() {
                return;
            }
        }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    client
        .messages()
        .create(&session.id, "second")
        .await
        .unwrap();
    let waiting = server
        .wait_until(&session.id, |s| s["status"] == "waitingfortoolresults")
        .await;
    assert_eq!(
        client.sessions().get(&session.id).await.unwrap().status,
        SessionStatus::WaitingForToolResults
    );
    let call = waiting["pending_approvals"][0]["tool_call_id"]
        .as_str()
        .unwrap()
        .to_string();
    let approved = server
        .post(
            &format!("/v1/sessions/{}/approvals/{call}", session.id),
            json!({ "decision": "approve" }),
        )
        .await;
    assert_eq!(approved.status(), 200);
    let mut seen = Vec::new();
    loop {
        let event = tokio::time::timeout(Duration::from_secs(10), live.recv())
            .await
            .expect("live events arrive")
            .unwrap();
        let done = event.event_type == "turn.completed";
        seen.push(event);
        if done {
            break;
        }
    }
    follower.abort();
    let first_new = listed.last().unwrap().sequence.unwrap() + 1;
    assert_eq!(seen[0].sequence, Some(first_new), "no gap before live");
    assert!(
        seen.iter()
            .any(|e| e.event_type == "tool.completed" && e.data["tool_name"] == "guarded")
    );

    let fetched = client.sessions().get(&session.id).await.unwrap();
    assert_eq!(fetched.status, SessionStatus::Idle);
    // Cancel with nothing running is a no-op the SDK accepts.
    client.sessions().cancel(&session.id).await.unwrap();
}

#[tokio::test]
async fn sse_replays_from_a_sequence_with_the_server_framing() {
    let host = Host::new(app(), Mode::Eval, None).unwrap();
    let server = serve(host).await;
    let created = server.post("/v1/sessions", json!({})).await;
    assert_eq!(created.status(), 201);
    let location = created.headers()["location"].to_str().unwrap().to_string();
    let session: Value = created.json().await.unwrap();
    let id = session["id"].as_str().unwrap().to_string();
    assert_eq!(location, format!("/v1/sessions/{id}"));
    assert_eq!(session["agent_name"], "tester");
    assert_eq!(session["status"], "idle");
    assert!(session["build_id"].is_string());

    let message = server
        .post(&format!("/v1/sessions/{id}/messages"), text_message("go"))
        .await;
    assert_eq!(message.status(), 201);
    let message: Value = message.json().await.unwrap();
    assert_eq!(message["role"], "user");
    assert_eq!(message["session_id"], id.as_str());
    assert_eq!(message["content"][0]["text"], "go");
    let events = server.wait_turns(&id, 1).await;
    if let Some(sequence) = message["sequence"].as_i64() {
        let input = events
            .iter()
            .find(|e| e["type"] == "input.message")
            .unwrap();
        assert_eq!(input["sequence"], sequence);
    }

    // after_sequence=0 replays everything; the stream is SSE with ids on
    // durable events only.
    let response = server
        .client
        .get(format!(
            "{}/v1/sessions/{id}/sse?after_sequence=0&exclude=output.message.delta",
            server.base
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let mut body = response.bytes_stream();
    let mut text = String::new();
    while text.matches("event: turn.completed").count() == 0 {
        let chunk = tokio::time::timeout(Duration::from_secs(10), body.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        text.push_str(&String::from_utf8_lossy(&chunk));
    }
    let blocks: Vec<&str> = text.split("\n\n").filter(|b| !b.is_empty()).collect();
    assert!(blocks[0].contains("event: connected"), "{}", blocks[0]);
    assert!(blocks[0].contains(r#"data: {"status":"connected"}"#));
    assert!(blocks[0].contains("retry: "));
    let first = blocks[1];
    assert!(first.contains(&format!("id: {}", events[0]["id"].as_str().unwrap())));
    let data: Value = serde_json::from_str(
        first
            .lines()
            .find_map(|line| line.strip_prefix("data: "))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(data, events[0]);

    // Both cursors at once is the server's 400.
    let both = server
        .get(&format!(
            "/v1/sessions/{id}/sse?after_sequence=0&since_id=event_0"
        ))
        .await;
    assert_eq!(both.status(), 400);
    assert_eq!(
        both.headers()["content-type"],
        "application/problem+json",
        "errors are problem details"
    );
}

#[tokio::test]
async fn the_events_list_pages_and_filters() {
    let host = Host::new(app(), Mode::Eval, None).unwrap();
    let server = serve(host).await;
    let session: Value = server
        .post("/v1/sessions", json!({}))
        .await
        .json()
        .await
        .unwrap();
    let id = session["id"].as_str().unwrap();
    server
        .post(&format!("/v1/sessions/{id}/messages"), text_message("go"))
        .await;
    let all = server.wait_turns(id, 1).await;
    let sequences: Vec<i64> = all
        .iter()
        .map(|e| e["sequence"].as_i64().unwrap())
        .collect();
    assert_eq!(sequences, (1..=all.len() as i64).collect::<Vec<_>>());

    let last_two = server
        .get(&format!("/v1/sessions/{id}/events?limit=2"))
        .await;
    assert_eq!(
        last_two.headers()["x-total-count"].to_str().unwrap(),
        all.len().to_string()
    );
    let last_two: Value = last_two.json().await.unwrap();
    assert_eq!(last_two["data"].as_array().unwrap(), &all[all.len() - 2..]);

    let tail = server.events(id, "after_sequence=3").await;
    assert_eq!(tail, all[3..]);
    let head = server.events(id, "before_sequence=3").await;
    assert_eq!(head, all[..2]);
    let since = server
        .events(id, &format!("since_id={}", all[1]["id"].as_str().unwrap()))
        .await;
    assert_eq!(since, all[2..]);
    let tools = server
        .events(id, "types=tool.started&types=tool.completed")
        .await;
    assert_eq!(kinds(&tools), ["tool.started", "tool.completed"]);
    assert_eq!(tools[0]["data"]["tool_call"]["name"], "shout");
    assert_eq!(
        tools[0]["data"]["tool_call"]["arguments"],
        json!({ "text": "hi" })
    );
}

#[tokio::test]
async fn an_approval_gates_the_tool_until_a_person_decides() {
    let host = Host::new(app(), Mode::Eval, None).unwrap();
    let server = serve(host).await;
    let session: Value = server
        .post("/v1/sessions", json!({}))
        .await
        .json()
        .await
        .unwrap();
    let id = session["id"].as_str().unwrap().to_string();
    server
        .post(
            &format!("/v1/sessions/{id}/messages"),
            text_message("first"),
        )
        .await;
    server.wait_turns(&id, 1).await;

    server
        .post(
            &format!("/v1/sessions/{id}/messages"),
            text_message("second"),
        )
        .await;
    let waiting = server
        .wait_until(&id, |s| s["status"] == "waitingfortoolresults")
        .await;
    let pending = &waiting["pending_approvals"][0];
    assert_eq!(pending["tool_name"], "guarded");
    let call = pending["tool_call_id"].as_str().unwrap().to_string();
    let before = server.events(&id, "").await;
    assert!(
        !before
            .iter()
            .any(|e| e["type"] == "tool.completed" && e["data"]["tool_name"] == "guarded"),
        "ran before approval"
    );

    let unknown = server
        .post(
            &format!("/v1/sessions/{id}/approvals/call_nope"),
            json!({ "decision": "approve" }),
        )
        .await;
    assert_eq!(unknown.status(), 404);
    let resolved = server
        .post(
            &format!("/v1/sessions/{id}/approvals/{call}"),
            json!({ "decision": "approve" }),
        )
        .await;
    assert_eq!(resolved.status(), 200);

    let events = server.wait_turns(&id, 2).await;
    let completed = events
        .iter()
        .rev()
        .find(|e| e["type"] == "tool.completed")
        .unwrap();
    assert_eq!(completed["data"]["tool_name"], "guarded");
    assert_eq!(completed["data"]["success"], true);
    assert_eq!(server.session(&id).await["status"], "idle");
}

#[tokio::test]
async fn a_denied_call_reaches_the_model_as_a_tool_error() {
    let host = Host::new(app(), Mode::Eval, None).unwrap();
    let id = host.create_session(NewSession::default()).await.unwrap();
    host.send(&id, "first".into())
        .await
        .unwrap()
        .wait()
        .await
        .unwrap();
    let mut notices = host.notices.subscribe();
    let turn = host.send(&id, "second".into()).await.unwrap();
    let call = loop {
        if let Notice::ApprovalRequested(view) = notices.recv().await.unwrap() {
            break view.tool_call_id;
        }
    };
    host.resolve_approval(&id, &call, false).unwrap();
    assert!(turn.wait().await.unwrap().success);
    let events = host.events_after(&id, 0).await.unwrap();
    let completed = events
        .iter()
        .rev()
        .find(|e| e.event_type() == "tool.completed")
        .unwrap();
    let data = completed.canonical_json()["data"].to_string();
    assert!(data.contains("rejected"), "{data}");
    assert!(!data.contains("\"ran\""), "{data}");
}

#[tokio::test]
async fn ask_user_is_answered_through_question_answers() {
    let host = Host::new(app(), Mode::Eval, None).unwrap();
    let server = serve(host).await;
    let session: Value = server
        .post("/v1/sessions", json!({ "agent_name": "asker" }))
        .await
        .json()
        .await
        .unwrap();
    let id = session["id"].as_str().unwrap().to_string();

    let nothing = server
        .post(
            &format!("/v1/sessions/{id}/question-answers"),
            json!({ "answers": [] }),
        )
        .await;
    assert_eq!(nothing.status(), 404);

    server
        .post(
            &format!("/v1/sessions/{id}/messages"),
            text_message("deploy"),
        )
        .await;
    let waiting = server
        .wait_until(&id, |s| s["status"] == "waitingfortoolresults")
        .await;
    let pending = &waiting["pending_questions"][0];
    let call = pending["tool_call_id"].as_str().unwrap().to_string();
    let question = pending["questions"][0]["id"].as_str().unwrap().to_string();

    let mismatched = server
        .post(
            &format!("/v1/sessions/{id}/question-answers"),
            json!({ "tool_call_id": call, "answers": [{ "id": question, "selected": ["Moon"] }] }),
        )
        .await;
    assert_eq!(mismatched.status(), 400);

    let answered = server
        .post(
            &format!("/v1/sessions/{id}/question-answers"),
            json!({ "tool_call_id": call, "answers": [{ "id": question, "selected": ["Production"] }] }),
        )
        .await;
    assert_eq!(answered.status(), 200);
    let answered: Value = answered.json().await.unwrap();
    assert_eq!(answered["status"], "answered");
    assert_eq!(answered["answered_by"], "user");

    let again = server
        .post(
            &format!("/v1/sessions/{id}/question-answers"),
            json!({ "tool_call_id": call, "answers": [{ "id": question, "selected": ["Staging"] }] }),
        )
        .await;
    assert_eq!(again.status(), 409);

    let events = server.wait_turns(&id, 1).await;
    let completed = events
        .iter()
        .find(|e| e["type"] == "tool.completed")
        .unwrap();
    assert_eq!(completed["data"]["tool_name"], "ask_user");
    assert!(
        completed["data"].to_string().contains("Production"),
        "{}",
        completed["data"]
    );
}

#[tokio::test]
async fn cancel_stops_a_turn_waiting_for_approval() {
    let host = Host::new(app(), Mode::Eval, None).unwrap();
    let server = serve(host).await;
    let session: Value = server
        .post("/v1/sessions", json!({}))
        .await
        .json()
        .await
        .unwrap();
    let id = session["id"].as_str().unwrap().to_string();
    server
        .post(
            &format!("/v1/sessions/{id}/messages"),
            text_message("first"),
        )
        .await;
    server.wait_turns(&id, 1).await;
    server
        .post(
            &format!("/v1/sessions/{id}/messages"),
            text_message("second"),
        )
        .await;
    server
        .wait_until(&id, |s| s["status"] == "waitingfortoolresults")
        .await;

    let cancelled: Value = server
        .post(&format!("/v1/sessions/{id}/cancel"), json!({}))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(cancelled["status"], "cancelled");
    let events = server.wait_turns(&id, 2).await;
    assert_eq!(kinds(&events).last(), Some(&"turn.cancelled"));
    let after = server.wait_until(&id, |s| s["status"] == "idle").await;
    assert_eq!(after["pending_approvals"], json!([]));

    let no_op: Value = server
        .post(&format!("/v1/sessions/{id}/cancel"), json!({}))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(no_op["status"], "no_op");
}

#[tokio::test]
async fn bad_requests_and_unknown_things_are_rejected() {
    let host = Host::new(app(), Mode::Eval, None).unwrap();
    let server = serve(host).await;
    let missing = server.get("/v1/sessions/session_nope/sse").await;
    assert_eq!(missing.status(), 404);
    let unknown_agent = server
        .post("/v1/sessions", json!({ "agent_name": "nobody" }))
        .await;
    assert_eq!(unknown_agent.status(), 404);
    let channel = server.post("/v1/channels/nope", json!({})).await;
    assert_eq!(channel.status(), 404);

    let session: Value = server
        .post("/v1/sessions", json!({ "unknown_field": true }))
        .await
        .json()
        .await
        .unwrap();
    let id = session["id"].as_str().unwrap();
    let image = server
        .post(
            &format!("/v1/sessions/{id}/messages"),
            json!({ "message": { "content": [{ "type": "image", "url": "https://x" }] } }),
        )
        .await;
    assert_eq!(image.status(), 400);
    let problem: Value = image.json().await.unwrap();
    assert_eq!(problem["status"], 400);
    assert!(problem["detail"].as_str().unwrap().contains("text"));
}

#[tokio::test]
async fn a_channel_thread_maps_to_one_session_and_gets_the_reply() {
    let host = Host::new(app(), Mode::Eval, None).unwrap();
    let mut notices = host.notices.subscribe();
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
    let delivered = loop {
        if let Notice::Delivered {
            session_id,
            to,
            error,
        } = notices.recv().await.unwrap()
        {
            assert!(error.is_none(), "{error:?}");
            break (session_id, to);
        }
    };
    assert_eq!(delivered, (id.clone(), "hook:log:t1".to_string()));

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
async fn a_session_and_its_log_survive_a_restart_of_the_host() {
    let dir = tempfile::tempdir().unwrap();
    let (id, before) = {
        let host = Host::new(app(), Mode::Dev, Some(dir.path().to_path_buf())).unwrap();
        let id = host.create_session(NewSession::default()).await.unwrap();
        let turn = host.send(&id, "before".into()).await.unwrap().wait().await;
        assert!(turn.unwrap().success);
        let before = host.events_after(&id, 0).await.unwrap().len();
        (id, before)
    };
    // A new process: same data dir, nothing in memory.
    let host = Host::new(app(), Mode::Dev, Some(dir.path().to_path_buf())).unwrap();
    let server = serve(host.clone()).await;
    let replay = server.events(&id, "").await;
    assert_eq!(replay.len(), before, "the engine's log is the wire log");
    let turn = host.send(&id, "after".into()).await.unwrap().wait().await;
    assert!(turn.unwrap().success);
    let tail = server
        .events(&id, &format!("after_sequence={before}"))
        .await;
    assert_eq!(
        tail[0]["sequence"],
        before as i64 + 1,
        "dense across restarts"
    );
    assert_eq!(tail[0]["type"], "input.message");
}

#[tokio::test]
async fn production_refuses_a_session_pinned_to_another_build() {
    let dir = tempfile::tempdir().unwrap();
    let id = {
        let host = Host::new(app(), Mode::Dev, Some(dir.path().to_path_buf())).unwrap();
        host.create_session(NewSession::default()).await.unwrap()
    };
    let store = crate::store::Store::open(&dir.path().join("serve.db")).unwrap();
    store.repin_for_test(&id, "b-older");
    drop(store);
    let host = Host::new(app(), Mode::Start, Some(dir.path().to_path_buf())).unwrap();
    let server = serve(host).await;
    let response = server
        .post(&format!("/v1/sessions/{id}/messages"), text_message("hi"))
        .await;
    assert_eq!(response.status(), 409);
    assert_eq!(response.headers()["x-serve-build"], "b-older");
    // Reading the catalog row does not need the build.
    assert_eq!(server.session(&id).await["build_id"], "b-older");
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
    assert!(
        manifest
            .routes
            .contains(&"GET /v1/sessions/{id}/sse".to_string())
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
    t.completed()
        .unwrap()
        .called_tool("guarded")
        .unwrap()
        .asked_approval("guarded")
        .unwrap();
}

#[tokio::test]
async fn remote_evals_drive_the_wire_api() {
    let host = Host::new(app(), Mode::Eval, None).unwrap();
    let server = serve(host).await;
    let mut t = crate::eval::EvalCx::remote_for_test(server.base.clone());
    t.send("one").await.unwrap();
    t.completed()
        .unwrap()
        .called_tool("shout")
        .unwrap()
        .reply_contains("shouted")
        .unwrap();
    t.send("two").await.unwrap();
    t.completed().unwrap().asked_approval("guarded").unwrap();

    let mut asked = crate::eval::EvalCx::remote_for_test(server.base.clone());
    asked.agent("asker");
    asked.send("deploy").await.unwrap();
    assert_eq!(asked.last().unwrap().questions, 1);
    asked.completed().unwrap().called_tool("ask_user").unwrap();
}

/// Simulated tool call ids repeat across sessions; pending approvals are
/// keyed by session too, so answering one leaves the other waiting.
#[tokio::test]
async fn approvals_in_two_sessions_do_not_collide() {
    let host = Host::new(app(), Mode::Eval, None).unwrap();
    let server = serve(host).await;
    let mut ids = Vec::new();
    for _ in 0..2 {
        let session: Value = server
            .post("/v1/sessions", json!({}))
            .await
            .json()
            .await
            .unwrap();
        let id = session["id"].as_str().unwrap().to_string();
        server
            .post(
                &format!("/v1/sessions/{id}/messages"),
                text_message("first"),
            )
            .await;
        server.wait_turns(&id, 1).await;
        server
            .post(
                &format!("/v1/sessions/{id}/messages"),
                text_message("second"),
            )
            .await;
        server
            .wait_until(&id, |s| s["status"] == "waitingfortoolresults")
            .await;
        ids.push(id);
    }
    let call = |session: &Value| session["pending_approvals"][0]["tool_call_id"].clone();
    let (a, b) = (server.session(&ids[0]).await, server.session(&ids[1]).await);
    let approved = server
        .post(
            &format!(
                "/v1/sessions/{}/approvals/{}",
                ids[0],
                call(&a).as_str().unwrap()
            ),
            json!({ "decision": "approve" }),
        )
        .await;
    assert_eq!(approved.status(), 200);
    server.wait_turns(&ids[0], 2).await;
    let still = server.session(&ids[1]).await;
    assert_eq!(still["status"], "waitingfortoolresults");
    assert_eq!(call(&still), call(&b));
}
