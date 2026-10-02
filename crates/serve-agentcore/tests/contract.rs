//! The AgentCore Runtime contract over a real socket, against an app
//! registered with serve's macros. Offline: the agent runs the simulator.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use serde_json::{Value, json};
use serve::prelude::*;
use serve::{Mode, Server, sim};
use serve_agentcore::{SESSION_HEADER, router};

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

async fn boot() -> String {
    let app = App::builder().discover().build();
    assert!(app.errors().is_empty(), "{:?}", app.errors());
    let server = Server::new(app, Mode::Eval, None).unwrap();
    let agent = server.default_agent().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = router(&server, agent);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    base
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
    let base = boot().await;
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
    let base = boot().await;
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
    let base = boot().await;
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
