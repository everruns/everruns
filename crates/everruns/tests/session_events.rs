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
    while let Some(event) = stream.recv().await.expect("event stream stays lossless") {
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
    let output_started = position(|e| matches!(e.kind, SessionEventKind::OutputStarted { .. }))
        .expect("an output.message.started event");
    let output_completed = position(|e| matches!(e.kind, SessionEventKind::OutputCompleted { .. }))
        .expect("an output.message.completed event");
    let model_generation = position(|e| matches!(e.kind, SessionEventKind::ModelGeneration))
        .expect("an llm.generation event");

    assert!(
        started < output_started
            && output_started < delta
            && delta < output_completed
            && model_generation < output_completed
            && output_completed < completed,
        "expected ordered turn start < output start < delta < output completion < turn completion"
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

    // The raw side of the bridge is the complete canonical event protocol, not
    // a second facade vocabulary. Durable replay sequence is optional because
    // live-only deltas are ephemeral; channel arrival order is the live ordering
    // contract. Every envelope retains timestamp, context, data, metadata, and
    // tags semantics.
    let sequences: Vec<i64> = events
        .iter()
        .filter_map(|event| {
            let raw = event.as_json();
            assert!(raw["id"].as_str().is_some());
            assert_eq!(raw["type"], event.event_type());
            assert!(raw["ts"].as_str().is_some());
            assert!(raw["session_id"].as_str().is_some());
            assert!(raw["context"].is_object());
            assert!(raw.get("data").is_some());
            let raw_sequence = raw.get("sequence").and_then(serde_json::Value::as_i64);
            assert_eq!(raw_sequence.map(|value| value as i32), event.sequence());
            raw_sequence
        })
        .collect();
    assert!(
        sequences.windows(2).all(|pair| pair[0] < pair[1]),
        "durable events retain strictly increasing replay positions"
    );
    assert!(
        events
            .iter()
            .filter(|event| matches!(event.kind, SessionEventKind::TextDelta { .. }))
            .all(|event| event.sequence().is_none()),
        "ephemeral deltas are ordered by live arrival, not replay sequence"
    );

    let output_message_ids: Vec<&str> = events
        .iter()
        .filter_map(|event| match &event.kind {
            SessionEventKind::OutputStarted { message_id }
            | SessionEventKind::OutputCompleted { message_id } => Some(message_id.as_str()),
            SessionEventKind::TextDelta { .. } => event.data()["message_id"].as_str(),
            _ => None,
        })
        .collect();
    assert!(!output_message_ids.is_empty());
    assert!(
        output_message_ids.windows(2).all(|pair| pair[0] == pair[1]),
        "one assistant message id spans start, deltas, and completion"
    );

    // Conversation history is rebuildable from canonical message events. The
    // facade adds no second writable message representation.
    let input = events
        .iter()
        .find(|event| event.event_type() == "input.message")
        .expect("canonical input message");
    let output = events
        .iter()
        .find(|event| event.event_type() == "output.message.completed")
        .expect("canonical assistant message");
    assert!(matches!(input.kind, SessionEventKind::InputMessage { .. }));
    assert_eq!(input.data()["message"]["role"], "user");
    assert!(input.data()["message"]["content"].is_array());
    assert_eq!(output.data()["message"]["role"], "agent");
    assert!(output.data()["message"]["content"].is_array());
    assert!(input.sequence().unwrap() < output.sequence().unwrap());

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
    let stream = session.events();
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

    drop(session);
    let events = drain(stream).await;
    let cancelled = events
        .iter()
        .find(|event| matches!(event.kind, SessionEventKind::TurnCancelled))
        .expect("cancellation emits its canonical terminal event");
    assert_eq!(cancelled.turn_id.as_deref(), Some(turn.turn_id.as_str()));
    assert_eq!(cancelled.data()["reason"], "cancelled by application");
    assert!(cancelled.sequence().is_some(), "cancellation is durable");
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
