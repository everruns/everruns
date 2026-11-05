//! An external host drives an Agent's turns over its own canonical log, and a
//! turn cut off mid-act finishes on a runtime rebuilt from that log alone.

use std::sync::Arc;
use std::time::Duration;

use everruns::hosting::runtime::{AcceptedTurnInput, HostBackends, TurnId, TurnSteering};
use everruns::prelude::*;
use everruns::{LlmSimConfig, SimToolCall, SimTurn};
use everruns_core::host::{EventReadLimit, EventReadRequest, EventReader, InMemoryEventLog};
use serde_json::{Value, json};

fn call(name: &str, id: &str) -> SimTurn {
    SimTurn::ToolCalls(vec![SimToolCall {
        name: name.into(),
        arguments: json!({}),
        id: Some(id.into()),
    }])
}

async fn event_types(log: &InMemoryEventLog, session: SessionId) -> Vec<String> {
    let page = log
        .read_page(EventReadRequest::new(session, EventReadLimit::default()))
        .await
        .expect("read log");
    page.events.into_iter().map(|e| e.event_type).collect()
}

#[tokio::test]
async fn a_turn_runs_over_a_host_supplied_log() {
    let agent = Agent::builder()
        .instructions("Answer.")
        .model(Model::simulated("Hello from the host log."))
        .build()
        .expect("agent");
    let log = Arc::new(InMemoryEventLog::new());
    let session = SessionId::new();
    let runtime = agent
        .external_runtime(
            HostBackends::in_memory().with_event_log(log.clone()),
            session,
        )
        .await
        .expect("runtime");

    let input = AcceptedTurnInput::new("hi");
    let message = runtime
        .persist_accepted_input(session, input)
        .await
        .expect("admit");
    let turn = runtime
        .run_stored_turn(session, message, TurnId::new(), TurnSteering::new())
        .await
        .expect("turn");

    assert!(turn.success, "{:?}", turn.error);
    assert_eq!(turn.response, "Hello from the host log.");
    let types = event_types(&log, session).await;
    assert!(types.iter().any(|t| t == "input.message"), "{types:?}");
    assert!(types.iter().any(|t| t == "turn.completed"), "{types:?}");
}

#[tokio::test]
async fn a_turn_cut_off_mid_act_finishes_on_a_rebuilt_runtime() {
    // A tool that never returns stands in for a process that dies mid-act.
    let hang = FunctionTool::new(
        "deploy",
        "Deploy.",
        json!({ "type": "object", "properties": {} }),
        |_args: Value| async move {
            std::future::pending::<()>().await;
            Ok::<_, String>(json!({}))
        },
    );
    let sim = LlmSimConfig::scripted(vec![
        call("deploy", "call_1"),
        SimTurn::Assistant("The deploy was interrupted.".into()),
    ]);
    let agent = Agent::builder()
        .instructions("Deploy, then report.")
        .model(Model::simulated_with_config(sim))
        .tool(hang)
        .build()
        .expect("agent");
    let log = Arc::new(InMemoryEventLog::new());
    let session = SessionId::new();

    // First holder: admits the input, starts the turn, and is dropped while
    // the tool runs. Only the log survives.
    {
        let runtime = agent
            .external_runtime(
                HostBackends::in_memory().with_event_log(log.clone()),
                session,
            )
            .await
            .expect("runtime");
        let message = runtime
            .persist_accepted_input(session, AcceptedTurnInput::new("ship it"))
            .await
            .expect("admit");
        let cut = tokio::time::timeout(
            Duration::from_millis(500),
            runtime.run_stored_turn(session, message, TurnId::new(), TurnSteering::new()),
        )
        .await;
        assert!(cut.is_err(), "the hung tool holds the turn");
    }
    let before = event_types(&log, session).await;
    assert!(before.iter().any(|t| t == "tool.started"), "{before:?}");
    assert!(!before.iter().any(|t| t == "turn.completed"), "{before:?}");

    // Second holder: a fresh runtime over the same log continues the turn.
    let runtime = agent
        .external_runtime(
            HostBackends::in_memory().with_event_log(log.clone()),
            session,
        )
        .await
        .expect("rebuilt runtime");
    let turn = runtime
        .resume_interrupted_turn(session, TurnSteering::new())
        .await
        .expect("resume");

    assert!(turn.success, "{:?}", turn.error);
    assert_eq!(turn.response, "The deploy was interrupted.");
    let after = event_types(&log, session).await;
    assert_eq!(
        after.iter().filter(|t| *t == "turn.started").count(),
        1,
        "the turn keeps its identity: {after:?}"
    );
    assert!(after.iter().any(|t| t == "turn.completed"), "{after:?}");
    // Rebuilding the runtime starts no second session.
    assert_eq!(
        after.iter().filter(|t| *t == "session.started").count(),
        1,
        "{after:?}"
    );
    // The hung call is settled as interrupted, never run a second time.
    assert_eq!(
        after.iter().filter(|t| *t == "tool.started").count(),
        1,
        "{after:?}"
    );
}
