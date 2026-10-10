//! A turn cut off in its act, then resumed from the session log.
//!
//! The cut: the tool's first run waits forever, and every handle to the
//! session is dropped while it does, which drops the turn's future mid-act
//! and gives its lease up. The log keeps a turn with a tool call and no
//! result, exactly what a process exit leaves. The engine reopens the session, and `resume_interrupted_turn` runs
//! the unfinished call again when its tool is idempotent, or records it as
//! interrupted when it is not, and lets the turn carry on.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use everruns::{Agent, FunctionTool, LlmSimConfig, Model, ToolCall};
use serde_json::json;
use tokio::sync::Notify;

use crate::support::{Observed, has_event, run_on};

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
async fn cut_off_and_resume(idempotent: bool) -> crate::support::Outcome {
    run_on(
        move || agent_with_waiting_tool(idempotent),
        |engine, session, probe| async move {
            let session_id = session.session_id();
            let sent = session.send("Deploy.").await.expect("send");
            probe.entered.notified().await;

            // The cut: nothing holds the session while its act waits.
            drop(sent);
            drop(session);
            probe.dropped.notified().await;

            let session = engine
                .resume(session_id)
                .await
                .expect("the session reopens");
            let interrupted = session
                .interrupted_turn()
                .await
                .expect("the log reads")
                .expect("the cut-off turn is interrupted");
            let ids = |calls: &[ToolCall]| -> Vec<String> {
                calls.iter().map(|call| call.id.clone()).collect()
            };
            let calls = ids(&interrupted.tool_calls);
            let not_rerun = ids(&interrupted.not_rerun);
            let resumed = session
                .resume_interrupted_turn()
                .await
                .expect("resume starts")
                .expect("there is a turn to resume");
            let turn = resumed.wait().await.expect("the resumed turn finishes");
            let same_turn = turn.turn_id == interrupted.turn_id;
            let after = session.interrupted_turn().await.expect("the log reads");

            Observed::turns(vec![turn])
                .note(format!(
                    "interrupted calls: {calls:?}, same turn: {same_turn}"
                ))
                .note(format!("not rerun: {not_rerun:?}"))
                .note(format!("tool runs: {}", probe.runs.load(Ordering::SeqCst)))
                .note(format!("interrupted after: {}", after.is_some()))
        },
    )
    .await
}

#[tokio::test]
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
    assert_eq!(turn.iterations, 1, "the resumed run reasons once");
    assert_eq!(
        turn.tool_calls, 1,
        "the resumed run reruns the cut-off call"
    );
    assert!(has_event(&outcome, "turn.completed"), "{outcome:?}");
}

#[tokio::test]
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
