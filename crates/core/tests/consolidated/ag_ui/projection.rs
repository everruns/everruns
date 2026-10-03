#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! The runtime-event projection keeps 1.0 sequencing on every path.

use everruns_contracts::typed_id::{MessageId, TurnId};
use everruns_core::RuntimeMessage;
use everruns_core::ag_ui::projection::{ProjectionPolicy, Projector};
use everruns_core::ag_ui::{
    Event, RunErrorEvent, RunFinishedOutcome, SCHEMA_JSON, SubagentFinishedOutcome,
};
use everruns_core::events::{OutputMessageCompletedData, OutputMessageDeltaData};
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

/// Every opened text message, reasoning message, span and subagent is closed
/// before the run's terminal event, attributed events name an open subagent,
/// and everything validates against the schema.
fn assert_conformant(events: &[Event]) {
    let mut schema: Value = serde_json::from_str(SCHEMA_JSON).unwrap();
    schema["$ref"] = json!("#/$defs/Event");
    let validator = jsonschema::validator_for(&schema).unwrap();
    let mut open: Vec<String> = Vec::new();
    let mut subagents: Vec<String> = Vec::new();
    for (i, event) in events.iter().enumerate() {
        let value = serde_json::to_value(event).unwrap();
        assert!(validator.is_valid(&value), "schema rejected {value}");
        let subagent = value["subagentRunId"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        match event {
            Event::SubagentStarted(_) => subagents.push(subagent),
            Event::SubagentFinished(_) | Event::SubagentError(_) => {
                assert!(subagents.contains(&subagent), "closes no open subagent");
                subagents.retain(|id| *id != subagent);
            }
            _ if !subagent.is_empty() => {
                assert!(
                    subagents.contains(&subagent),
                    "attributed to no open subagent"
                );
            }
            _ => {}
        }
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
                assert!(subagents.is_empty(), "subagents open at terminal");
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
    projector.interrupt(vec![everruns_core::ag_ui::Interrupt::new(
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
    let call = everruns_contracts::tool_types::ToolCall {
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
        vec![everruns_core::ag_ui::ToolCall::function(
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
        vec![everruns_core::ag_ui::ToolCall::function(
            "call_2", "confirm", "",
        )],
        vec![everruns_core::ag_ui::Interrupt::new(
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

fn task(id: &str, kind: &str, mode: &str, state: &str, extra: Value) -> Value {
    let mut task = json!({
        "id": id,
        "session_id": everruns_contracts::typed_id::SessionId::new().to_string(),
        "kind": kind,
        "display_name": "Researcher",
        "spec": { "instructions": "secret instructions", "mode": mode },
        "state": state,
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:01Z",
    });
    if let (Some(task), Value::Object(extra)) = (task.as_object_mut(), extra) {
        task.extend(extra);
    }
    json!({ "task": task })
}

fn task_message(task_id: &str, id: &str, direction: &str, content: Value) -> Value {
    json!({
        "task_id": task_id,
        "message": {
            "id": id,
            "task_id": task_id,
            "direction": direction,
            "content": content,
            "created_at": "2026-01-01T00:00:00Z",
        },
    })
}

#[test]
fn foreground_subagent_streams_attributed_output_and_finishes() {
    let mut projector = Projector::new("t", "r", policy());
    projector.project(
        "task.created",
        &task("task_a", "subagent", "foreground", "running", json!({})),
    );
    projector.project(
        "task.message.received",
        &task_message(
            "task_a",
            "tmsg_1",
            "outbound",
            json!([{ "type": "text", "text": "halfway" }, { "type": "data", "data": { "step": 2 } }]),
        ),
    );
    projector.project(
        "task.message.sent",
        &task_message(
            "task_a",
            "tmsg_2",
            "inbound",
            json!([{ "type": "text", "text": "go on" }]),
        ),
    );
    projector.project(
        "task.updated",
        &task(
            "task_a",
            "subagent",
            "foreground",
            "succeeded",
            json!({ "summary": "Found it" }),
        ),
    );
    projector.project("turn.completed", &json!({}));
    let events: Vec<Event> = projector.drain().collect();
    assert_conformant(&events);
    assert_eq!(
        types(&events),
        [
            "SUBAGENT_STARTED",
            "TEXT_MESSAGE_START",
            "TEXT_MESSAGE_CONTENT",
            "TEXT_MESSAGE_END",
            "ACTIVITY_SNAPSHOT",
            "TEXT_MESSAGE_START",
            "TEXT_MESSAGE_CONTENT",
            "TEXT_MESSAGE_END",
            "SUBAGENT_FINISHED",
            "RUN_FINISHED",
        ],
        "inbound messages are the parent's, not the subagent's output"
    );
    let Event::SubagentStarted(started) = &events[0] else {
        panic!("expected SUBAGENT_STARTED");
    };
    assert_eq!(started.subagent_run_id, "task_a");
    assert_eq!(started.name, "Researcher");
    let wire = serde_json::to_string(&events).unwrap();
    assert!(
        !wire.contains("secret instructions"),
        "the spec stays private"
    );
    let Event::TextMessageContent(summary) = &events[6] else {
        panic!("expected the summary text");
    };
    assert_eq!(summary.delta, "Found it");
    assert_eq!(summary.subagent_run_id.as_deref(), Some("task_a"));
    let Event::SubagentFinished(finished) = &events[8] else {
        panic!("expected SUBAGENT_FINISHED");
    };
    assert_eq!(finished.outcome, None, "absent outcome is success");
}

#[test]
fn failed_and_cancelled_subagents_report_errors_through_the_policy() {
    let mut projector = Projector::new(
        "t",
        "r",
        ProjectionPolicy {
            error: std::sync::Arc::new(|_| RunErrorEvent::new("Something went wrong")),
            ..ProjectionPolicy::default()
        },
    );
    for id in ["task_f", "task_c"] {
        projector.project(
            "task.created",
            &task(id, "subagent", "foreground", "running", json!({})),
        );
    }
    projector.project(
        "task.updated",
        &task(
            "task_f",
            "subagent",
            "foreground",
            "failed",
            json!({ "error": { "kind": "provider_error", "message": "upstream key sk-1 rejected" } }),
        ),
    );
    projector.project(
        "task.updated",
        &task("task_c", "subagent", "foreground", "canceled", json!({})),
    );
    projector.project("turn.completed", &json!({}));
    let events: Vec<Event> = projector.drain().collect();
    assert_conformant(&events);
    let errors: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            Event::SubagentError(error) => Some(error),
            _ => None,
        })
        .collect();
    assert_eq!(errors.len(), 2);
    assert_eq!(
        errors[0].message, "Something went wrong",
        "sanitized by policy"
    );
    assert_eq!(errors[1].code.as_deref(), Some("cancelled"));
}

#[test]
fn background_subagent_open_at_run_end_is_suspended_after_an_activity() {
    let mut projector = Projector::new("t", "r", policy());
    projector.project(
        "task.created",
        &task("task_b", "subagent", "background", "running", json!({})),
    );
    projector.project(
        "task.created",
        &task("task_f", "subagent", "foreground", "running", json!({})),
    );
    projector.project("turn.cancelled", &json!({}));
    let events: Vec<Event> = projector.drain().collect();
    assert_conformant(&events);
    assert_eq!(
        types(&events),
        [
            "SUBAGENT_STARTED",
            "SUBAGENT_STARTED",
            "ACTIVITY_SNAPSHOT",
            "SUBAGENT_FINISHED",
            "SUBAGENT_FINISHED",
            "RUN_FINISHED",
        ]
    );
    for event in &events[3..5] {
        let Event::SubagentFinished(finished) = event else {
            panic!("expected SUBAGENT_FINISHED");
        };
        assert_eq!(
            finished.outcome,
            Some(SubagentFinishedOutcome::Suspended {
                interrupt_ids: None
            })
        );
    }
    // Updates after the run ended are ignored.
    projector.project(
        "task.updated",
        &task("task_b", "subagent", "background", "succeeded", json!({})),
    );
    assert_eq!(projector.drain().count(), 0);
}

#[test]
fn subagents_hidden_by_policy_other_kinds_and_settled_tasks_are_not_projected() {
    let mut hidden = Projector::new(
        "t",
        "r",
        ProjectionPolicy {
            subagents_visible: false,
            ..ProjectionPolicy::default()
        },
    );
    hidden.project(
        "task.created",
        &task("task_a", "subagent", "foreground", "running", json!({})),
    );
    hidden.project(
        "task.message.received",
        &task_message(
            "task_a",
            "tmsg_1",
            "outbound",
            json!([{ "type": "text", "text": "x" }]),
        ),
    );
    hidden.project("turn.completed", &json!({}));
    assert_eq!(types(&hidden.drain().collect::<Vec<_>>()), ["RUN_FINISHED"]);

    let mut projector = Projector::new("t", "r", policy());
    projector.project(
        "task.created",
        &task(
            "task_t",
            "background_tool",
            "background",
            "running",
            json!({}),
        ),
    );
    projector.project(
        "task.updated",
        &task(
            "task_old",
            "subagent",
            "background",
            "succeeded",
            json!({ "summary": "old" }),
        ),
    );
    projector.project(
        "task.message.received",
        &task_message(
            "task_old",
            "tmsg_1",
            "outbound",
            json!([{ "type": "text", "text": "x" }]),
        ),
    );
    projector.project("turn.completed", &json!({}));
    assert_eq!(
        types(&projector.drain().collect::<Vec<_>>()),
        ["RUN_FINISHED"]
    );
}

#[test]
fn run_metadata_names_turn_model_and_only_a_given_session() {
    let mut projector = Projector::new("t", "r", policy());
    assert_eq!(projector.run_metadata(), None, "nothing known yet");
    projector.observe_turn("turn_1");
    projector.project("llm.generation", &generation("openai", "gpt-5", 10, 5, 0));
    projector.project("turn.completed", &json!({}));
    let events: Vec<Event> = projector.drain().collect();
    assert_conformant(&events);
    let Some(Event::RunFinished(finished)) = events.last() else {
        panic!("expected RUN_FINISHED");
    };
    assert_eq!(
        serde_json::to_value(&finished.base.metadata).unwrap(),
        json!({ "everruns": { "turnId": "turn_1", "model": "gpt-5" } })
    );

    let mut public = Projector::new(
        "t",
        "r",
        ProjectionPolicy {
            model_visible: false,
            session_id: Some("session_1".into()),
            ..ProjectionPolicy::default()
        },
    );
    public.observe_turn("turn_2");
    public.project("llm.generation", &generation("openai", "gpt-5", 10, 5, 0));
    public.project("turn.failed", &json!({ "error": "boom" }));
    let events: Vec<Event> = public.drain().collect();
    assert_conformant(&events);
    let Some(Event::RunError(error)) = events.last() else {
        panic!("expected RUN_ERROR");
    };
    assert_eq!(
        serde_json::to_value(&error.base.metadata).unwrap(),
        json!({ "everruns": { "sessionId": "session_1", "turnId": "turn_2" } })
    );
}
