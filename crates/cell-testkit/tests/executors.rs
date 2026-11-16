//! A thin, complete cell deployment (data plane v2 step 6): a three-node
//! cell and a one-node cell whose turns run on executors outside them, each
//! node and executor in its own runtime so a test can kill it outright.
//!
//! What it shows: turns run on executors, never on the nodes; a cancel
//! reaches a turn on an executor; a killed executor's turn finishes on
//! another; a turn survives its cell's leader dying; live deltas reach SSE
//! from an executor; the workspace survives executors and a move; an
//! executor runs a session again after it moved cells; a cell short of
//! memory sheds new work while admitted turns finish; a cell with Raft on
//! its own listener and threads fails over like any other; a cell that
//! stops committing sheds new work and takes it again once it commits; only
//! the leader says a session is missing; Raft's own listener takes large
//! snapshot chunks.

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

/// The same, with Raft on each node's own listener and threads: killing
/// the leader's node stops its Raft too, and the cell elects another.
#[tokio::test(flavor = "multi_thread")]
async fn a_turn_survives_its_leader_dying_with_raft_on_its_own_threads() {
    let mut fleet = Fleet::start_isolated().await;
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

/// A leader that cannot commit (both followers gone) refuses new input
/// with 429 and a retry hint instead of holding every request to its
/// deadline, and takes input again once its followers are back.
#[tokio::test(flavor = "multi_thread")]
async fn a_cell_that_stops_committing_sheds_new_work_until_it_commits() {
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let session = fleet.create_in("worker", 0).await;
    fleet.say(&session, "go").await;
    fleet.turns(&session, 1).await;
    let leader = fleet.leader(0).await;
    let followers: Vec<u64> = (1..=3).filter(|n| *n != leader).collect();
    for node in &followers {
        fleet.kill_node(0, *node);
    }
    let messages = format!("/v1/sessions/{session}/messages");
    let deadline = Instant::now() + Duration::from_secs(60);
    let (status, retry, body) = loop {
        let reply = fleet.post_once(&messages, json!({ "text": "more" })).await;
        if reply.0 == 429 || Instant::now() > deadline {
            break reply;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    assert_eq!(status, 429, "{body}");
    assert_eq!(
        retry.as_deref(),
        Some("2"),
        "the refusal says when to retry"
    );
    let addr = fleet
        .addrs(0)
        .into_iter()
        .find(|(node, _)| *node == leader)
        .map(|(_, addr)| addr)
        .unwrap();
    let metrics = reqwest::get(format!("http://{addr}/metrics"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        metrics
            .lines()
            .any(|l| l.starts_with("cell_commit_stalled{") && l.ends_with(" 1")),
        "the leader reports the stall"
    );
    for node in followers {
        fleet.start_node(0, node);
    }
    fleet.leader(0).await;
    fleet.say(&session, "again").await;
    fleet.turns(&session, 2).await;
}

/// Raft's own listener takes bodies past axum's default 2 MB: snapshot
/// chunks are 16 MB, and a refused chunk left a follower that fell behind
/// unable to catch up for good (EKS run 43803384547).
#[tokio::test(flavor = "multi_thread")]
async fn raft_on_its_own_listener_takes_large_snapshot_chunks() {
    let fleet = Fleet::start_isolated().await;
    let (_, addr) = fleet.addrs(0).into_iter().next().unwrap();
    let raft = everruns_cell::network::raft_addr(&addr, 1);
    let response = fleet
        .http
        .post(format!("http://{raft}/raft/snapshot"))
        .body(vec![0u8; 3 << 20])
        .send()
        .await
        .unwrap();
    // Read in full and refused for its missing header, not for its size.
    assert_eq!(response.status().as_u16(), 400);
}

/// Only the leader's "not found" is final. A follower answers from its own
/// copy, and one that rejoined behind (EKS run 43803384547) said "not found"
/// for sessions created while it was down, failing their first message. A
/// follower that has not seen a session sends the caller to another member.
#[tokio::test(flavor = "multi_thread")]
async fn only_the_leader_says_a_session_is_missing() {
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let leader = fleet.leader(0).await;
    let unknown = "session_01a1e29c3fd97657b9d3603032190000";
    for (node, addr) in fleet.addrs(0) {
        let response = fleet
            .http
            .get(format!("http://{addr}/cell/sessions/{unknown}"))
            .send()
            .await
            .unwrap();
        let expected = if node == leader { 200 } else { 421 };
        assert_eq!(response.status().as_u16(), expected, "node {node}");
    }
    let (status, _, body) = fleet
        .post_once(
            &format!("/v1/sessions/{unknown}/messages"),
            json!({ "text": "hi" }),
        )
        .await;
    assert_eq!(status, 404, "{body}");

    let follower = (1..=3).find(|n| *n != leader).unwrap();
    fleet.kill_node(0, follower);
    let session = fleet.create_in("worker", 0).await;
    fleet.start_node(0, follower);
    fleet.say(&session, "go").await;
    fleet.turns(&session, 1).await;
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

/// A cell whose node is near its memory limit refuses new input and new
/// sessions with 429 and a retry hint, lets the turn it already admitted
/// finish, gets no new sessions from the gateway, and takes work again once
/// memory frees.
#[tokio::test(flavor = "multi_thread")]
async fn a_cell_short_of_memory_sheds_new_work_and_finishes_admitted_turns() {
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let session = fleet.create_in("slow", 1).await;
    fleet.say(&session, "go").await;
    // Cell 1's only node now measures itself against 1 MiB: shedding.
    fleet.restart_with_memory_limit(1, 1, 1 << 20);
    fleet.leader(1).await;
    let say = |text: &str| json!({ "text": text });
    let messages = format!("/v1/sessions/{session}/messages");
    let deadline = Instant::now() + Duration::from_secs(30);
    let (status, retry, body) = loop {
        let reply = fleet.post_once(&messages, say("more")).await;
        if reply.0 == 429 || Instant::now() > deadline {
            break reply;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    assert_eq!(status, 429, "{body}");
    assert_eq!(
        retry.as_deref(),
        Some("2"),
        "the refusal says when to retry"
    );
    // The turn admitted before still finishes, and nothing else ran.
    fleet.turns(&session, 1).await;
    // New sessions: the gateway places them elsewhere, and one asked for
    // in cell 1 is refused.
    let definition = everruns_cell_testkit::definition("worker").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let created = fleet
            .post("/v1/sessions", json!({ "agent": definition }))
            .await;
        if created["cell"] == 0 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "still placed in cell 1: {created}"
        );
    }
    let (status, _, body) = fleet
        .post_once("/v1/sessions", json!({ "agent": definition, "cell": 1 }))
        .await;
    assert_eq!(status, 429, "{body}");
    let metrics = reqwest::get(format!("http://{}/metrics", fleet.addrs(1)[0].1))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let level = metrics
        .lines()
        .find(|l| l.starts_with("cell_overload_level{"))
        .unwrap_or_default();
    assert!(level.ends_with(" 2"), "shedding: {level}");
    // Memory frees: the cell takes work again.
    fleet.restart_with_memory_limit(1, 1, 1 << 40);
    fleet.leader(1).await;
    fleet.say(&session, "again").await;
    fleet.turns(&session, 2).await;
}
