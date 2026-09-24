//! Unit tests for [`super`], the session event stream.
//!
//! Split out of `events.rs` to keep it under the file-size ratchet.

use std::sync::Arc;

use everruns_core::event_emitter::EventEmitter;
use everruns_core::events::{
    ActStartedData, OutputMessageDeltaData, OutputMessageReplacedData, ToolStartedData,
    TurnCancelledData, TurnStartedData,
};
use everruns_host::{HostEventEmitter, InMemoryEventLog};
use everruns_provider::tool_types::ToolCall;
use everruns_provider::typed_id::{MessageId, SessionId, TurnId};
use serde_json::json;

use super::{EventStreamError, FacadeEventBus, SessionEvent, SessionEventKind};
use crate::{Agent, InMemoryEngine, Model};

fn host(bus: Arc<FacadeEventBus>) -> HostEventEmitter {
    HostEventEmitter::new(Arc::new(InMemoryEventLog::new()), bus)
}

fn turn_started(session_id: SessionId, turn_id: TurnId) -> everruns_core::EventRequest {
    let input_message_id = MessageId::new();
    everruns_core::EventRequest::new(
        session_id,
        everruns_core::EventContext::turn(turn_id, input_message_id),
        TurnStartedData {
            turn_id,
            input_message_id,
            input_content: Some("hello".to_string()),
            agent_id: None,
            agent_name: None,
            agent_description: None,
        },
    )
}

#[test]
fn reviewed_data_never_exceeds_the_canonical_payload() {
    // The reviewed projection must be a *subset* of what the runtime
    // emitted: it may drop fields, never invent them. A synthesised field
    // would be a value no consumer could correlate with the canonical
    // record, and would quietly become API nobody reviewed.
    let canonical = json!({
        "tool_call_id": "call_1",
        "tool_name": "lookup",
        "success": true,
        "status": "success",
        "result": ["secret"],
        "narration": "Looking it up",
        "display_name": "Knowledge lookup",
    });
    let kind = SessionEventKind::ToolCompleted {
        tool_call_id: "call_1".to_string(),
        tool_name: "lookup".to_string(),
        success: true,
    };

    let reviewed = SessionEvent::reviewed_data(&kind, &canonical);
    let reviewed = reviewed.as_object().expect("reviewed data is an object");

    for (key, value) in reviewed {
        assert_eq!(
            Some(value),
            canonical.get(key),
            "reviewed key {key} is absent from or differs in the canonical payload"
        );
    }
    // Promoted identity and outcome survive; the tool's result does not.
    assert_eq!(reviewed["tool_call_id"], "call_1");
    assert_eq!(reviewed["narration"], "Looking it up");
    assert!(!reviewed.contains_key("result"));
}

#[test]
fn a_cancellation_field_nobody_promoted_stays_off_the_reviewed_surface() {
    // `TurnCancelled` is the one kind whose reviewed payload comes entirely
    // from the event data rather than from variant fields, so it is the one
    // most likely to drift back into a wholesale clone. A field added to the
    // cancellation payload inside the runtime must not become public API
    // just by existing.
    let canonical = json!({
        "turn_id": "turn_1",
        "reason": "user cancelled",
        "usage": { "input_tokens": 12, "output_tokens": 3 },
        "partial_output": "the model had written this far",
    });

    let reviewed = SessionEvent::reviewed_data(&SessionEventKind::TurnCancelled, &canonical);

    assert_eq!(reviewed["turn_id"], "turn_1");
    assert_eq!(reviewed["reason"], "user cancelled");
    assert_eq!(reviewed["usage"]["input_tokens"], 12);
    assert!(
        reviewed.get("partial_output").is_none(),
        "an unpromoted cancellation field must not reach the reviewed surface"
    );
}

#[tokio::test]
async fn envelope_is_complete_while_data_stays_reviewed() {
    let session_id = SessionId::new();
    let turn_id = TurnId::new();
    let bus = Arc::new(FacadeEventBus::new());
    let emitter = host(bus.clone());
    let mut stream = bus.subscribe();

    let request = turn_started(session_id, turn_id)
        .with_metadata(json!({"provider": "simulated"}))
        .with_tags(vec!["terminal".to_string()]);
    let canonical = emitter.emit(request).await.expect("event emits");
    let observed = stream
        .recv()
        .await
        .expect("stream remains lossless")
        .expect("event is delivered");

    assert!(matches!(observed.kind, SessionEventKind::TurnStarted));

    // `turn.started` carries the user's prompt in `data.input_content`.
    // `TurnStarted` promotes nothing, so the reviewed surface omits it —
    // this is the leak the split exists to close, since an application
    // forwarding these envelopes would otherwise ship the prompt with them.
    assert_eq!(observed.data(), &json!({}));
    assert!(!observed.as_json().to_string().contains("hello"));

    // Nothing is lost: the canonical envelope is byte-for-byte the event
    // the runtime emitted, prompt included.
    assert_eq!(
        observed.canonical_json(),
        &serde_json::to_value(canonical).expect("canonical event serializes")
    );
    assert_eq!(observed.canonical_json()["data"]["input_content"], "hello");

    // Envelope fields stay complete on the reviewed form; only `data` differs.
    assert_eq!(observed.event_type(), "turn.started");
    assert_eq!(
        observed.turn_id.as_deref(),
        Some(turn_id.to_string().as_str())
    );
    assert_eq!(observed.as_json()["sequence"], 1);
    assert_eq!(observed.sequence(), Some(1));
    assert!(!observed.timestamp().is_empty());
    assert_eq!(observed.as_json()["metadata"]["provider"], "simulated");
    assert_eq!(observed.as_json()["tags"], json!(["terminal"]));
}

#[tokio::test]
async fn bounded_stream_reports_lag_instead_of_hiding_loss() {
    let session_id = SessionId::new();
    let bus = Arc::new(FacadeEventBus::with_capacity(2));
    let emitter = host(bus.clone());
    let mut stream = bus.subscribe();

    for _ in 0..3 {
        emitter
            .emit(turn_started(session_id, TurnId::new()))
            .await
            .expect("event emits without observer backpressure");
    }

    assert!(matches!(
        stream.recv().await,
        Err(EventStreamError::Lagged { missed: 1 })
    ));
    assert!(stream.recv().await.expect("gap reported").is_some());
}

#[tokio::test]
async fn subscriber_after_earlier_events_still_observes_later_ones() {
    // The broadcast channel is allocated on the first subscribe, so this
    // pins the semantics that laziness relies on: events emitted before
    // anyone subscribed are not replayed, and later events still arrive.
    let session_id = SessionId::new();
    let bus = Arc::new(FacadeEventBus::new());
    let emitter = host(bus.clone());

    emitter
        .emit(turn_started(session_id, TurnId::new()))
        .await
        .expect("emitting without a subscriber succeeds");

    let mut stream = bus.subscribe();
    let turn_id = TurnId::new();
    emitter
        .emit(turn_started(session_id, turn_id))
        .await
        .expect("emitting to a live subscriber succeeds");

    let observed = stream
        .recv()
        .await
        .expect("stream stays open")
        .expect("the event emitted after subscribing arrives");
    assert_eq!(
        observed.turn_id.as_deref(),
        Some(turn_id.to_string().as_str())
    );
}

#[tokio::test]
async fn no_subscriber_is_a_noop_not_a_closed_sink_failure() {
    let bus = Arc::new(FacadeEventBus::new());
    let emitter = host(bus);

    emitter
        .emit(turn_started(SessionId::new(), TurnId::new()))
        .await
        .expect("observation absence cannot reverse the append");

    assert_eq!(emitter.delivery_stats().closed, 0);
}

#[tokio::test]
async fn live_arrival_interleaves_durable_and_sequence_less_ephemeral_events() {
    let session_id = SessionId::new();
    let turn_id = TurnId::new();
    let message_id = MessageId::new();
    let bus = Arc::new(FacadeEventBus::new());
    let emitter = host(bus.clone());
    let mut stream = bus.subscribe();

    emitter
        .emit(turn_started(session_id, turn_id))
        .await
        .unwrap();
    emitter
        .emit(everruns_core::EventRequest::new(
            session_id,
            everruns_core::EventContext::turn(turn_id, message_id),
            OutputMessageDeltaData {
                turn_id,
                message_id,
                delta: "hi".to_string(),
                accumulated: "hi".to_string(),
                phase: None,
            },
        ))
        .await
        .unwrap();
    emitter
        .emit(everruns_core::EventRequest::new(
            session_id,
            everruns_core::EventContext::turn(turn_id, message_id),
            TurnCancelledData {
                turn_id,
                reason: Some("test".to_string()),
                usage: None,
            },
        ))
        .await
        .unwrap();

    let started = stream.recv().await.unwrap().unwrap();
    let delta = stream.recv().await.unwrap().unwrap();
    let cancelled = stream.recv().await.unwrap().unwrap();

    assert_eq!(started.event_type(), "turn.started");
    assert_eq!(delta.event_type(), "output.message.delta");
    assert_eq!(delta.data()["delta"], "hi");
    assert!(delta.data().get("accumulated").is_none());
    assert!(delta.as_json()["data"].get("accumulated").is_none());
    assert_eq!(cancelled.event_type(), "turn.cancelled");
    assert_eq!(started.sequence(), Some(1));
    assert_eq!(delta.sequence(), None);
    assert!(delta.as_json().get("sequence").is_none());
    assert_eq!(cancelled.sequence(), Some(2));
}

#[tokio::test]
async fn cancellation_uses_the_active_turn_and_canonical_sequence() {
    let session_id = SessionId::new();
    let turn_id = TurnId::new();
    let bus = Arc::new(FacadeEventBus::new());
    let emitter = host(bus.clone());
    let mut stream = bus.subscribe();
    emitter
        .emit(turn_started(session_id, turn_id))
        .await
        .expect("turn starts");

    let (cancelled_turn_id, request) = bus.cancellation_request(session_id);
    emitter.emit(request).await.expect("cancellation commits");
    assert_eq!(cancelled_turn_id, turn_id);

    let started = stream.recv().await.expect("no lag").expect("start event");
    let cancelled = stream.recv().await.expect("no lag").expect("cancel event");
    assert!(matches!(started.kind, SessionEventKind::TurnStarted));
    assert!(matches!(cancelled.kind, SessionEventKind::TurnCancelled));
    assert_eq!(
        cancelled.turn_id.as_deref(),
        Some(turn_id.to_string().as_str())
    );
    assert_eq!(cancelled.as_json()["sequence"], 2);
    assert_eq!(
        cancelled.data(),
        &serde_json::to_value(TurnCancelledData {
            turn_id,
            reason: Some("cancelled by application".to_string()),
            usage: None,
        })
        .expect("cancel data serializes")
    );
}

#[tokio::test]
async fn output_replacement_retains_rebuildable_message_identity_and_text() {
    let session_id = SessionId::new();
    let turn_id = TurnId::new();
    let message_id = MessageId::new();
    let bus = Arc::new(FacadeEventBus::new());
    let emitter = host(bus.clone());
    let mut stream = bus.subscribe();
    emitter
        .emit(everruns_core::EventRequest::new(
            session_id,
            everruns_core::EventContext {
                turn_id: Some(turn_id),
                ..everruns_core::EventContext::default()
            },
            OutputMessageReplacedData {
                turn_id,
                message_id,
                guardrail_capability_id: "guardrails".to_string(),
                guardrail_id: "output-policy".to_string(),
                reason_code: "blocked".to_string(),
                replacement: "Response withheld.".to_string(),
            },
        ))
        .await
        .expect("replacement emits");

    let replacement = stream
        .recv()
        .await
        .expect("no lag")
        .expect("replacement delivered");
    assert!(matches!(
        &replacement.kind,
        SessionEventKind::OutputReplaced {
            message_id: observed_id,
            replacement,
        } if observed_id == &message_id.to_string() && replacement == "Response withheld."
    ));
    assert_eq!(replacement.data()["message_id"], message_id.to_string());
    assert_eq!(replacement.data()["replacement"], "Response withheld.");
}

#[tokio::test]
async fn tool_narration_is_preserved_for_renderers() {
    let session_id = SessionId::new();
    let turn_id = TurnId::new();
    let bus = Arc::new(FacadeEventBus::new());
    let emitter = host(bus.clone());
    let mut stream = bus.subscribe();

    emitter
        .emit(everruns_core::EventRequest::new(
            session_id,
            everruns_core::EventContext {
                turn_id: Some(turn_id),
                ..everruns_core::EventContext::default()
            },
            ToolStartedData {
                tool_call: ToolCall {
                    id: "call_1".to_string(),
                    name: "lookup".to_string(),
                    arguments: json!({"key": "answer"}),
                },
                tool_call_fingerprint: None,
                display_name: Some("Knowledge lookup".to_string()),
                narration: Some("Looking up the answer".to_string()),
            },
        ))
        .await
        .expect("tool start emits");

    let observed = stream.recv().await.unwrap().unwrap();
    assert_eq!(observed.narration(), Some("Looking up the answer"));
    assert_eq!(observed.data()["display_name"], "Knowledge lookup");
}

#[tokio::test]
async fn unpromoted_event_kind_is_identified_but_not_projected() {
    let session_id = SessionId::new();
    let turn_id = TurnId::new();
    let bus = Arc::new(FacadeEventBus::new());
    let emitter = host(bus.clone());
    let mut stream = bus.subscribe();
    let canonical = emitter
        .emit(everruns_core::EventRequest::new(
            session_id,
            everruns_core::EventContext {
                turn_id: Some(turn_id),
                ..everruns_core::EventContext::default()
            },
            ActStartedData {
                tool_calls: Vec::new(),
                headline: Some("running tools".to_string()),
            },
        ))
        .await
        .expect("event emits");

    let observed = stream.recv().await.unwrap().unwrap();

    // Identified, so a renderer can still route it.
    assert!(matches!(
        &observed.kind,
        SessionEventKind::Other { event_type } if event_type == "act.started"
    ));

    // Not projected: an event type this version does not recognize has by
    // definition not been reviewed, so its payload stays off the reviewed
    // surface instead of becoming public API by accident.
    assert_eq!(observed.data(), &json!({}));
    assert!(!observed.as_json().to_string().contains("running tools"));

    // But nothing is lost — the complete envelope is one explicit call away.
    assert_eq!(
        observed.canonical_json(),
        &serde_json::to_value(canonical).expect("canonical event serializes")
    );
    assert_eq!(
        observed.canonical_json()["data"]["headline"],
        "running tools"
    );
}

#[tokio::test]
async fn provider_failure_retains_reason_and_turn_terminal_payloads() {
    let agent = Agent::builder()
        .instructions("Answer concisely.")
        .model(Model::simulated_error("provider unavailable"))
        .build()
        .expect("valid agent");
    let session = InMemoryEngine::new().create(agent.clone());
    let mut stream = session.events();
    let result = session
        .run("hello")
        .await
        .expect("provider failure resolves to a failed turn");
    assert!(!result.success);
    drop(session);

    let mut observed = Vec::new();
    while let Some(event) = stream.recv().await.expect("failure stream does not lag") {
        observed.push(event);
    }

    let reason_failure = observed
        .iter()
        .find(|event| {
            matches!(
                event.kind,
                SessionEventKind::ReasonCompleted { success: false, .. }
            )
        })
        .expect("reason.completed preserves the provider failure");
    assert!(
        reason_failure.data()["error"]
            .as_str()
            .is_some_and(|error| error.contains("provider unavailable"))
    );

    let turn_failure = observed
        .iter()
        .find(|event| matches!(event.kind, SessionEventKind::TurnFailed { .. }))
        .expect("turn.failed is the terminal event");
    assert_eq!(turn_failure.event_type(), "turn.failed");
    assert_eq!(
        turn_failure.turn_id.as_deref(),
        Some(result.turn_id.as_str())
    );
    assert!(turn_failure.data()["error"].as_str().is_some());
}

#[tokio::test]
async fn tool_lifecycle_keeps_order_and_reaches_arguments_canonically() {
    let tool = crate::FunctionTool::new(
        "lookup",
        "Look up a value.",
        json!({
            "type": "object",
            "properties": { "key": { "type": "string" } },
            "required": ["key"]
        }),
        |arguments: serde_json::Value| async move {
            Ok::<_, String>(json!({ "value": arguments["key"] }))
        },
    );
    let agent = Agent::builder()
        .instructions("Use the lookup tool.")
        .model(Model::simulated_scripted(
            "done",
            vec![
                vec![ToolCall {
                    id: "call_lookup_1".to_string(),
                    name: "lookup".to_string(),
                    arguments: json!({ "key": "answer" }),
                }],
                vec![],
            ],
        ))
        .tool(tool)
        .build()
        .expect("valid agent");
    let session = InMemoryEngine::new().create(agent.clone());
    let mut stream = session.events();
    let result = session.run("look it up").await.expect("tool turn runs");
    assert!(result.success);
    drop(session);

    let mut observed = Vec::new();
    while let Some(event) = stream.recv().await.expect("tool stream does not lag") {
        observed.push(event);
    }
    let started = observed
        .iter()
        .find(|event| matches!(event.kind, SessionEventKind::ToolStarted { .. }))
        .expect("tool.started");
    let completed = observed
        .iter()
        .find(|event| matches!(event.kind, SessionEventKind::ToolCompleted { .. }))
        .expect("tool.completed");

    assert!(
        started.sequence().expect("tool start is durable")
            < completed.sequence().expect("tool completion is durable")
    );
    // Identity and outcome are promoted, so a timeline renders from the
    // reviewed surface alone.
    assert_eq!(started.data()["tool_call_id"], "call_lookup_1");
    assert_eq!(completed.data()["tool_call_id"], "call_lookup_1");
    assert_eq!(completed.data()["success"], true);

    // Arguments and results are the payload most worth not leaking by
    // default — a tool call can carry credentials in, and file contents out.
    assert!(started.data()["tool_call"].is_null());
    assert!(completed.data()["result"].is_null());

    // Still reachable for an auditor or recorder that asks explicitly.
    assert_eq!(
        started.canonical_json()["data"]["tool_call"]["arguments"]["key"],
        "answer"
    );
    assert_eq!(completed.canonical_json()["data"]["status"], "success");
    assert!(completed.canonical_json()["data"]["result"].is_array());
}
