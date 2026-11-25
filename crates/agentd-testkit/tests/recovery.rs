//! Step 9's disaster recovery against a live fleet (design.md, "Disaster
//! recovery"): a shard that loses two zones comes back from the object
//! store and the disk left in the third, without running an effect twice,
//! and a point-in-time restore takes a shard back to a moment before later
//! turns.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::time::{Duration, Instant};

use everruns_agentd::restore::FENCE_GAP;
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
                std::env::set_var("AGENTD_LEDGER", file.path());
                std::env::set_var("AGENTD_WORK_MS", "2000");
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

/// Event reads rotate across replicas, unlike the committed restore write.
/// Wait for the expected history and return that same sample: another GET
/// could hit a follower that has not applied the restore (or next turn) yet.
async fn completed_events(
    fleet: &Fleet,
    session: &str,
    turns: usize,
    input_id: Option<&str>,
) -> Vec<serde_json::Value> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let events = fleet.events(session).await;
        let has_input = input_id.is_none_or(|id| {
            events
                .iter()
                .any(|event| event["data"]["message"]["id"].as_str() == Some(id))
        });
        if completed(&events) == turns && has_input {
            return events;
        }
        fleet.check();
        assert!(
            Instant::now() < deadline,
            "{session} did not reach {turns} completed turns: {:?}",
            types(&events)
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Lose two zones of shard 0 while a tool that must not run twice is
/// running, keeping `survivor`'s disk, and check the rebuilt shard knows
/// the call started: it finishes the turn without running it again.
async fn two_zones_lost_mid_tool(fleet: &mut Fleet, ledger: &std::path::Path, survivor: u64) {
    let session = fleet.create_in("worker", 0).await;
    fleet.say(&session, "go").await;
    let lost_fence = fleet.fence(&session).await;
    fleet
        .until("the tool started", || async {
            executions(ledger, &session) >= 1
        })
        .await;
    let rebuilt = fleet.rebuild_with_survivor(0, survivor).await;
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

/// Every voter held the tool's start before it ran (the shipped log may
/// not have it yet): the disk left in the third zone brings it back.
#[tokio::test(flavor = "multi_thread")]
async fn a_shard_that_loses_two_zones_comes_back_from_the_store_and_the_survivor() {
    let ledger = ledger();
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let leader = fleet.leader(0).await;
    let survivor = (1..=3).find(|n| *n != leader).unwrap();
    two_zones_lost_mid_tool(&mut fleet, ledger, survivor).await;
}

/// A voter was down before the tool started, so its start was shipped to
/// the object store instead; then the two up-to-date zones go and the
/// lagging replica is the one left. The store has what it lacks.
#[tokio::test(flavor = "multi_thread")]
async fn two_zones_lost_while_the_survivor_lagged_still_keeps_every_started_call() {
    let ledger = ledger();
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let leader = fleet.leader(0).await;
    let lagging = (1..=3).find(|n| *n != leader).unwrap();
    fleet.kill_node(0, lagging);
    two_zones_lost_mid_tool(&mut fleet, ledger, lagging).await;
}

/// Every disk of shard 0 is lost seconds after a session was created and a
/// tool that must not run twice started, well inside the 10 s shipping
/// window: both were in the object store before they were answered, so the
/// rebuild from the store alone has them (the disaster drill at 50,000
/// sessions lost them while they waited only for every voter's disk).
#[tokio::test(flavor = "multi_thread")]
async fn every_disk_lost_keeps_answered_creates_and_started_calls() {
    let ledger = ledger();
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let quiet = fleet.create_in("worker", 0).await;
    let session = fleet.create_in("worker", 0).await;
    fleet.say(&session, "go").await;
    fleet
        .until("the tool started", || async {
            executions(ledger, &session) >= 1
        })
        .await;
    let rebuilt = fleet.rebuild(0, None).await;
    assert_eq!(rebuilt.incarnation, 2, "{rebuilt:?}");

    // The session created and never used is still there (a lost one is a
    // 404 until the read gives up).
    let record = fleet.get(&format!("/v1/sessions/{quiet}")).await;
    assert_eq!(record["id"].as_str(), Some(quiet.as_str()), "{record}");
    // The turn finishes, and the tool that had started did not run again.
    fleet.turns(&session, 1).await;
    let kinds = types(&fleet.events(&session).await);
    assert!(kinds.contains(&"tool.started"), "{kinds:?}");
    assert_eq!(
        executions(ledger, &session),
        kinds.iter().filter(|t| **t == "tool.started").count(),
        "every effect is a durable tool.started, and none ran twice: {kinds:?}"
    );
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
    // The leader ships every ten seconds; wait for turn 2 to reach the store,
    // so the restore really has something after the point to leave out.
    let store = fleet.store();
    fleet
        .until("turn 2 shipped", || async {
            everruns_agentd::restore::state_at(&store, 1, None)
                .await
                .is_ok_and(|r| r.state.clock_ms > point)
        })
        .await;
    // Nothing runs while the shard is taken back.
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
        // In every shard: the fleet spreads sessions, and without a control
        // plane nothing grants a shard money when one arrives.
        for shard in [0, 1] {
            assert_eq!(fleet.propose(shard, grant.clone()).await["result"], "ok");
        }
        let definition = everruns_agentd_testkit::definition("worker").unwrap();
        let created = fleet
            .post(
                "/v1/sessions",
                json!({ "agent": definition, "tenant": org, "shard": 1 }),
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
    // Wait for both sessions' completed second turns in the store. A shard
    // with neither session need not advance its clock past the restore point,
    // and an unrelated later command does not prove the turns were shipped.
    let store = fleet.store();
    fleet
        .until("turn 2 shipped", || async {
            let mut shipped = [false; 2];
            for shard in [0, 1] {
                if let Ok(restored) = everruns_agentd::restore::state_at(&store, shard, None).await
                {
                    for (i, session) in sessions.iter().enumerate() {
                        shipped[i] |= restored.state.sessions.iter().any(|(id, state)| {
                            id.to_string() == *session && state.turns_completed() == 2
                        });
                    }
                }
            }
            shipped.into_iter().all(|done| done)
        })
        .await;

    let gateway = everruns_agentd::gateway::Gateway::new(fleet.topology());
    gateway.refresh_registry().await;
    let report = everruns_agentd::restore::restore_org(&store, &gateway, "restored", point)
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
    // The restore is committed through each shard's log; a read served by a
    // node that has not applied it yet still shows turn 2. Wait for it.
    fleet
        .until("the restored session is back at the point", || async {
            completed(&fleet.events(restored).await) == 1
        })
        .await;
    let other = completed_events(&fleet, untouched, 2, None).await;
    assert_eq!(
        completed(&other),
        2,
        "the other org is untouched: {:?}",
        types(&other)
    );

    // The restored session carries on from the point; the executor that
    // ran its second turn drops what it remembered past the point.
    let input = fleet.say(restored, "after the restore").await;
    fleet.turns(restored, 2).await;
    // A stale pre-restore replica also has two completed turns. Require the
    // new input in this same history so that old branch cannot satisfy the wait.
    let events = completed_events(&fleet, restored, 2, Some(input["id"].as_str().unwrap())).await;
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
