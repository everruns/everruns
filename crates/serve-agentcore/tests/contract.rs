//! The AgentCore Runtime contract over a real socket, against an app
//! registered with serve's macros. Offline: the agent runs the simulator.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use serde_json::{Value, json};
use serve::prelude::*;
use serve::{Mode, sim};
use serve_agentcore::{Options, SESSION_HEADER, router};

/// Calls the slow tool, then replies.
#[agent(default)]
fn assistant() -> Agent {
    Agent::builder()
        .model("sim")
        .instructions("Test agent.")
        .offline(sim::script([
            sim::call("slow", json!({})),
            sim::reply("pong"),
        ]))
        .build()
}

/// Takes long enough for a health check to see the turn running.
#[tool]
async fn slow() -> Result<&'static str> {
    tokio::time::sleep(Duration::from_millis(600)).await;
    Ok("done")
}

struct Booted {
    base: String,
    _dir: tempfile::TempDir,
}

async fn boot_in(dir: &std::path::Path) -> String {
    let app = App::builder().discover().build();
    assert!(app.errors().is_empty(), "{:?}", app.errors());
    let mut options = Options::new(Mode::Eval);
    options.data_dir = Some(dir.join("data"));
    options.workspace = Some(dir.join("workspace"));
    let (app, agent) = router(app, options).unwrap();
    assert_eq!(agent, "assistant");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    base
}

async fn boot() -> Booted {
    let dir = tempfile::tempdir().unwrap();
    let base = boot_in(dir.path()).await;
    Booted { base, _dir: dir }
}

async fn ping(client: &reqwest::Client, base: &str) -> String {
    let body: Value = client
        .get(format!("{base}/ping"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    body["status"].as_str().unwrap().to_string()
}

/// The `data:` payloads of an SSE body.
fn events(body: &str) -> Vec<Value> {
    body.lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter_map(|data| serde_json::from_str(data).ok())
        .collect()
}

#[tokio::test]
async fn prompt_invocation_streams_ag_ui_and_ping_tracks_the_turn() {
    let booted = boot().await;
    let base = booted.base.clone();
    let client = reqwest::Client::new();
    assert_eq!(ping(&client, &base).await, "Healthy");

    let session = "agentcore-session-0123456789abcdef0123456789";
    let request = client
        .post(format!("{base}/invocations"))
        .header(SESSION_HEADER, session)
        .json(&json!({ "prompt": "hi" }))
        .send();
    let run = tokio::spawn(async move {
        let response = request.await.unwrap();
        assert_eq!(response.status(), 200);
        let content_type = response.headers()["content-type"].to_str().unwrap();
        assert!(
            content_type.starts_with("text/event-stream"),
            "{content_type}"
        );
        response.text().await.unwrap()
    });

    // The slow tool keeps the turn running long enough to be seen.
    let mut saw_busy = false;
    for _ in 0..50 {
        if ping(&client, &base).await == "HealthyBusy" {
            saw_busy = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(saw_busy, "/ping never reported HealthyBusy during the turn");

    let events = events(&run.await.unwrap());
    let kinds: Vec<&str> = events.iter().filter_map(|e| e["type"].as_str()).collect();
    assert_eq!(kinds.first(), Some(&"RUN_STARTED"), "{kinds:?}");
    assert_eq!(kinds.last(), Some(&"RUN_FINISHED"), "{kinds:?}");
    assert_eq!(events[0]["threadId"], session);
    let text: String = events
        .iter()
        .filter(|e| e["type"] == "TEXT_MESSAGE_CONTENT")
        .filter_map(|e| e["delta"].as_str())
        .collect();
    assert_eq!(text, "pong");

    // The turn has finished, so the session may go idle.
    let mut idle = false;
    for _ in 0..50 {
        if ping(&client, &base).await == "Healthy" {
            idle = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(idle, "/ping stayed HealthyBusy after the turn finished");
}

#[tokio::test]
async fn invocation_without_a_thread_or_session_is_a_problem() {
    let booted = boot().await;
    let base = booted.base.clone();
    let response = reqwest::Client::new()
        .post(format!("{base}/invocations"))
        .json(&json!({ "prompt": "hi" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    assert_eq!(
        response.headers()["content-type"],
        "application/problem+json"
    );
    let body: Value = response.json().await.unwrap();
    assert!(body["detail"].as_str().unwrap().contains(SESSION_HEADER));
}

#[tokio::test]
async fn serve_wire_api_is_still_served() {
    let booted = boot().await;
    let base = booted.base.clone();
    let health: Value = reqwest::get(format!("{base}/health"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(health["status"], "ok");
    let card: Value = reqwest::get(format!("{base}/v1/agent"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(card["agents"][0]["name"], "assistant");
}

#[tokio::test]
async fn ping_answers_before_anything_boots_and_storage_opens_on_first_use() {
    let dir = tempfile::tempdir().unwrap();
    let base = boot_in(dir.path()).await;
    let client = reqwest::Client::new();
    assert_eq!(ping(&client, &base).await, "Healthy");
    // AgentCore mounts session storage only at the first invocation, so a
    // health check must not create the store.
    assert!(!dir.path().join("data").exists());

    reqwest::get(format!("{base}/health")).await.unwrap();
    assert!(dir.path().join("data").join("serve.db").exists());
}

#[tokio::test]
async fn sessions_survive_a_microvm_restart_on_the_same_storage() {
    let dir = tempfile::tempdir().unwrap();
    let client = reqwest::Client::new();

    let first = boot_in(dir.path()).await;
    let session: Value = client
        .post(format!("{first}/v1/sessions"))
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = session["id"].as_str().unwrap().to_string();
    let message =
        json!({ "message": { "role": "user", "content": [{ "type": "text", "text": "hi" }] } });
    let sent = client
        .post(format!("{first}/v1/sessions/{id}/messages"))
        .json(&message)
        .send()
        .await
        .unwrap();
    assert!(sent.status().is_success(), "{}", sent.status());
    // Wait for the turn to finish so its events are durable.
    let mut done = false;
    for _ in 0..100 {
        let view: Value = client
            .get(format!("{first}/v1/sessions/{id}"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if view["status"] == "idle" {
            done = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert!(done, "the first turn never finished");

    // A new process on the same storage: what a resumed AgentCore session is.
    let second = boot_in(dir.path()).await;
    let events: Value = client
        .get(format!("{second}/v1/sessions/{id}/events"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let kinds: Vec<&str> = events["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|event| event["type"].as_str())
        .collect();
    assert!(kinds.contains(&"input.message"), "{kinds:?}");
    assert!(kinds.contains(&"output.message.completed"), "{kinds:?}");
}

/// Reads text frames until `RUN_FINISHED` or `RUN_ERROR`.
async fn ws_run<S>(socket: &mut S) -> Vec<Value>
where
    S: futures_util::Stream<
            Item = Result<
                tokio_tungstenite::tungstenite::Message,
                tokio_tungstenite::tungstenite::Error,
            >,
        > + Unpin,
{
    use futures_util::StreamExt;
    let mut events = Vec::new();
    while let Some(message) = tokio::time::timeout(Duration::from_secs(10), socket.next())
        .await
        .expect("a frame within 10s")
    {
        let text = message.unwrap().into_text().unwrap();
        let event: Value = serde_json::from_str(text.as_str()).unwrap();
        let last = matches!(event["type"].as_str(), Some("RUN_FINISHED" | "RUN_ERROR"));
        events.push(event);
        if last {
            break;
        }
    }
    events
}

#[tokio::test]
async fn websocket_carries_runs_one_event_per_message() {
    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

    let booted = boot().await;
    let session = "agentcore-session-ws-0123456789abcdef01234567";
    let mut request = format!("{}/ws", booted.base.replace("http://", "ws://"))
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert(SESSION_HEADER, session.parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();

    // Two runs in turn on one connection, both on the AgentCore session.
    for _ in 0..2 {
        socket
            .send(Message::text(json!({ "prompt": "hi" }).to_string()))
            .await
            .unwrap();
        let events = ws_run(&mut socket).await;
        let kinds: Vec<&str> = events.iter().filter_map(|e| e["type"].as_str()).collect();
        assert_eq!(kinds.first(), Some(&"RUN_STARTED"), "{kinds:?}");
        assert_eq!(kinds.last(), Some(&"RUN_FINISHED"), "{kinds:?}");
        assert_eq!(events[0]["threadId"], session);
    }

    // A body that cannot run is one RUN_ERROR, and the socket stays open.
    socket.send(Message::text("{}")).await.unwrap();
    let events = ws_run(&mut socket).await;
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["type"], "RUN_ERROR");
    assert_eq!(events[0]["code"], "400");
    socket
        .send(Message::text(json!({ "prompt": "again" }).to_string()))
        .await
        .unwrap();
    let events = ws_run(&mut socket).await;
    assert_eq!(events.last().unwrap()["type"], "RUN_FINISHED");
}
