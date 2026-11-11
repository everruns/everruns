//! Step 6's failure qualification against a live fleet (design.md,
//! "Implementation plan", step 6), beside the control plane cases in
//! `control_plane.rs`: a stale executor is fenced out, old messages of
//! earlier moves change nothing, and a whole cell restarting after an
//! external effect neither loses the turn nor runs the effect again.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::time::Duration;

use everruns_cell::remote::EXECUTOR_IDS;
use serde_json::{Value, json};
use support::{Fleet, types};

/// The current epoch's fence of `session`.
async fn fence(fleet: &Fleet, session: &str) -> u64 {
    fleet.held(session).await;
    fleet.get(&format!("/v1/sessions/{session}")).await["active"]["fence"]
        .as_u64()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stale_executor_is_fenced_out() {
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let session = fleet.create_in("slow", 0).await;
    fleet.say(&session, "go").await;
    assert_eq!(fleet.held(&session).await, EXECUTOR_IDS + 1);
    let stale = fence(&fleet, &session).await;
    // Executor 1 stops answering (a long pause looks the same to the
    // cell); executor 2 takes the turn over under a newer epoch.
    fleet.kill_executor(1);
    fleet.executor(2);
    fleet
        .until("executor 2 holds the session", || async {
            fleet.get(&format!("/v1/sessions/{session}")).await["active"]["node"]
                == json!(EXECUTOR_IDS + 2)
        })
        .await;
    // Executor 1 comes back and writes as if nothing happened.
    let write = json!({
        "op": "files",
        "session": session,
        "fence": stale,
        "op_id": "stale-write",
        "changes": [{ "op": "put", "path": "/late.md", "encoding": "utf8",
                      "content": { "kind": "inline", "data": "late" }, "size": 4 }],
        "now_ms": chrono::Utc::now().timestamp_millis(),
    });
    let response = fleet.propose(0, write).await;
    assert_eq!(response["result"], "fenced", "{response}");
    let live = fleet
        .post(
            "/cell/exec/live",
            json!({ "session": session, "fence": stale, "events": [] }),
        )
        .await;
    assert_eq!(live["taken"], false, "its live deltas are dropped too");
    fleet.turns(&session, 1).await;
    let events = fleet.events(&session).await;
    assert!(types(&events).contains(&"turn.completed"));
}

#[tokio::test(flavor = "multi_thread")]
async fn old_messages_of_earlier_moves_change_nothing() {
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let session = fleet.create_in("worker", 0).await;
    // Back and forth: 0 -> 1 -> 0 -> 1 -> 0, a turn between moves.
    let mut placements: Vec<Value> = Vec::new();
    for (turn, to) in [1, 0, 1, 0].into_iter().enumerate() {
        fleet.say(&session, "go").await;
        fleet.turns(&session, turn as u64 + 1).await;
        let moved = fleet
            .post(&format!("/v1/sessions/{session}/move"), json!({ "cell": to }))
            .await;
        assert_eq!(moved["to"], to, "{moved}");
        let placed = fleet.get(&format!("/cell/directory/{session}")).await;
        placements.push(placed);
    }
    let now = placements.last().unwrap().clone();
    assert_eq!(now["cell"], 0);
    let first = &placements[0];
    let generation = first["generation"].as_u64().unwrap();
    // The first move's messages, delivered late.
    let place = fleet
        .propose(
            0,
            json!({ "op": "place", "session": session, "cell": 1,
                    "generation": generation, "previous": generation - 1 }),
        )
        .await;
    assert_eq!(place["result"], "placement", "{place}");
    let activate = fleet
        .propose(
            1,
            json!({ "op": "activate", "session": session, "generation": generation }),
        )
        .await;
    assert_ne!(activate["result"], "ok", "{activate}");
    let forward = fleet
        .propose(
            0,
            json!({ "op": "forward", "session": session, "cell": 1, "generation": generation }),
        )
        .await;
    assert_ne!(forward["result"], "ok", "{forward}");
    // Nothing moved: the session lives where the last move put it, and
    // still runs.
    assert_eq!(fleet.get(&format!("/cell/directory/{session}")).await, now);
    fleet.say(&session, "once more").await;
    fleet.turns(&session, 5).await;
    let events = fleet.events(&session).await;
    let completed = types(&events)
        .iter()
        .filter(|t| **t == "turn.completed")
        .count();
    assert_eq!(completed, 5);
}

/// Lines of the work tool's ledger naming `session`: real executions.
fn executions(ledger: &std::path::Path, session: &str) -> usize {
    std::fs::read_to_string(ledger)
        .unwrap_or_default()
        .lines()
        .filter(|line| *line == session)
        .count()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cell_restart_after_an_external_effect_runs_it_once() {
    let ledger = tempfile::NamedTempFile::new().unwrap();
    // SAFETY-free in practice: this file's tests read the variable only
    // through the work tool, and no other test in this binary sets it.
    unsafe {
        std::env::set_var("CELL_LEDGER", ledger.path());
    }
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let session = fleet.create_in("worker3", 0).await;
    fleet.say(&session, "go").await;
    // Once the first step's effect happened, the whole cell goes down.
    fleet
        .until("the first step ran", || async {
            executions(ledger.path(), &session) >= 1
        })
        .await;
    for node in [1, 2, 3] {
        fleet.kill_node(0, node);
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    for node in [1, 2, 3] {
        fleet.start_node(0, node);
    }
    fleet.leader(0).await;
    fleet.turns(&session, 1).await;
    let events = fleet.events(&session).await;
    let kinds = types(&events);
    assert!(kinds.contains(&"turn.completed"), "{kinds:?}");
    let started = kinds.iter().filter(|t| **t == "tool.started").count();
    assert_eq!(
        executions(ledger.path(), &session),
        started,
        "every effect is a durable tool.started, and none ran twice"
    );
    let completed = kinds.iter().filter(|t| **t == "tool.completed").count();
    assert_eq!(completed, 3, "{kinds:?}");
}
