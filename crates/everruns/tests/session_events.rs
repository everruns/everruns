//! Public-surface session events and cancellation (EVE-833).
//!
//! This file imports only `everruns::prelude::*` — no `everruns-core` or
//! `everruns-runtime` — proving library code can render streaming output and
//! cancel a turn using facade types alone. Behaviors that need the in-process
//! simulator's scripting/delay knobs (tool events, mid-flight cancellation) are
//! covered by the crate's unit tests, which reach those internal test helpers.

use everruns::prelude::*;

/// Collect every event from a stream until it closes (the session is dropped).
async fn drain(mut stream: EventStream) -> Vec<SessionEvent> {
    let mut events = Vec::new();
    while let Some(event) = stream.recv().await {
        events.push(event);
    }
    events
}

#[tokio::test]
async fn stream_emits_ordered_start_delta_completion() {
    let agent = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated("Hello, world!"))
        .build()
        .expect("valid agent");

    let mut session = agent.session();
    // Subscribe before running so the turn's events are observed from the start.
    let stream = session.events();

    let turn = session.run("hi").await.expect("turn runs");
    assert!(turn.success);
    assert_eq!(turn.response, "Hello, world!");

    // Drop the session so the stream closes and `drain` terminates.
    drop(session);
    let events = drain(stream).await;

    let position = |pred: fn(&SessionEvent) -> bool| events.iter().position(pred);
    let started = position(|e| matches!(e.kind, SessionEventKind::TurnStarted))
        .expect("a turn.started event");
    let delta = position(|e| matches!(e.kind, SessionEventKind::TextDelta { .. }))
        .expect("at least one text delta");
    let completed = position(|e| matches!(e.kind, SessionEventKind::TurnCompleted))
        .expect("a turn.completed event");

    assert!(
        started < delta && delta < completed,
        "expected ordered start < delta < completion, got indices {started}/{delta}/{completed}"
    );

    // The concatenated deltas reconstruct the assistant's response.
    let streamed: String = events
        .iter()
        .filter_map(|e| match &e.kind {
            SessionEventKind::TextDelta { delta } => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(streamed, "Hello, world!");

    // Correlation ids are preserved: turn-scoped events share one turn id, and
    // every event carries the same session id.
    let turn_ids: Vec<&String> = events.iter().filter_map(|e| e.turn_id.as_ref()).collect();
    assert!(!turn_ids.is_empty(), "turn-scoped events carry a turn id");
    assert!(
        turn_ids.windows(2).all(|w| w[0] == w[1]),
        "all turn-scoped events in one turn share a turn id"
    );
    let session_id = events[0].session_id.clone();
    assert!(
        events.iter().all(|e| e.session_id == session_id),
        "every event carries the session id"
    );
    assert_eq!(session_id, session_id_from_turn_stream(&events));
}

/// The session id is stable across every event of a single session's stream.
fn session_id_from_turn_stream(events: &[SessionEvent]) -> String {
    events
        .last()
        .map(|e| e.session_id.clone())
        .unwrap_or_default()
}

#[tokio::test]
async fn pre_cancelled_token_yields_cancelled_stop_reason() {
    let agent = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated("hi"))
        .build()
        .expect("valid agent");

    let mut session = agent.session();
    let token = CancellationToken::new();
    token.cancel();
    assert!(token.is_cancelled());

    // A token cancelled before the call stops the turn before it starts.
    let turn = session
        .run_with("hi", RunOptions::new().cancel_token(token))
        .await
        .expect("run_with resolves");
    assert!(!turn.success, "a cancelled turn is not a success");
    assert_eq!(turn.stop_reason, TurnStopReason::Cancelled);
}

#[tokio::test]
async fn dropped_and_slow_consumers_do_not_stall_the_runner() {
    let agent = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated("ok"))
        .build()
        .expect("valid agent");

    let mut session = agent.session();

    // One consumer subscribes then immediately drops its stream; another never
    // reads. Neither must block or fail the turn.
    let dropped = session.events();
    drop(dropped);
    let _never_read = session.events();

    let turn = session
        .run("hi")
        .await
        .expect("turn runs despite consumers");
    assert!(turn.success);
    assert_eq!(turn.response, "ok");
}

#[tokio::test]
async fn two_sessions_do_not_receive_each_others_events() {
    let agent = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated("ok"))
        .build()
        .expect("valid agent");

    let mut first = agent.session();
    let second = agent.session();

    let first_stream = first.events();
    let second_stream = second.events();
    let first_id = first.id();

    // Only the first session runs a turn.
    let turn = first.run("hi").await.expect("turn runs");
    assert!(turn.success);

    drop(first);
    drop(second);

    let first_events = drain(first_stream).await;
    let second_events = drain(second_stream).await;

    assert!(
        !first_events.is_empty(),
        "the session that ran a turn observes its events"
    );
    assert!(
        second_events.is_empty(),
        "a session that ran nothing observes no events from the other"
    );
    assert!(
        first_events.iter().all(|e| e.session_id == first_id),
        "every observed event belongs to the first session"
    );
}
