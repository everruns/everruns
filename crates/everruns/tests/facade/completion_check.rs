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

#[tokio::test]
async fn always_reviews_a_promise_made_without_a_tool_call() {
    let verdict =
        |state: &str| SimTurn::Assistant(json!({"state": state, "reason": "checked"}).to_string());
    let session = InMemoryEngine::new().create(
        Agent::builder()
            .instructions("Run the tests.")
            .model(Model::simulated_with_config(LlmSimConfig::scripted(vec![
                SimTurn::Assistant("Running the tests now.".into()),
                verdict("in_progress"),
                SimTurn::Assistant("All tests pass.".into()),
                verdict("achieved"),
            ])))
            .completion_check(CompletionCheck::Always)
            .build()
            .expect("valid agent"),
    );

    let turn = session.run("Run the tests.").await.expect("turn runs");

    assert!(turn.success, "{:?}", turn.error);
    assert_eq!(turn.response, "All tests pass.");
}

fn stops_after_tools(turns: Vec<SimTurn>, max_turns: u32) -> Agent {
    let run_tests = FunctionTool::new(
        "run_tests",
        "Run the test suite.",
        json!({"type": "object", "properties": {}}),
        |_: Value| async move { Ok::<_, String>(ToolResponse::text("1 failed")) },
    );
    let mut script = vec![SimTurn::ToolCalls(vec![SimToolCall {
        name: "run_tests".into(),
        arguments: json!({}),
        id: None,
    }])];
    script.extend(turns);
    Agent::builder()
        .instructions("Fix the failing test.")
        .model(Model::simulated_with_config(LlmSimConfig::scripted(script)))
        .tool(run_tests)
        .completion_check(CompletionCheck::Always)
        .continuation_limits(ContinuationLimits {
            max_turns,
            max_tokens: 256_000,
            max_elapsed_secs: 600,
        })
        .build()
        .expect("valid agent")
}

/// The `action` of each `turn.completion_checked` the session recorded.
async fn check_actions(session: &Session) -> Vec<String> {
    session
        .events_after(0)
        .await
        .expect("events")
        .iter()
        .filter(|event| event.event_type() == "turn.completion_checked")
        .map(|event| {
            event.canonical_json()["data"]["action"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        })
        .collect()
}

/// Tools ran and the turn ended with nothing said: it continues without a
/// review call, and the continuation's answer is reviewed as usual.
#[tokio::test]
async fn an_empty_final_after_tools_continues_without_a_review() {
    let session = InMemoryEngine::new().create(stops_after_tools(
        vec![
            SimTurn::Assistant(String::new()),
            SimTurn::Assistant("Fixed the test.".into()),
            SimTurn::Assistant(json!({"state": "achieved", "reason": "checked"}).to_string()),
        ],
        5,
    ));

    let turn = session
        .run("Fix the failing test.")
        .await
        .expect("turn runs");

    assert!(turn.success, "{:?}", turn.error);
    assert_eq!(turn.response, "Fixed the test.");
    assert_eq!(check_actions(&session).await, ["continued", "finished"]);
}

/// With the continuation budget spent, the same stop finishes the turn.
#[tokio::test]
async fn an_empty_final_with_the_budget_spent_finishes() {
    let session = InMemoryEngine::new().create(stops_after_tools(
        vec![
            SimTurn::Assistant(String::new()),
            SimTurn::Assistant(String::new()),
        ],
        1,
    ));

    let turn = session
        .run("Fix the failing test.")
        .await
        .expect("turn runs");

    assert!(turn.success, "{:?}", turn.error);
    assert_eq!(turn.response, "");
    assert_eq!(
        check_actions(&session).await,
        ["continued", "budget_exhausted"]
    );
}
