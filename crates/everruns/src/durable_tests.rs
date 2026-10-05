//! The same sessions on the in-process and the durable memory backend.
//!
//! Each scenario runs once per [`BackendKind`] and the outcomes must match:
//! the final answer, the turn's shape, and the session's persisted event
//! types in order. This is the seed of the backend conformance suite;
//! scenarios join [`run_on`] as more turn inputs reach the durable backend.

use std::time::Duration;

use everruns_contracts::tool_types::ToolCall;
use everruns_core::turn::TurnStopReason;
use serde_json::json;

use crate::{Agent, Engine, Model, SendDisposition, Session, durable};

#[derive(Clone, Copy, Debug)]
enum BackendKind {
    InProcess,
    DurableMemory,
}

const BACKENDS: [BackendKind; 2] = [BackendKind::InProcess, BackendKind::DurableMemory];

fn engine(kind: BackendKind) -> Engine {
    match kind {
        BackendKind::InProcess => Engine::new(),
        BackendKind::DurableMemory => Engine::builder()
            .backend(durable::Backend::memory().workers(2))
            .build(),
    }
}

/// What a scenario observed, compared across backends.
#[derive(Debug, PartialEq)]
struct Outcome {
    turns: Vec<TurnShape>,
    event_types: Vec<String>,
}

#[derive(Debug, PartialEq)]
struct TurnShape {
    response: String,
    success: bool,
    stop_reason: TurnStopReason,
    iterations: usize,
    tool_calls: usize,
}

/// Run `scenario` on a fresh session of `agent` on each backend, and require
/// the same outcome from both.
async fn run_on<F, Fut>(agent: impl Fn() -> Agent, scenario: F) -> Outcome
where
    F: Fn(Session) -> Fut,
    Fut: std::future::Future<Output = Vec<crate::Turn>>,
{
    let mut outcomes = Vec::new();
    for kind in BACKENDS {
        let session = engine(kind).create(agent());
        let turns = tokio::time::timeout(Duration::from_secs(20), scenario(session.clone()))
            .await
            .unwrap_or_else(|_| panic!("{kind:?}: the scenario finishes"));
        let event_types = session
            .events_after(0)
            .await
            .expect("history reads")
            .iter()
            .map(|event| event.event_type().to_string())
            .collect();
        let turns = turns
            .into_iter()
            .map(|turn| TurnShape {
                response: turn.response,
                success: turn.success,
                stop_reason: turn.stop_reason,
                iterations: turn.iterations,
                tool_calls: turn.tool_calls,
            })
            .collect();
        outcomes.push((kind, Outcome { turns, event_types }));
    }
    let (_, in_process) = outcomes.remove(0);
    for (kind, outcome) in outcomes {
        assert_eq!(outcome, in_process, "{kind:?} diverges from in process");
    }
    in_process
}

fn agent_with(model: Model) -> Agent {
    Agent::builder()
        .instructions("You are concise.")
        .model(model)
        .build()
        .expect("valid agent")
}

#[tokio::test]
async fn a_single_turn_matches_in_process() {
    let outcome = run_on(
        || agent_with(Model::simulated("Sure.")),
        |session| async move {
            vec![
                session.send_and_wait("hi").await.expect("first turn"),
                session.send_and_wait("again").await.expect("second turn"),
            ]
        },
    )
    .await;
    assert_eq!(outcome.turns[0].response, "Sure.");
    assert!(outcome.turns.iter().all(|turn| turn.success));
    assert!(
        outcome
            .event_types
            .iter()
            .any(|kind| kind == "turn.completed"),
        "{:?}",
        outcome.event_types
    );
}

#[tokio::test]
async fn a_tool_loop_matches_in_process() {
    let agent = || {
        let tool = crate::FunctionTool::new(
            "ping",
            "Respond to a ping.",
            json!({ "type": "object", "properties": {} }),
            |_args: serde_json::Value| async move { Ok::<_, String>(json!({ "ok": true })) },
        );
        Agent::builder()
            .instructions("Call ping when asked.")
            .model(Model::simulated_scripted(
                "pinged",
                vec![
                    vec![ToolCall {
                        id: "call_ping_1".into(),
                        name: "ping".into(),
                        arguments: json!({}),
                    }],
                    vec![],
                ],
            ))
            .tool(tool)
            .build()
            .expect("valid agent")
    };
    let outcome = run_on(agent, |session| async move {
        vec![session.send_and_wait("please ping").await.expect("turn")]
    })
    .await;
    assert_eq!(outcome.turns[0].response, "pinged");
    assert_eq!(outcome.turns[0].tool_calls, 1);
    assert!(
        outcome
            .event_types
            .iter()
            .any(|kind| kind == "tool.completed"),
        "{:?}",
        outcome.event_types
    );
}

#[tokio::test]
async fn steering_joins_the_running_turn_as_in_process() {
    let outcome = run_on(
        || {
            agent_with(Model::simulated_delayed(
                "Sure.",
                Duration::from_millis(300),
            ))
        },
        |session| async move {
            let first = session.send("hi").await.expect("send");
            assert_eq!(first.disposition, SendDisposition::Started);
            tokio::time::sleep(Duration::from_millis(100)).await;
            let steered = session.send("and this").await.expect("steer");
            assert_eq!(steered.disposition, SendDisposition::Steered);
            assert_eq!(steered.turn_id, first.turn_id);
            vec![first.wait().await.expect("turn")]
        },
    )
    .await;
    assert_eq!(
        outcome.turns[0].iterations, 2,
        "the steered input ran a reason"
    );
}

#[tokio::test]
async fn cancel_stops_the_turn_as_in_process() {
    let outcome = run_on(
        || {
            agent_with(Model::simulated_delayed(
                "eventually",
                Duration::from_secs(1),
            ))
        },
        |session| async move {
            let sent = session.send("hi").await.expect("send");
            tokio::time::sleep(Duration::from_millis(100)).await;
            tokio::time::timeout(Duration::from_millis(500), sent.turn().cancel())
                .await
                .expect("cancel does not wait for the slow step")
                .expect("the turn was active");
            let cancelled = sent.wait().await.expect("a cancelled turn still resolves");
            // The session takes its next turn after a cancel.
            let next = session.send_and_wait("again").await.expect("next turn");
            vec![cancelled, next]
        },
    )
    .await;
    assert_eq!(outcome.turns[0].stop_reason, TurnStopReason::Cancelled);
    assert_eq!(outcome.turns[1].response, "eventually");
    assert!(
        outcome
            .event_types
            .iter()
            .any(|kind| kind == "turn.cancelled"),
        "{:?}",
        outcome.event_types
    );
}

#[tokio::test]
async fn dropping_the_engine_mid_turn_does_not_hang() {
    let session = engine(BackendKind::DurableMemory).create(agent_with(Model::simulated_delayed(
        "eventually",
        Duration::from_secs(30),
    )));
    let _sent = session.send("hi").await.expect("send");
    tokio::time::sleep(Duration::from_millis(100)).await;
    tokio::time::timeout(Duration::from_secs(5), async move { drop(session) })
        .await
        .expect("dropping the last handle returns at once");
}
