//! A turn cut off in its act, then resumed from the session log.
//!
//! The cut: the tool's first run waits forever. Dropping frontend handles
//! leaves it running; aborting the independent executor drops the turn mid-act.
//! Its lease must expire before a replacement can claim it. The log keeps a turn with a tool call and no
//! result, exactly what a process exit leaves. The engine reopens the session, and `resume_interrupted_turn` runs
//! the unfinished call again when its tool is idempotent, or records it as
//! interrupted when it is not, and lets the turn carry on.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use everruns::{Agent, FunctionTool, LlmSimConfig, Model, ToolCall};
use serde_json::json;
use tokio::sync::Notify;

use crate::support::{Observed, Outcome, TurnShape, has_event};
#[path = "../support/session_hosting.rs"]
mod session_hosting;
use session_hosting::External;

/// Observes the tool across the cut.
#[derive(Clone, Default)]
struct Probe {
    runs: Arc<AtomicUsize>,
    /// The first run is waiting.
    entered: Arc<Notify>,
    /// The first run was dropped.
    dropped: Arc<Notify>,
}

struct NotifyOnDrop(Arc<Notify>);

impl Drop for NotifyOnDrop {
    fn drop(&mut self) {
        self.0.notify_one();
    }
}

fn agent_with_waiting_tool(idempotent: bool) -> (Agent, Probe) {
    let probe = Probe::default();
    let tool_probe = probe.clone();
    let mut tool = FunctionTool::new(
        "wait_for_approval",
        "Wait until a person approves the deploy.",
        json!({ "type": "object", "properties": {} }),
        move |_args: serde_json::Value| {
            let probe = tool_probe.clone();
            async move {
                if probe.runs.fetch_add(1, Ordering::SeqCst) == 0 {
                    let _dropped = NotifyOnDrop(probe.dropped.clone());
                    probe.entered.notify_one();
                    std::future::pending::<()>().await;
                }
                Ok::<_, String>(json!({ "approved": true }))
            }
        },
    );
    if idempotent {
        tool = tool.idempotent();
    }
    let model = Model::simulated_with_config(
        LlmSimConfig::fixed("Deployed.").with_tool_call_sequence(vec![
            vec![ToolCall {
                id: "call_approval".into(),
                name: "wait_for_approval".into(),
                arguments: json!({}),
            }],
            vec![],
        ]),
    );
    let agent = Agent::builder()
        .instructions("Wait for approval before deploying.")
        .model(model)
        .tool(tool)
        .build()
        .expect("valid agent");
    (agent, probe)
}

/// Cut the turn off in its act, resume it, and note what happened.
async fn cut_off_and_resume(idempotent: bool) -> Outcome {
    let profile = everruns::hosting::MemoryHosting::default();
    let build = everruns::hosting::AgentBuild::new("interrupted", "v1");
    let (agent, probe) = agent_with_waiting_tool(idempotent);
    let make = || {
        everruns::Engine::builder()
            .hosting(External(profile.clone()))
            .register(build.clone(), agent.clone())
            .build()
    };
    let engine = make();
    let session_id = everruns::SessionId::new();
    let session = engine.create_registered(session_id, &build).await.unwrap();
    let sent = session.send("Deploy.").await.unwrap();
    let executor = tokio::spawn({
        let engine = engine.clone();
        async move { engine.activate(session_id).await }
    });
    probe.entered.notified().await;
    assert!(
        session.interrupted_turn().await.unwrap().is_none(),
        "a live epoch is not interrupted"
    );
    let message = sent.message_id.parse().unwrap();
    drop(sent);
    drop(session);
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(10),
            probe.dropped.notified()
        )
        .await
        .is_err(),
        "dropping a frontend must not cancel accepted work"
    );
    executor.abort();
    let _ = executor.await;
    probe.dropped.notified().await;
    drop(engine);
    tokio::time::advance(std::time::Duration::from_secs(11)).await;
    let replacement = make();
    let session = replacement.resume(session_id).await.unwrap();
    let interrupted = session
        .interrupted_turn()
        .await
        .unwrap()
        .expect("expired executor leaves interrupted work");
    let ids =
        |calls: &[ToolCall]| -> Vec<String> { calls.iter().map(|call| call.id.clone()).collect() };
    let calls = ids(&interrupted.tool_calls);
    let not_rerun = ids(&interrupted.not_rerun);
    let resumed = session
        .resume_interrupted_turn()
        .await
        .unwrap()
        .expect("original admission is recoverable");
    replacement.activate(session_id).await.unwrap();
    let turn = resumed.wait().await.unwrap();
    assert_eq!(
        session
            .admission(message)
            .await
            .unwrap()
            .unwrap()
            .wait()
            .await
            .unwrap()
            .turn_id,
        turn.turn_id
    );
    let same_turn = turn.turn_id == interrupted.turn_id;
    let after = session.interrupted_turn().await.unwrap();
    let observed = Observed::turns(vec![turn])
        .note(format!(
            "interrupted calls: {calls:?}, same turn: {same_turn}"
        ))
        .note(format!("not rerun: {not_rerun:?}"))
        .note(format!("tool runs: {}", probe.runs.load(Ordering::SeqCst)))
        .note(format!("interrupted after: {}", after.is_some()));
    Outcome {
        turns: observed.turns.into_iter().map(TurnShape::from).collect(),
        notes: observed.notes,
        event_types: session
            .events_after(0)
            .await
            .unwrap()
            .iter()
            .map(|event| event.event_type().to_string())
            .collect(),
    }
}

#[tokio::test(start_paused = true)]
async fn a_turn_cut_off_mid_act_resumes_from_the_log() {
    let outcome = cut_off_and_resume(true).await;
    assert_eq!(
        outcome.notes,
        [
            r#"interrupted calls: ["call_approval"], same turn: true"#,
            "not rerun: []",
            "tool runs: 2",
            "interrupted after: false",
        ]
    );
    let turn = &outcome.turns[0];
    assert!(turn.success, "{turn:?}");
    assert_eq!(turn.response, "Deployed.");
    assert_eq!(
        turn.iterations, 2,
        "the finalized outcome includes both model calls"
    );
    assert_eq!(
        turn.tool_calls, 1,
        "the resumed run reruns the cut-off call"
    );
    assert!(has_event(&outcome, "turn.completed"), "{outcome:?}");
}

#[tokio::test(start_paused = true)]
async fn a_call_that_may_not_run_twice_is_settled_as_interrupted() {
    let outcome = cut_off_and_resume(false).await;
    assert_eq!(
        outcome.notes,
        [
            r#"interrupted calls: ["call_approval"], same turn: true"#,
            r#"not rerun: ["call_approval"]"#,
            "tool runs: 1",
            "interrupted after: false",
        ]
    );
    let turn = &outcome.turns[0];
    assert!(turn.success, "{turn:?}");
    assert_eq!(turn.response, "Deployed.");
    assert!(has_event(&outcome, "turn.completed"), "{outcome:?}");
}
