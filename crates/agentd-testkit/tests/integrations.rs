//! An org's integrations on the data plane (data plane v2, step 11.5).
//!
//! A channel (Slack and the others) reaches a v2 session through the
//! control plane's `/dp/v1/orgs/{org}/sessions/{id}/...` routes: it admits
//! a message with its thread and sender, seeds history, cancels a turn and
//! follows the session's events. Observers, trace exports and reports take
//! the session events the org's policy names, which the shard copies onto
//! the feed.
//!
//! The bench holds the step's gate and prints its numbers (`cargo test -p
//! agentd-testkit --test integrations -- --nocapture`).

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use everruns_contracts::typed_id::MessageId;
use futures::StreamExt;
use reqwest::StatusCode;
use serde_json::{Value, json};
use support::{Fleet, answers};

use support::org::{ORG, command, org_fleet, session};

/// The session's events after `after`, as a channel follows them, until
/// one of type `until`.
async fn follow(fleet: &Fleet, session: &str, after: i64, until: &str) -> Vec<Value> {
    let url = format!(
        "{}/dp/v1/orgs/{ORG}/sessions/{session}/sse?after_sequence={after}",
        fleet.base()
    );
    let response = fleet.http.get(url).send().await.unwrap();
    assert!(response.status().is_success(), "{}", response.status());
    let mut body = response.bytes_stream();
    let (mut buffer, mut events) = (String::new(), Vec::new());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while let Ok(Some(chunk)) = tokio::time::timeout_at(deadline, body.next()).await {
        buffer.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
        while let Some(end) = buffer.find('\n') {
            let line = buffer[..end].trim().to_owned();
            buffer.drain(..=end);
            let Some(data) = line.strip_prefix("data:") else {
                continue;
            };
            let Ok(event) = serde_json::from_str::<Value>(data.trim()) else {
                continue;
            };
            let done = event["type"] == until;
            events.push(event);
            if done {
                return events;
            }
        }
    }
    panic!("{session}: no {until} in {events:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_channel_reaches_a_v2_session_and_its_events_reach_the_control_plane() {
    let (fleet, cp) = org_fleet(&["turn.completed"]).await;
    let session = session(&fleet, &cp).await;

    // A Slack message: its thread and sender go with it, and the answer
    // says it did not steer a running turn, so the host owes a reply (the
    // turn starts when an executor takes it: `pending`).
    let message = json!({
        "id": MessageId::new(),
        "text": "ping",
        "metadata": { "slack_thread_ts": "1700000000.0001" },
        "external_actor": { "actor_id": "U1", "actor_name": "Ann", "source": "slack" },
    });
    let (status, admitted) = command(&fleet, &cp, ORG, &session, "messages", message).await;
    assert_eq!(status, StatusCode::OK, "{admitted}");
    assert_ne!(admitted["disposition"], "steered", "{admitted}");
    let events = follow(&fleet, &session, 0, "turn.completed").await;
    assert_eq!(answers(&events).len(), 1, "the reply: {events:?}");
    let input = events
        .iter()
        .find(|e| e["type"] == "input.message")
        .unwrap();
    let carried = input["data"]["message"].to_string();
    assert!(
        carried.contains("1700000000.0001") && carried.contains("\"U1\""),
        "the thread and the sender: {carried}"
    );
    // Following again from the last sequence seen (a reconnect) starts
    // after it.
    let last = events.last().unwrap()["sequence"].as_i64().unwrap();
    let (_, again) = command(
        &fleet,
        &cp,
        ORG,
        &session,
        "messages",
        json!({ "id": MessageId::new(), "text": "again" }),
    )
    .await;
    assert_ne!(again["disposition"], "steered", "{again}");
    let later = follow(&fleet, &session, last, "turn.completed").await;
    assert!(
        later
            .iter()
            .filter(|e| e["type"].is_string())
            .all(|e| e["sequence"].as_i64().is_none_or(|s| s > last)),
        "{later:?}"
    );

    // Cancel with no turn running is a no-op.
    let (status, _) = command(
        &fleet,
        &cp,
        ORG,
        &session,
        "cancel-turn",
        json!({ "command": uuid::Uuid::new_v4() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // History is seeded only while a session has no conversation.
    let fresh = self::session(&fleet, &cp).await;
    let history = json!([
        everruns_core::RuntimeMessage::user("earlier"),
        everruns_core::RuntimeMessage::assistant("noted"),
    ]);
    let seed = || json!({ "command": uuid::Uuid::new_v4(), "messages": history.clone() });
    let (status, seeded) = command(&fleet, &cp, ORG, &fresh, "seed", seed()).await;
    assert_eq!(status, StatusCode::OK, "{seeded}");
    assert_eq!(seeded["seeded"], true, "{seeded}");
    let (_, seeded) = command(&fleet, &cp, ORG, &session, "seed", seed()).await;
    assert_eq!(seeded["seeded"], false, "a session with a conversation");

    // Another org's session reads as missing, and only these commands go.
    for (org, name) in [
        ("org_other", "messages"),
        ("org_other", "seed"),
        (ORG, "delete"),
    ] {
        let (status, _) = command(&fleet, &cp, org, &session, name, json!({})).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{org} {name}");
    }

    // The org's policy taps turn completions only: one per turn, its org's.
    fleet
        .until("both turns tapped", || async {
            cp.tapped_events()
                .iter()
                .filter(|(_, s, _)| *s == session)
                .count()
                >= 2
        })
        .await;
    let tapped: Vec<_> = cp
        .tapped_events()
        .into_iter()
        .filter(|(_, s, _)| *s == session)
        .collect();
    assert_eq!(tapped.len(), 2, "{tapped:?}");
    for (org, _, event) in &tapped {
        assert_eq!(org, ORG);
        assert_eq!(event["type"], "turn.completed", "{event}");
    }
}

fn percentile(sorted: &[Duration], p: usize) -> Duration {
    sorted[(sorted.len() * p).div_ceil(100).max(1) - 1]
}

/// The gate: a channel message gets its reply within 5 s p99, a traced
/// event reaches the control plane well within 2 minutes, and observers
/// see every turn exactly once, across a leader change.
#[tokio::test(flavor = "multi_thread")]
async fn bench_channel_replies_and_tapped_events() {
    const SESSIONS: usize = 200;
    let traced = ["turn.started", "turn.completed", "llm.generation"];
    let (mut fleet, cp) = org_fleet(&traced).await;
    let mut sessions = Vec::new();
    for _ in 0..SESSIONS {
        sessions.push(session(&fleet, &cp).await);
    }

    // Each session takes a message and is followed to its reply, as the
    // channel host does; then the directory shard's leader is killed, and
    // every session takes another message.
    let fleet_ref = &fleet;
    let cp_ref = &cp;
    let run = futures::stream::iter(sessions.iter().cloned())
        .map(|session| async move {
            let start = Instant::now();
            let message = json!({ "id": MessageId::new(), "text": "ping" });
            let (status, _) = command(fleet_ref, cp_ref, ORG, &session, "messages", message).await;
            assert_eq!(status, StatusCode::OK);
            follow(fleet_ref, &session, 0, "turn.completed").await;
            start.elapsed()
        })
        .buffer_unordered(10)
        .collect::<Vec<_>>();
    let mut replies = run.await;
    replies.sort_unstable();
    let leader = fleet.leader(0).await;
    fleet.kill_node(0, leader);
    let killed = Instant::now();
    fleet.leader(0).await;
    let election = killed.elapsed();
    for session in &sessions {
        let message = json!({ "id": MessageId::new(), "text": "again" });
        let (status, _) = command(&fleet, &cp, ORG, session, "messages", message).await;
        assert_eq!(status, StatusCode::OK);
    }
    for session in &sessions {
        fleet.turns(session, 2).await;
    }

    // Every turn's completion arrives once, whichever leader sent it.
    let wanted = SESSIONS * 2;
    fleet
        .until("every turn tapped", || async {
            cp.tapped_events()
                .iter()
                .filter(|(_, _, e)| e["type"] == "turn.completed")
                .count()
                >= wanted
        })
        .await;
    let mut per_turn: HashMap<String, usize> = HashMap::new();
    for (_, _, event) in cp.tapped_events() {
        if event["type"] == "turn.completed" {
            *per_turn
                .entry(event["data"]["turn_id"].to_string())
                .or_default() += 1;
        }
    }
    assert_eq!(per_turn.len(), wanted, "every turn");
    assert!(per_turn.values().all(|n| *n == 1), "exactly once");
    let feed_p99 = cp.feed_lag(99).unwrap();
    println!(
        "channel reply: p50 {:?}, p99 {:?}, max {:?}; tapped event to control plane: p99 {feed_p99:?}; election after the leader died: {election:?}",
        percentile(&replies, 50),
        percentile(&replies, 99),
        replies.last().unwrap(),
    );
    assert!(percentile(&replies, 99) < Duration::from_secs(5));
    assert!(feed_p99 < Duration::from_secs(120));
}
