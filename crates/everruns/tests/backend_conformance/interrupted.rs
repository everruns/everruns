//! A turn cut off in its act, then resumed from the session log.
//!
//! The cut: the tool's first run waits forever, and every handle to the
//! session is dropped while it does. On the in-process backend that drops
//! the turn's future mid-act; on the durable backend it detaches the session,
//! which drops the in-flight act step and fails its task. Either way the log
//! keeps a turn with a tool call and no result, exactly what a process exit
//! leaves. The engine reopens the session, and `resume_interrupted_turn` runs
//! the unfinished call again and lets the turn carry on.

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

fn agent_with_waiting_tool() -> (Agent, Probe) {
    let probe = Probe::default();
    let tool_probe = probe.clone();
    let tool = FunctionTool::new(
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

#[tokio::test]
async fn a_turn_cut_off_mid_act_resumes_from_the_log() {
    let outcome = run_on(
        agent_with_waiting_tool,
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
            let calls: Vec<String> = interrupted
                .tool_calls
                .iter()
                .map(|call| call.id.clone())
                .collect();
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
                .note(format!("tool runs: {}", probe.runs.load(Ordering::SeqCst)))
                .note(format!("interrupted after: {}", after.is_some()))
        },
    )
    .await;
    assert_eq!(
        outcome.notes,
        [
            r#"interrupted calls: ["call_approval"], same turn: true"#,
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
