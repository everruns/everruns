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

fn generation(provider: &str, model: &str, input: u64, output: u64, cache_read: u64) -> Value {
    json!({
        "messages": [],
        "output": { "text": "", "tool_calls": [] },
        "metadata": {
            "model": format!("{model}-alias"),
            "response_model": model,
            "provider": provider,
            "usage": {
                "input_tokens": input,
                "output_tokens": output,
                "cache_read_tokens": cache_read,
            },
        },
    })
}

#[test]
fn usage_is_summed_per_provider_and_model_in_ag_ui_accounting() {
    let mut projector = Projector::new("t", "r", policy());
    projector.project(
        "llm.generation",
        &generation("openai", "gpt-a", 100, 10, 40),
    );
    projector.project("llm.generation", &generation("openai", "gpt-a", 50, 5, 0));
    projector.project(
        "llm.generation",
        &generation("anthropic", "claude-b", 7, 3, 0),
    );
    // A generation without usage reports nothing rather than zeros.
    projector.project("llm.generation", &json!({ "metadata": { "model": "x" } }));
    projector.project("turn.completed", &json!({}));
    let events: Vec<Event> = projector.drain().collect();
    assert_conformant(&events);
    let Some(Event::RunFinished(finished)) = events.last() else {
        panic!("expected RUN_FINISHED");
    };
    let usage = finished.usage.as_ref().expect("usage reported");
    assert_eq!(usage.len(), 2);
    let gpt = usage
        .iter()
        .find(|u| u.model.as_deref() == Some("gpt-a"))
        .unwrap();
    // Cache reads are disjoint in Everruns and part of the input total in AG-UI.
    assert_eq!(gpt.input_tokens, Some(190));
    assert_eq!(gpt.cached_input_tokens, Some(40));
    assert_eq!(gpt.output_tokens, Some(15));
    assert_eq!(gpt.total_tokens, Some(205));
    assert_eq!(gpt.provider.as_deref(), Some("openai"));
    assert_eq!(gpt.reasoning_tokens, None);
}

#[test]
fn usage_is_withheld_when_the_policy_hides_it() {
    let mut projector = Projector::new(
        "t",
        "r",
        ProjectionPolicy {
            usage_visible: false,
            ..ProjectionPolicy::default()
        },
    );
    projector.project("llm.generation", &generation("openai", "gpt-a", 1, 1, 0));
    projector.project(
        "turn.failed",
        &json!({ "turn_id": TurnId::new(), "error": "x" }),
    );
    let events: Vec<Event> = projector.drain().collect();
    let Some(Event::RunError(error)) = events.last() else {
        panic!("expected RUN_ERROR");
    };
    assert_eq!(error.usage, None);
}

#[test]
fn interrupt_closes_open_messages_and_ends_the_run() {
    let mut projector = Projector::new("t", "r", policy());
    projector.project("reason.thinking.delta", &json!({ "delta": "I should ask" }));
    projector.project(
        "output.message.delta",
        &delta(MessageId::new(), "One question:"),
    );
    projector.project("tool.started", &json!({}));
    projector.project("llm.generation", &generation("openai", "gpt-a", 3, 2, 0));
    // Nothing to answer is not an interrupt.
    projector.interrupt(Vec::new());
    assert!(!projector.is_finished());
    projector.interrupt(vec![everruns_ag_ui::Interrupt::new(
        "call_1",
        "everruns.ask_user",
    )]);
    let events: Vec<Event> = projector.drain().collect();
    assert_conformant(&events);
    let Some(Event::RunFinished(finished)) = events.last() else {
        panic!("expected RUN_FINISHED");
    };
    let Some(RunFinishedOutcome::Interrupt { interrupts }) = &finished.outcome else {
        panic!("expected the interrupt outcome, got {:?}", finished.outcome);
    };
    assert_eq!(interrupts[0].id, "call_1");
    assert!(
        finished.usage.is_some(),
        "an interrupted run reports its usage"
    );
}

#[test]
fn frontend_tool_calls_stream_under_their_message_and_end_in_success() {
    let mut projector = Projector::new("t", "r", policy());
    let carrier = MessageId::new();
    let call = everruns_provider::tool_types::ToolCall {
        id: "call_1".into(),
        name: "confirm".into(),
        arguments: json!({ "text": "ok?" }),
    };
    projector.project("output.message.delta", &delta(carrier, "Checking."));
    projector.project(
        "output.message.completed",
        &serde_json::to_value(OutputMessageCompletedData::new(
            RuntimeMessage::assistant_with_tools("Checking.", vec![call]).with_id(carrier),
        ))
        .unwrap(),
    );
    assert!(
        !projector.is_finished(),
        "a tool-call carrier is not the answer"
    );
    projector.park(
        vec![everruns_ag_ui::ToolCall::function(
            "call_1",
            "confirm",
            r#"{"text":"ok?"}"#,
        )],
        Vec::new(),
    );
    let events: Vec<Event> = projector.drain().collect();
    assert_conformant(&events);
    assert_eq!(
        types(&events[events.len() - 4..]),
        [
            "TOOL_CALL_START",
            "TOOL_CALL_ARGS",
            "TOOL_CALL_END",
            "RUN_FINISHED"
        ]
    );
    let Some(Event::ToolCallStart(start)) =
        events.iter().find(|e| e.event_type() == "TOOL_CALL_START")
    else {
        panic!("expected TOOL_CALL_START");
    };
    assert_eq!(start.tool_call_name, "confirm");
    assert_eq!(
        start.parent_message_id.as_deref(),
        Some(carrier.uuid().to_string().as_str())
    );
    let Some(Event::RunFinished(finished)) = events.last() else {
        panic!("expected RUN_FINISHED");
    };
    assert_eq!(
        finished.outcome,
        Some(RunFinishedOutcome::Success {
            pending_tool_call_ids: Some(vec!["call_1".to_string()]),
        })
    );
}

#[test]
fn frontend_tool_calls_beside_an_interrupt_end_in_the_interrupt() {
    let mut projector = Projector::new("t", "r", policy());
    projector.park(
        vec![everruns_ag_ui::ToolCall::function("call_2", "confirm", "")],
        vec![everruns_ag_ui::Interrupt::new(
            "call_1",
            "everruns.ask_user",
        )],
    );
    let events: Vec<Event> = projector.drain().collect();
    assert_conformant(&events);
    assert_eq!(
        types(&events),
        ["TOOL_CALL_START", "TOOL_CALL_END", "RUN_FINISHED"],
        "empty arguments send no TOOL_CALL_ARGS"
    );
    let Some(Event::RunFinished(finished)) = events.last() else {
        panic!("expected RUN_FINISHED");
    };
    assert!(matches!(
        finished.outcome,
        Some(RunFinishedOutcome::Interrupt { .. })
    ));
}
