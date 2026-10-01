#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! The runtime-event projection keeps 1.0 sequencing on every path.

use everruns_ag_ui::projection::{ProjectionPolicy, Projector};
use everruns_ag_ui::{Event, RunFinishedOutcome, SCHEMA_JSON};
use everruns_core::RuntimeMessage;
use everruns_core::events::{OutputMessageCompletedData, OutputMessageDeltaData};
use everruns_provider::typed_id::{MessageId, TurnId};
use serde_json::{Value, json};

fn policy() -> ProjectionPolicy {
    ProjectionPolicy {
        tool_activity_text: Some("Working...".into()),
        ..ProjectionPolicy::default()
    }
}

fn types(events: &[Event]) -> Vec<&'static str> {
    events.iter().map(Event::event_type).collect()
}

/// Every opened text message, reasoning message and span is closed before the
/// run's terminal event, and everything validates against the schema.
fn assert_conformant(events: &[Event]) {
    let mut schema: Value = serde_json::from_str(SCHEMA_JSON).unwrap();
    schema["$ref"] = json!("#/$defs/Event");
    let validator = jsonschema::validator_for(&schema).unwrap();
    let mut open: Vec<String> = Vec::new();
    for (i, event) in events.iter().enumerate() {
        let value = serde_json::to_value(event).unwrap();
        assert!(validator.is_valid(&value), "schema rejected {value}");
        let id = value["messageId"].as_str().unwrap_or_default().to_string();
        match event {
            Event::TextMessageStart(_)
            | Event::ReasoningMessageStart(_)
            | Event::ReasoningStart(_) => open.push(format!("{}:{id}", &event.event_type()[..9])),
            Event::TextMessageEnd(_) | Event::ReasoningMessageEnd(_) | Event::ReasoningEnd(_) => {
                let key = format!("{}:{id}", &event.event_type()[..9]);
                let pos = open.iter().position(|k| *k == key);
                assert!(pos.is_some(), "{} closes nothing open", event.event_type());
                open.remove(pos.unwrap());
            }
            Event::RunFinished(_) | Event::RunError(_) => {
                assert!(open.is_empty(), "still open at terminal: {open:?}");
                assert_eq!(i, events.len() - 1, "events after the terminal event");
            }
            _ => {}
        }
    }
}

fn delta(message_id: MessageId, text: &str) -> Value {
    serde_json::to_value(OutputMessageDeltaData {
        turn_id: TurnId::new(),
        message_id,
        delta: text.into(),
        accumulated: text.into(),
        phase: None,
    })
    .unwrap()
}

#[test]
fn full_turn_with_reasoning_and_tools_is_conformant() {
    let mut projector = Projector::new("t", "r", policy());
    let answer = MessageId::new();
    projector.project("reason.thinking.started", &json!({}));
    projector.project("reason.thinking.delta", &json!({ "delta": "Let me think" }));
    projector.project("tool.started", &json!({}));
    projector.project("tool.completed", &json!({}));
    projector.project("reason.thinking.completed", &json!({}));
    projector.project("tool.started", &json!({}));
    projector.project("tool.completed", &json!({}));
    projector.project("output.message.delta", &delta(answer, "Hello"));
    projector.project(
        "output.message.completed",
        &serde_json::to_value(OutputMessageCompletedData::new(
            RuntimeMessage::assistant("Hello").with_id(answer),
        ))
        .unwrap(),
    );
    let events: Vec<Event> = projector.drain().collect();
    assert_conformant(&events);
    assert!(projector.is_finished());
    assert_eq!(
        types(&events)[..3],
        [
            "REASONING_START",
            "REASONING_MESSAGE_START",
            "REASONING_MESSAGE_CONTENT"
        ]
    );
    let Some(Event::TextMessageStart(start)) = events
        .iter()
        .find(|e| matches!(e, Event::TextMessageStart(_)))
    else {
        panic!("no text message");
    };
    assert_eq!(start.message_id, answer.uuid().to_string());
}

#[test]
fn cancelled_turn_closes_open_messages_and_reports_cancelled() {
    let mut projector = Projector::new("t", "r", policy());
    projector.project("reason.thinking.delta", &json!({ "delta": "hmm" }));
    projector.project("output.message.delta", &delta(MessageId::new(), "partial"));
    projector.project("turn.cancelled", &json!({}));
    let events: Vec<Event> = projector.drain().collect();
    assert_conformant(&events);
    let Some(Event::RunFinished(finished)) = events.last() else {
        panic!("expected RUN_FINISHED");
    };
    assert_eq!(finished.outcome, Some(RunFinishedOutcome::Cancelled));
}

#[test]
fn failed_turn_closes_open_spans_and_uses_the_error_policy() {
    let mut projector = Projector::new("t", "r", policy());
    projector.project("tool.started", &json!({}));
    projector.project(
        "turn.failed",
        &json!({
            "turn_id": TurnId::new(),
            "error": "provider exploded",
            "error_code": "provider_unavailable",
        }),
    );
    let events: Vec<Event> = projector.drain().collect();
    assert_conformant(&events);
    let Some(Event::RunError(error)) = events.last() else {
        panic!("expected RUN_ERROR");
    };
    assert_eq!(error.code.as_deref(), Some("provider_unavailable"));
    // Input after the terminal event is ignored.
    projector.project("turn.completed", &json!({}));
    assert_eq!(projector.drain().count(), 0);
}

#[test]
fn hidden_reasoning_and_tools_emit_only_the_answer() {
    let mut projector = Projector::new(
        "t",
        "r",
        ProjectionPolicy {
            reasoning_visible: false,
            tool_activity_text: None,
            ..ProjectionPolicy::default()
        },
    );
    projector.project("reason.thinking.delta", &json!({ "delta": "secret" }));
    projector.project("tool.started", &json!({}));
    projector.project("tool.completed", &json!({}));
    projector.project("turn.completed", &json!({}));
    let events: Vec<Event> = projector.drain().collect();
    assert_eq!(types(&events), ["RUN_FINISHED"]);
}
