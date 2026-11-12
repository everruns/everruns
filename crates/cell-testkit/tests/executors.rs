//! A thin, complete cell deployment (data plane v2 step 6): a three-node
//! cell and a one-node cell whose turns run on executors outside them, each
//! node and executor in its own runtime so a test can kill it outright.
//!
//! What it shows: turns run on executors, never on the nodes; a cancel
//! reaches a turn on an executor; a killed executor's turn finishes on
//! another; a turn survives its cell's leader dying; live deltas reach SSE
//! from an executor; the workspace survives executors and a move.

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
