//! Step 9's disaster recovery against a live fleet (design.md, "Disaster
//! recovery"): a cell that loses every node, disks and all, comes back
//! from the object store alone, and a point-in-time restore takes a cell
//! back to a moment before later turns.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::time::Duration;

use everruns_cell::restore::FENCE_GAP;
use serde_json::json;
use support::{Fleet, now_ms, types};

/// The work tool's ledger, one for the whole binary: the variable naming it
/// is process-wide, and the tests run side by side.
fn ledger() -> &'static std::path::Path {
    static LEDGER: std::sync::OnceLock<tempfile::NamedTempFile> = std::sync::OnceLock::new();
    LEDGER
        .get_or_init(|| {
            let file = tempfile::NamedTempFile::new().unwrap();
            // SAFETY-free in practice: set once, before any fleet starts,
            // and read only through the work tool.
            unsafe {
                std::env::set_var("CELL_LEDGER", file.path());
                std::env::set_var("CELL_WORK_MS", "2000");
            }
            file
        })
        .path()
}

/// Lines of the work tool's ledger naming `session`: real executions.
fn executions(ledger: &std::path::Path, session: &str) -> usize {
    std::fs::read_to_string(ledger)
        .unwrap_or_default()
        .lines()
        .filter(|line| *line == session)
        .count()
}

fn completed(events: &[serde_json::Value]) -> usize {
    types(events)
        .iter()
        .filter(|t| **t == "turn.completed")
        .count()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cell_that_loses_every_node_comes_back_from_the_store() {
    let ledger = ledger();
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let session = fleet.create_in("worker", 0).await;
    fleet.say(&session, "go").await;
    let lost_fence = fleet.fence(&session).await;
    // The tool is running (an effect that must not happen twice) when
    // every node of the cell goes, disks included.
    fleet
        .until("the tool started", || async {
            executions(ledger, &session) >= 1
        })
        .await;
    let rebuilt = fleet.rebuild(0, None).await;
    assert_eq!(rebuilt.incarnation, 2, "{rebuilt:?}");
    assert!(rebuilt.sessions >= 1, "{rebuilt:?}");
    assert_eq!(fleet.store().incarnations(0).await.unwrap(), vec![2]);

    // The session is back, its turn finishes under an epoch the lost
    // incarnation never handed out, and the tool did not run again.
    fleet.turns(&session, 1).await;
    let events = fleet.events(&session).await;
    let kinds = types(&events);
    assert!(kinds.contains(&"tool.started"), "{kinds:?}");
    assert_eq!(
        executions(ledger, &session),
        kinds.iter().filter(|t| **t == "tool.started").count(),
        "every effect is a durable tool.started, and none ran twice: {kinds:?}"
    );
    fleet.say(&session, "again").await;
    assert!(fleet.fence(&session).await >= lost_fence + FENCE_GAP);
    fleet.turns(&session, 2).await;
    assert_eq!(completed(&fleet.events(&session).await), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_point_in_time_restore_drops_what_came_after() {
    ledger();
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let session = fleet.create_in("worker", 1).await;
    fleet.say(&session, "first").await;
    fleet.turns(&session, 1).await;
    // Let the last write of turn 1 settle a little apart from turn 2.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let point = now_ms();
    tokio::time::sleep(Duration::from_millis(200)).await;
    fleet.say(&session, "second").await;
    fleet.turns(&session, 2).await;
    // The leader ships every second; wait for turn 2 to reach the store,
    // so the restore really has something after the point to leave out.
    let store = fleet.store();
    fleet
        .until("turn 2 shipped", || async {
            everruns_cell::restore::state_at(&store, 1, None)
                .await
                .is_ok_and(|r| r.state.clock_ms > point)
        })
        .await;
    // Nothing runs while the cell is taken back.
    fleet.kill_executor(1);
    let rebuilt = fleet.rebuild(1, Some(point)).await;
    assert!(rebuilt.clock_ms <= point, "{rebuilt:?}");
    let events = fleet.events(&session).await;
    assert_eq!(completed(&events), 1, "{:?}", types(&events));
    // The restored session carries on from there.
    fleet.executor(2);
    fleet.say(&session, "after the restore").await;
    fleet.turns(&session, 2).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_one_org_restore_takes_back_only_that_org() {
    ledger();
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let far = 4_000_000_000_000i64;
    let mut sessions = Vec::new();
    for org in ["restored", "untouched"] {
        let grant = json!({
            "op": "grant", "tenant": org, "grant_id": format!("{org}-1"),
            "grant": { "granted_micros": 1_000_000_000_000i64, "per_call_micros": 1_000_000,
                       "spend_until_ms": far },
        });
        // In every cell: the fleet spreads sessions, and without a control
        // plane nothing grants a cell money when one arrives.
        for cell in [0, 1] {
            assert_eq!(fleet.propose(cell, grant.clone()).await["result"], "ok");
        }
        let definition = everruns_cell_testkit::definition("worker").unwrap();
        let created = fleet
            .post(
                "/v1/sessions",
                json!({ "agent": definition, "tenant": org, "cell": 1 }),
            )
            .await;
        sessions.push(created["id"].as_str().unwrap().to_string());
    }
    let (restored, untouched) = (&sessions[0], &sessions[1]);
    for session in &sessions {
        fleet.say(session, "first").await;
    }
    for session in &sessions {
        fleet.turns(session, 1).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    let point = now_ms();
    tokio::time::sleep(Duration::from_millis(200)).await;
    for session in &sessions {
        fleet.say(session, "second").await;
    }
    for session in &sessions {
        fleet.turns(session, 2).await;
    }
    // Every cell's leader ships each second; let turn 2 reach the store
    // wherever the fleet moved the sessions.
    let store = fleet.store();
    fleet
        .until("turn 2 shipped", || async {
            let mut shipped = true;
            for cell in [0, 1] {
                shipped &= everruns_cell::restore::state_at(&store, cell, None)
                    .await
                    .is_ok_and(|r| r.state.clock_ms > point);
            }
            shipped
        })
        .await;

    let gateway = everruns_cell::gateway::Gateway::new(fleet.topology());
    gateway.refresh_registry().await;
    let report = everruns_cell::restore::restore_org(&store, &gateway, "restored", point)
        .await
        .unwrap();
    assert_eq!(
        report
            .restored
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>(),
        vec![restored.clone()],
        "{report:?}"
    );
    assert!(
        report.missing.is_empty() && report.refused.is_empty(),
        "{report:?}"
    );
    assert_eq!(completed(&fleet.events(restored).await), 1);
    let other = fleet.events(untouched).await;
    assert_eq!(
        completed(&other),
        2,
        "the other org is untouched: {:?}",
        types(&other)
    );

    // The restored session carries on from the point; the executor that
    // ran its second turn drops what it remembered past the point.
    fleet.say(restored, "after the restore").await;
    fleet.turns(restored, 2).await;
    let events = fleet.events(restored).await;
    assert_eq!(completed(&events), 2, "{:?}", types(&events));
    let leftover: Vec<&serde_json::Value> = events
        .iter()
        .filter(|e| {
            e["data"]["message"]["content"]
                .to_string()
                .contains("second")
        })
        .collect();
    assert!(
        leftover.is_empty(),
        "the turn after the point is gone: {leftover:?}"
    );
    let sequences: Vec<i64> = events
        .iter()
        .filter_map(|e| e["sequence"].as_i64())
        .collect();
    assert!(
        sequences.windows(2).all(|w| w[1] == w[0] + 1),
        "one dense history: {sequences:?}"
    );
}
