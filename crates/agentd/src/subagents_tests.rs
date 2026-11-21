//! Reading a child's outcome and answer from its events.

use super::*;
use everruns_core::events::EventContext;
use serde_json::Value;

fn event(kind: &str, data: Value) -> Event {
    let mut event: Event = serde_json::from_value(serde_json::json!({
        "id": everruns_contracts::typed_id::EventId::new(),
        "type": kind,
        "ts": "2027-01-15T08:00:00Z",
        "session_id": SessionId::new(),
        "context": EventContext::default(),
        "data": data,
    }))
    .unwrap();
    event.event_type = kind.to_owned();
    event
}

#[test]
fn the_latest_finished_turn_is_the_outcome() {
    let started = || event("turn.started", serde_json::json!({}));
    let done = || event("turn.completed", serde_json::json!({}));
    let failed = || event("turn.failed", serde_json::json!({}));
    assert_eq!(outcome(&[]), None);
    assert_eq!(outcome(&[started(), done()]), Some("completed"));
    assert_eq!(outcome(&[started(), done(), started()]), None, "one runs");
    assert_eq!(
        outcome(&[started(), done(), started(), failed()]),
        Some("failed")
    );
}

#[test]
fn a_message_reads_as_its_text() {
    let said = |text: &str| {
        let message = everruns_core::RuntimeMessage::assistant(text);
        event(
            "output.message.completed",
            serde_json::json!({ "message": message }),
        )
    };
    assert_eq!(text_of(&said("found it")).as_deref(), Some("found it"));
    assert_eq!(text_of(&said("")), None);
}
