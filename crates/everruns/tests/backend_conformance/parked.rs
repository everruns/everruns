//! A turn parks on a client-side tool call; the call's result resumes it.
//!
//! Through a session, parking and resuming are AG-UI runs: the first run ends
//! with the parked call pending, the next run's tool message is its result.
//! The facade does not hand back the resumed turn's result, so the seam
//! scenario below starts the same turns directly on each `TurnBackend` and
//! compares their `TurnResult`s too.

use std::sync::Arc;
use std::time::Duration;

use everruns::ag_ui::wire::RunFinishedOutcome;
use everruns::ag_ui::{Event, Message, RunAgentInput};
use everruns::{Agent, LlmSimConfig, Model, Session, ToolCall};
use everruns_contracts::tool_types::{ClientSideTool, ToolDefinition};
use everruns_contracts::typed_id::{AgentId, HarnessId, SessionId, TurnId};
use everruns_core::events::ToolCompletedData;
use everruns_core::host::{
    AcceptedTurnInput, AgentBuilder, HarnessBuilder, InProcessBackend, InProcessRuntime,
    SessionBuilder, TurnBackend, TurnInput, TurnRequest, TurnResult, TurnTicket,
};
use everruns_durable_engine::DurableBackend;
use everruns_llmsim::LlmSimRuntimeExt;
use futures::StreamExt;
use serde_json::{Value, json};

use crate::support::{BACKENDS, BackendKind, Observed, has_event, run_on};

/// Calls the client-side `confirm` tool, then answers.
fn confirming_model() -> LlmSimConfig {
    LlmSimConfig::fixed("Confirmed.").with_tool_call_sequence(vec![
        vec![ToolCall {
            id: "call_confirm".to_string(),
            name: "confirm".to_string(),
            arguments: json!({ "what": "deploy" }),
        }],
        vec![],
    ])
}

// --- Through a session's AG-UI runs ---------------------------------------------

fn confirm_tool() -> everruns::ag_ui::wire::Tool {
    serde_json::from_value(json!({
        "name": "confirm",
        "description": "Ask the person to confirm in the page.",
        "parameters": { "type": "object", "properties": { "what": { "type": "string" } } },
    }))
    .expect("valid tool")
}

fn run_input(run_id: &str, messages: Vec<Message>) -> RunAgentInput {
    RunAgentInput {
        thread_id: "thread-1".into(),
        run_id: run_id.into(),
        messages,
        tools: vec![confirm_tool()],
        ..RunAgentInput::default()
    }
}

fn tool_result(id: &str, call_id: &str, content: &str) -> Message {
    serde_json::from_value(json!({
        "id": id,
        "role": "tool",
        "toolCallId": call_id,
        "content": content,
    }))
    .expect("valid tool message")
}

async fn run(session: &Session, input: RunAgentInput) -> Vec<Event> {
    let stream = session.ag_ui(input).await.expect("run starts");
    tokio::time::timeout(Duration::from_secs(10), stream.collect())
        .await
        .expect("run ends")
}

/// The run's AG-UI event types, its streamed text, and how it finished.
fn summarize(events: &[Event]) -> String {
    let types: Vec<String> = events
        .iter()
        .map(|event| {
            serde_json::to_value(event).expect("event serializes")["type"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    let text: String = events
        .iter()
        .filter_map(|event| match event {
            Event::TextMessageContent(content) => Some(content.delta.as_str()),
            _ => None,
        })
        .collect();
    let finished = match events.last() {
        Some(Event::RunFinished(finished)) => match &finished.outcome {
            Some(RunFinishedOutcome::Success {
                pending_tool_call_ids,
            }) => format!("pending {pending_tool_call_ids:?}"),
            other => format!("{other:?}"),
        },
        other => panic!("a run ends with RUN_FINISHED, got {other:?}"),
    };
    format!("{types:?} text={text:?} finished={finished}")
}

#[tokio::test]
async fn a_session_parks_on_a_client_call_and_its_result_resumes_the_turn() {
    let outcome = run_on(
        || {
            let agent = Agent::builder()
                .instructions("Confirm before deploying.")
                .model(Model::simulated_with_config(confirming_model()))
                .build()
                .expect("valid agent");
            (agent, ())
        },
        |_, session, ()| async move {
            let parked = run(
                &session,
                run_input("run-1", vec![Message::user("m1", "Deploy the service.")]),
            )
            .await;
            let resumed = run(
                &session,
                run_input(
                    "run-2",
                    vec![
                        Message::user("m1", "Deploy the service."),
                        tool_result("t1", "call_confirm", "{\"confirmed\":true}"),
                    ],
                ),
            )
            .await;
            Observed::default()
                .note(summarize(&parked))
                .note(summarize(&resumed))
        },
    )
    .await;
    assert!(
        outcome.notes[0].ends_with(r#"finished=pending Some(["call_confirm"])"#),
        "{:?}",
        outcome.notes
    );
    assert!(
        outcome.notes[1].contains(r#"text="Confirmed.""#),
        "{:?}",
        outcome.notes
    );
    assert!(has_event(&outcome, "tool.completed"), "{outcome:?}");
}

// --- Directly on the backend seam -----------------------------------------------

/// A runtime whose one session has the client-side `confirm` tool and a client
/// that answers a pause (the `setup_connection` hint).
async fn client_tool_runtime() -> (InProcessRuntime, SessionId) {
    let harness_id = HarnessId::new();
    let agent_id = AgentId::new();
    let session_id = SessionId::new();
    let mut session = SessionBuilder::new(harness_id)
        .id(session_id)
        .agent(agent_id)
        .tool(ToolDefinition::ClientSide(ClientSideTool::new(
            "confirm",
            "Ask the person to confirm.",
            json!({ "type": "object" }),
        )))
        .build();
    session.hints = Some([("setup_connection".to_string(), json!(true))].into());
    let runtime = InProcessRuntime::builder()
        .harness(
            HarnessBuilder::new("client", "Confirm before deploying.")
                .id(harness_id)
                .build(),
        )
        .agent(
            AgentBuilder::new("deployer", "Use tools when needed.")
                .id(agent_id)
                .build(),
        )
        .session(session)
        .llm_sim_as_default(confirming_model())
        .build()
        .await
        .expect("runtime builds");
    (runtime, session_id)
}

/// A backend over `runtime`, and what keeps it running.
fn seam_backend(
    kind: BackendKind,
    runtime: &InProcessRuntime,
    session_id: SessionId,
) -> (Arc<dyn TurnBackend>, Option<DurableBackend>) {
    match kind {
        BackendKind::InProcess => (Arc::new(InProcessBackend::new(runtime.clone())), None),
        BackendKind::DurableMemory => {
            let durable = DurableBackend::memory(2);
            let session = durable.attach(session_id, runtime.clone());
            (Arc::new(session), Some(durable))
        }
    }
}

/// The comparable part of a `TurnResult`; its turn id differs per run.
fn shape(result: &TurnResult) -> Value {
    json!({
        "response": result.response,
        "iterations": result.iterations,
        "tool_calls": result.tool_calls_count,
        "success": result.success,
        "error": result.error,
        "stop_reason": result.stop_reason,
    })
}

#[tokio::test]
async fn a_backend_parks_a_turn_and_tool_results_resume_it() {
    let mut outcomes = Vec::new();
    for kind in BACKENDS {
        let (runtime, session_id) = client_tool_runtime().await;
        let (backend, _durable) = seam_backend(kind, &runtime, session_id);
        let finish = |ticket: TurnTicket| async move {
            tokio::time::timeout(Duration::from_secs(10), ticket)
                .await
                .unwrap_or_else(|_| panic!("{kind:?}: the turn ends"))
                .unwrap_or_else(|error| panic!("{kind:?}: the turn succeeds: {error}"))
        };

        let request = TurnRequest::new(
            session_id,
            TurnId::new(),
            TurnInput::Message(Box::new(AcceptedTurnInput::new(
                everruns_core::InputMessage::user("Deploy."),
            ))),
        );
        let parked: TurnResult = finish(backend.start_turn(request).await.expect("starts")).await;
        let calls = runtime
            .parked_tool_calls(session_id)
            .unwrap_or_else(|| panic!("{kind:?}: the turn parked on the client-side call"));
        assert_eq!(calls.turn_id, parked.turn_id, "{kind:?}");
        let call_ids: Vec<String> = calls
            .tool_calls
            .iter()
            .map(|call| call.id.clone())
            .collect();

        let results = vec![ToolCompletedData::success(
            "call_confirm".to_string(),
            "confirm".to_string(),
            vec![everruns_core::ContentPart::text("confirmed")],
            None,
        )];
        let request = TurnRequest::new(session_id, calls.turn_id, TurnInput::ToolResults(results));
        let resumed: TurnResult = finish(backend.start_turn(request).await.expect("resumes")).await;
        assert_eq!(resumed.turn_id, parked.turn_id, "{kind:?}: the same turn");
        assert!(runtime.parked_tool_calls(session_id).is_none(), "{kind:?}");
        assert!(!backend.is_running(session_id).await, "{kind:?}");

        let event_types: Vec<String> = runtime
            .events()
            .await
            .expect("events")
            .into_iter()
            .map(|event| event.data.event_type().to_string())
            .collect();
        outcomes.push((
            kind,
            json!({
                "parked": shape(&parked),
                "parked_calls": call_ids,
                "resumed": shape(&resumed),
                "event_types": event_types,
            }),
        ));
    }
    let (_, in_process) = outcomes.remove(0);
    for (kind, outcome) in outcomes {
        assert_eq!(outcome, in_process, "{kind:?} diverges from in process");
    }
    assert_eq!(in_process["parked_calls"], json!(["call_confirm"]));
    assert_eq!(in_process["parked"]["iterations"], 1);
    assert_eq!(in_process["parked"]["tool_calls"], 1);
    assert_eq!(in_process["resumed"]["response"], "Confirmed.");
    assert_eq!(in_process["resumed"]["iterations"], 1);
    assert_eq!(in_process["resumed"]["tool_calls"], 0);
}
