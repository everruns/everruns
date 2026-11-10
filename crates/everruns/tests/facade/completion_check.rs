//! The completion check from an embedded agent: `always` reviews an attended
//! session's finishing turn and continues announced but unfinished work; the
//! default leaves attended sessions alone.

use everruns::completion_check::{CompletionCheck, ContinuationLimits};
use everruns::prelude::*;
use everruns::{FunctionTool, LlmSimConfig, SimToolCall, SimTurn, ToolResponse};
use serde_json::{Value, json};

fn script() -> LlmSimConfig {
    let verdict =
        |state: &str| SimTurn::Assistant(json!({"state": state, "reason": "checked"}).to_string());
    LlmSimConfig::scripted(vec![
        SimTurn::ToolCalls(vec![SimToolCall {
            name: "run_tests".into(),
            arguments: json!({}),
            id: None,
        }]),
        SimTurn::Assistant("Fixing the test now.".into()),
        verdict("in_progress"),
        SimTurn::Assistant("Fixed the test.".into()),
        verdict("achieved"),
    ])
}

fn agent(check: Option<CompletionCheck>) -> Agent {
    let run_tests = FunctionTool::new(
        "run_tests",
        "Run the test suite.",
        json!({"type": "object", "properties": {}}),
        |_: Value| async move { Ok::<_, String>(ToolResponse::text("1 failed")) },
    );
    let builder = Agent::builder()
        .instructions("Fix the failing test.")
        .model(Model::simulated_with_config(script()))
        .tool(run_tests)
        .continuation_limits(ContinuationLimits {
            max_turns: 5,
            max_tokens: 256_000,
            max_elapsed_secs: 600,
        });
    match check {
        Some(check) => builder.completion_check(check),
        None => builder,
    }
    .build()
    .expect("valid agent")
}

#[tokio::test]
async fn always_continues_an_announced_but_unfinished_turn() {
    let session = InMemoryEngine::new().create(agent(Some(CompletionCheck::Always)));

    let turn = session
        .run("Fix the failing test.")
        .await
        .expect("turn runs");

    assert!(turn.success, "{:?}", turn.error);
    assert_eq!(turn.response, "Fixed the test.");
}

#[tokio::test]
async fn the_default_leaves_an_attended_turn_unchecked() {
    let session = InMemoryEngine::new().create(agent(None));

    let turn = session
        .run("Fix the failing test.")
        .await
        .expect("turn runs");

    assert!(turn.success, "{:?}", turn.error);
    assert_eq!(turn.response, "Fixing the test now.");
}
