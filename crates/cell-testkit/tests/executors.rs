//! A thin, complete cell deployment (data plane v2 step 6): a three-node
//! cell and a one-node cell whose turns run on executors outside them, each
//! node and executor in its own runtime so a test can kill it outright.
//!
//! What it shows: turns run on executors, never on the nodes; a cancel
//! reaches a turn on an executor; a killed executor's turn finishes on
//! another; a turn survives its cell's leader dying; live deltas reach SSE
//! from an executor; the workspace survives executors and a move; an
//! executor runs a session again after it moved cells.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::time::{Duration, Instant};

use everruns_cell::remote::EXECUTOR_IDS;
use serde_json::json;
use support::{Fleet, answers, pulled, types};

#[tokio::test(flavor = "multi_thread")]
async fn turns_run_on_executors_not_nodes() {
    let mut fleet = Fleet::start().await;
    let executor = fleet.executor(1);
    let session = fleet.create("worker").await;
    fleet.say(&session, "go").await;
    fleet.turns(&session, 1).await;
    let events = fleet.events(&session).await;
    let kinds = types(&events);
    assert!(kinds.contains(&"tool.completed"), "{kinds:?}");
    assert!(kinds.contains(&"turn.completed"), "{kinds:?}");
    assert!(pulled(&executor) >= 1, "the executor pulled the turn");

    // A second turn of the same session goes back to the warm executor.
    fleet.say(&session, "again").await;
    fleet.turns(&session, 2).await;
    assert!(pulled(&executor) >= 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_reaches_a_turn_on_an_executor() {
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let session = fleet.create("slow").await;
    let admitted = fleet.say(&session, "go").await;
    let holder = fleet.held(&session).await;
    assert!(holder >= EXECUTOR_IDS, "held by an executor, not a node");
    let message = admitted["id"].as_str().unwrap();
    fleet
        .post(
            &format!("/v1/sessions/{session}/messages/{message}/cancel"),
            json!({}),
        )
        .await;
    fleet.turns(&session, 1).await;
    let events = fleet.events(&session).await;
    let kinds = types(&events);
    assert!(kinds.contains(&"turn.cancelled"), "{kinds:?}");
    assert!(!kinds.contains(&"turn.completed"), "{kinds:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_killed_executors_turn_finishes_on_another() {
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let session = fleet.create("slow").await;
    fleet.say(&session, "go").await;
    assert_eq!(fleet.held(&session).await, EXECUTOR_IDS + 1);
    let survivor = fleet.executor(2);
    fleet.kill_executor(1);
    fleet.turns(&session, 1).await;
    let events = fleet.events(&session).await;
    let kinds = types(&events);
    assert!(kinds.contains(&"turn.completed"), "{kinds:?}");
    assert!(pulled(&survivor) >= 1, "the survivor resumed the turn");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_turn_survives_its_cells_leader_dying() {
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let session = fleet.create_in("slow", 0).await;
    fleet.say(&session, "go").await;
    fleet.held(&session).await;
    let leader = fleet.leader(0).await;
    fleet.kill_node(0, leader);
    let next = fleet.leader(0).await;
    assert_ne!(next, leader);
    fleet.turns(&session, 1).await;
    let kinds_events = fleet.events(&session).await;
    let kinds = types(&kinds_events);
    assert!(kinds.contains(&"turn.completed"), "{kinds:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn live_deltas_from_an_executor_reach_sse() {
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let session = fleet.create("chat").await;
    let url = format!("{}/v1/sessions/{session}/sse", fleet.base());
    let mut stream = fleet.http.get(&url).send().await.unwrap();
    fleet.say(&session, "tell me").await;
    let mut seen = String::new();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !seen.contains("turn.completed") && Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(15), stream.chunk()).await {
            Ok(Ok(Some(chunk))) => seen.push_str(&String::from_utf8_lossy(&chunk)),
            _ => break,
        }
    }
    assert!(seen.contains("turn.completed"), "{seen}");
    assert!(
        seen.contains("event: output.message.delta"),
        "a live delta streamed: {seen}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_workspace_survives_executors_and_a_move() {
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let session = fleet.create_in("files", 0).await;
    fleet.say(&session, "write /notes/plan.md step one").await;
    fleet.turns(&session, 1).await;
    // Larger than an inline entry: its content goes to the object store.
    let big = "lorem ipsum ".repeat(2_000);
    fleet
        .say(&session, &format!("write /notes/big.md {big}tail-marker"))
        .await;
    fleet.turns(&session, 2).await;

    // Another executor, in another cell.
    fleet.kill_executor(1);
    fleet.executor(2);
    let moved = fleet
        .post(
            &format!("/v1/sessions/{session}/move"),
            json!({ "cell": 1 }),
        )
        .await;
    assert_eq!(moved["to"], 1, "{moved}");

    fleet.say(&session, "read /notes/plan.md").await;
    fleet.turns(&session, 3).await;
    fleet.say(&session, "read /notes/big.md").await;
    fleet.turns(&session, 4).await;
    let events = fleet.events(&session).await;
    let answers = answers(&events);
    assert_eq!(answers.len(), 4, "{answers:?}");
    assert!(answers[2].contains("step one"), "{}", answers[2]);
    assert!(
        answers[3].contains("tail-marker"),
        "the big file came back whole"
    );
    assert!(answers.iter().all(|a| !a.contains("error")), "{answers:?}");
}

/// The balancer moves a session that last ran here: the next turn runs
/// here again at once. Log positions are per cell, so what this executor
/// waited for in the old cell means nothing in the new one (a load run
/// found turns waiting for cell 0's position in cell 1's log).
#[tokio::test(flavor = "multi_thread")]
async fn an_executor_runs_a_session_again_after_it_moved_cells() {
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    // Put cell 0's log well ahead of cell 1's.
    for _ in 0..10 {
        let other = fleet.create_in("chat", 0).await;
        fleet.say(&other, "hello").await;
        fleet.turns(&other, 1).await;
    }
    let session = fleet.create_in("chat", 0).await;
    fleet.say(&session, "hello").await;
    fleet.turns(&session, 1).await;
    let ahead = fleet.stats(0).await["applied"].as_u64().unwrap();

    let moved = fleet
        .post(
            &format!("/v1/sessions/{session}/move"),
            json!({ "cell": 1 }),
        )
        .await;
    assert_eq!(moved["to"], 1, "{moved}");
    let behind = fleet.stats(1).await["applied"].as_u64().unwrap();
    assert!(ahead > behind + 30, "cell 0 at {ahead}, cell 1 at {behind}");
    let started = Instant::now();
    fleet.say(&session, "again").await;
    fleet.turns(&session, 2).await;
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "the turn after the move took {:?}",
        started.elapsed()
    );
    fleet.check();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tool_waits_for_approval_and_runs_once_approved() {
    approved("approve", "guarded").await;
}

/// The `tool_approval` capability an agent names (the console's "ask before
/// acting") gates a state-changing tool the same way.
#[tokio::test(flavor = "multi_thread")]
async fn the_approval_capability_gates_a_tool_until_approved() {
    approved("gate", "note").await;
}

/// `agent`'s turn parks on `tool` until a person allows it, then runs it
/// and answers.
async fn approved(agent: &str, tool: &str) {
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let session = fleet.create(agent).await;
    fleet.say(&session, "change it").await;
    // The turn parks on the request: no answer until a person decides.
    let deadline = Instant::now() + Duration::from_secs(30);
    let request = loop {
        let events = fleet.events(&session).await;
        if let Some(asked) = events.iter().find(|e| e["type"] == "tool.call_requested") {
            break asked["data"]["tool_calls"][0]["id"]
                .as_str()
                .unwrap()
                .to_owned();
        }
        assert!(
            Instant::now() < deadline,
            "no approval request: {:?}",
            types(&events)
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let answered = fleet
        .post(
            &format!("/v1/sessions/{session}/tool-approvals"),
            json!({ "decisions": [{ "tool_call_id": request, "decision": "allow" }] }),
        )
        .await;
    assert_eq!(
        answered["resolved"].as_array().map(Vec::len),
        Some(1),
        "{answered}"
    );
    fleet.turns(&session, 1).await;
    let events = fleet.events(&session).await;
    let ran = events.iter().any(|e| {
        e["type"] == "tool.completed"
            && e["data"]["tool_name"] == tool
            && e["data"]["status"] == "success"
    });
    assert!(ran, "{tool} ran once approved: {:?}", types(&events));
    assert_eq!(answers(&events), vec!["Changed, as approved.".to_string()]);
}
