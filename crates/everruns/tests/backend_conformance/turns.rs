//! Turns started by messages: answers, tools, steering and cancellation.

use std::time::Duration;

use everruns::{
    Agent, FunctionTool, LlmSimConfig, Model, SendDisposition, ToolCall, TurnStopReason,
};
use serde_json::json;

use crate::support::{Observed, agent_with, has_event, run_on};

/// Poll the session's history until an event of `event_type` is persisted.
async fn wait_for_event(session: &everruns::Session, event_type: &str) {
    loop {
        let events = session.events_after(0).await.expect("history reads");
        if events.iter().any(|event| event.event_type() == event_type) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn delayed(response: &str, delay: Duration) -> Model {
    Model::simulated_with_config(LlmSimConfig::fixed(response).with_response_delay(delay))
}

#[tokio::test]
async fn a_single_turn() {
    let outcome = run_on(
        || (agent_with(Model::simulated("Sure.")), ()),
        |_, session, ()| async move {
            Observed::turns(vec![
                session.send_and_wait("hi").await.expect("first turn"),
                session.send_and_wait("again").await.expect("second turn"),
            ])
        },
    )
    .await;
    assert_eq!(outcome.turns[0].response, "Sure.");
    assert!(outcome.turns.iter().all(|turn| turn.success));
    assert_eq!(outcome.turns[0].iterations, 1);
    assert!(has_event(&outcome, "turn.completed"), "{outcome:?}");
}

#[tokio::test]
async fn a_tool_loop() {
    let agent = || {
        let tool = FunctionTool::new(
            "ping",
            "Respond to a ping.",
            json!({ "type": "object", "properties": {} }),
            |_args: serde_json::Value| async move { Ok::<_, String>(json!({ "ok": true })) },
        );
        let model = Model::simulated_with_config(
            LlmSimConfig::fixed("pinged").with_tool_call_sequence(vec![
                vec![ToolCall {
                    id: "call_ping_1".into(),
                    name: "ping".into(),
                    arguments: json!({}),
                }],
                vec![],
            ]),
        );
        let agent = Agent::builder()
            .instructions("Call ping when asked.")
            .model(model)
            .tool(tool)
            .build()
            .expect("valid agent");
        (agent, ())
    };
    let outcome = run_on(agent, |_, session, ()| async move {
        Observed::turns(vec![
            session.send_and_wait("please ping").await.expect("turn"),
        ])
    })
    .await;
    assert_eq!(outcome.turns[0].response, "pinged");
    assert_eq!(outcome.turns[0].iterations, 2);
    assert_eq!(outcome.turns[0].tool_calls, 1);
    assert!(has_event(&outcome, "tool.completed"), "{outcome:?}");
}

/// A message sent while a turn reasons joins it; one sent after it ended
/// starts the next turn.
#[tokio::test]
async fn steering_joins_the_running_turn_and_a_later_send_starts_the_next() {
    let outcome = run_on(
        || (agent_with(delayed("Sure.", Duration::from_millis(300))), ()),
        |_, session, ()| async move {
            let first = session.send("hi").await.expect("send");
            // Steer only once the turn reasons: a fixed sleep let a slow
            // backend (PostgreSQL on CI) take the steer as initial input.
            wait_for_event(&session, "reason.started").await;
            let steered = session.send("and this").await.expect("steer");
            let same_turn = steered.turn_id == first.turn_id;
            let first_turn = first.wait().await.expect("turn");

            let next = session.send("next").await.expect("send");
            let new_turn = next.turn_id != first.turn_id;
            let next_turn = next.wait().await.expect("next turn");
            Observed::turns(vec![first_turn, next_turn])
                .note(format!("first: {:?}", first.disposition))
                .note(format!(
                    "steered: {:?}, same turn: {same_turn}",
                    steered.disposition
                ))
                .note(format!(
                    "next: {:?}, new turn: {new_turn}",
                    next.disposition
                ))
        },
    )
    .await;
    assert_eq!(
        outcome.notes,
        [
            format!("first: {:?}", SendDisposition::Started),
            format!("steered: {:?}, same turn: true", SendDisposition::Steered),
            format!("next: {:?}, new turn: true", SendDisposition::Started),
        ]
    );
    assert_eq!(
        outcome.turns[0].iterations, 2,
        "the steered input ran a reason"
    );
    assert_eq!(outcome.turns[1].iterations, 1);
}

#[tokio::test]
async fn cancel_then_the_next_turn() {
    let outcome = run_on(
        || {
            (
                agent_with(delayed("eventually", Duration::from_secs(1))),
                (),
            )
        },
        |_, session, ()| async move {
            let sent = session.send("hi").await.expect("send");
            tokio::time::sleep(Duration::from_millis(100)).await;
            tokio::time::timeout(Duration::from_millis(500), sent.turn().cancel())
                .await
                .expect("cancel does not wait for the slow step")
                .expect("the turn was active");
            let cancelled = sent.wait().await.expect("a cancelled turn still resolves");
            let next = session.send_and_wait("again").await.expect("next turn");
            Observed::turns(vec![cancelled, next])
        },
    )
    .await;
    assert_eq!(outcome.turns[0].stop_reason, TurnStopReason::Cancelled);
    assert_eq!(outcome.turns[1].response, "eventually");
    assert!(has_event(&outcome, "turn.cancelled"), "{outcome:?}");
}
