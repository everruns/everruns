//! Chat latency regression: an idle worker picks up a new message at once.
//!
//! The standalone worker's poll loop backs off to 5s while the queue is empty,
//! so before push wake-ups a message sent after a pause waited up to 5s before
//! its turn even started. This drives the real server + worker pair the way a
//! user does (send, pause, send again) and reads the pickup delay straight from
//! the session's event timestamps: `input.message` to `turn.started`.

use crate::support::*;
use chrono::{DateTime, Utc};
use everruns_contracts::model::Model;
use everruns_contracts::provider::Provider;
use everruns_platform::{Agent, Session};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

/// Longer than the worker's default `poll_backoff_max` (5s), so the poll loop
/// is fully backed off when the next message arrives.
const IDLE_PAUSE: Duration = Duration::from_secs(7);

/// With push wake-ups pickup is tens of milliseconds; the bound leaves room for
/// a loaded CI runner while staying far below the 5s backoff it guards against.
const MAX_PICKUP_MS: i64 = 1_500;

fn event_ts(event: &Value) -> DateTime<Utc> {
    event["ts"]
        .as_str()
        .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok())
        .map(|ts| ts.with_timezone(&Utc))
        .unwrap_or_else(|| panic!("event without ts: {event}"))
}

async fn session_events(client: &reqwest::Client, session_id: &str) -> Vec<Value> {
    let body: Value = client
        .get(format!(
            "{}/v1/sessions/{}/events?limit=1000",
            API_BASE_URL, session_id
        ))
        .send()
        .await
        .expect("list events")
        .json()
        .await
        .expect("parse events");
    body["data"].as_array().cloned().unwrap_or_default()
}

/// Send one message and return its pickup delay in milliseconds once the turn
/// has finished.
async fn send_and_measure_pickup(client: &reqwest::Client, session_id: &str) -> i64 {
    let seen = session_events(client, session_id).await.len();
    let response = client
        .post(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session_id
        ))
        .json(&json!({"message": {"content": [{"type": "text", "text": "Hello"}]}}))
        .send()
        .await
        .expect("send message");
    assert_eq!(response.status(), 201, "send message");

    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let events = session_events(client, session_id).await;
        let turn: Vec<&Value> = events.iter().skip(seen).collect();
        if turn.iter().any(|e| e["type"] == "turn.completed") {
            let at = |event_type: &str| {
                turn.iter()
                    .find(|e| e["type"] == event_type)
                    .map(|e| event_ts(e))
                    .unwrap_or_else(|| panic!("turn has no {event_type}: {turn:?}"))
            };
            return (at("turn.started") - at("input.message")).num_milliseconds();
        }
        let owned: Vec<Value> = turn.iter().map(|e| (*e).clone()).collect();
        if let Some(failure) = turn_failure(&owned) {
            panic!("turn failed: {failure}");
        }
        assert!(Instant::now() < deadline, "turn did not finish: {turn:?}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn test_idle_worker_picks_up_new_message_promptly() {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("client");

    let provider: Provider = client
        .post(format!("{}/v1/providers", API_BASE_URL))
        .json(&json!({"name": "LlmSim Latency Provider", "provider_type": "llmsim"}))
        .send()
        .await
        .expect("create provider")
        .json()
        .await
        .expect("parse provider");
    let model: Model = client
        .post(format!(
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": "llmsim-pickup-latency",
            "display_name": "LlmSim Pickup Latency",
            "enabled": true
        }))
        .send()
        .await
        .expect("create model")
        .json()
        .await
        .expect("parse model");
    let agent: Agent = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "pickup-latency-agent",
            "system_prompt": "Reply briefly.",
            "default_model_id": model.id
        }))
        .send()
        .await
        .expect("create agent")
        .json()
        .await
        .expect("parse agent");
    let session: Session = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({
            "harness_name": SEED_HARNESS_NAME,
            "agent_id": agent.public_id,
            "title": "Pickup latency"
        }))
        .send()
        .await
        .expect("create session")
        .json()
        .await
        .expect("parse session");
    let session_id = session.id.to_string();

    // Warm-up turn: the first turn of a session pays one-off costs that are not
    // what this test is about.
    send_and_measure_pickup(&client, &session_id).await;

    let mut pickups = Vec::new();
    for _ in 0..2 {
        tokio::time::sleep(IDLE_PAUSE).await;
        pickups.push(send_and_measure_pickup(&client, &session_id).await);
    }
    println!("pickup after {IDLE_PAUSE:?} idle: {pickups:?} ms");

    // `sessions.model_id` references the model, so the session goes first.
    cleanup_delete(
        &client,
        format!("{}/v1/sessions/{}", API_BASE_URL, session_id),
        "session",
    )
    .await;
    cleanup_agent(&client, &agent.public_id).await;
    cleanup_delete(
        &client,
        format!("{}/v1/models/{}", API_BASE_URL, model.id),
        "model",
    )
    .await;
    cleanup_delete(
        &client,
        format!("{}/v1/providers/{}", API_BASE_URL, provider.id),
        "provider",
    )
    .await;

    for pickup in pickups {
        assert!(
            pickup < MAX_PICKUP_MS,
            "idle worker took {pickup}ms to start the turn (limit {MAX_PICKUP_MS}ms): \
             it waited for its poll backoff instead of a push wake-up"
        );
    }
}
