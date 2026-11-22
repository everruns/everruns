//! Fault injection for the shard's durability promise (design.md, "Disaster
//! recovery"): the object store is slowed (`AGENTD_STORE_THROTTLE_MS`) so
//! it is provably behind, and a write answered as safe must still survive
//! two lost zones. The direct check is the one a slowed store makes
//! possible: the bucket does not have the write yet, and the rebuild does.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use support::{Fleet, types};

/// The work tool's ledger and the slowed store, one for the whole binary:
/// the variables are process-wide.
fn setup() -> &'static std::path::Path {
    static LEDGER: std::sync::OnceLock<tempfile::NamedTempFile> = std::sync::OnceLock::new();
    LEDGER
        .get_or_init(|| {
            let file = tempfile::NamedTempFile::new().unwrap();
            // Set once, before any fleet starts.
            unsafe {
                std::env::set_var("AGENTD_LEDGER", file.path());
                std::env::set_var("AGENTD_WORK_MS", "3000");
                std::env::set_var("AGENTD_STORE_THROTTLE_MS", "400");
            }
            file
        })
        .path()
}

fn executions(ledger: &std::path::Path, session: &str) -> usize {
    std::fs::read_to_string(ledger)
        .unwrap_or_default()
        .lines()
        .filter(|line| *line == session)
        .count()
}

/// `agentd_shard_ship_first_total{path}` summed over shard 0's live nodes.
async fn ship_first(fleet: &Fleet, path: &str) -> u64 {
    let mut total = 0;
    for (_, addr) in fleet.addrs(0) {
        let Ok(response) = reqwest::get(format!("http://{addr}/metrics")).await else {
            continue;
        };
        let text = response.text().await.unwrap_or_default();
        total += text
            .lines()
            .filter(|l| {
                l.starts_with("agentd_shard_ship_first_total{")
                    && l.contains(&format!("path=\"{path}\""))
            })
            .filter_map(|l| l.rsplit(' ').next()?.parse::<u64>().ok())
            .sum::<u64>();
    }
    total
}

#[tokio::test(flavor = "multi_thread")]
async fn a_started_call_outlives_two_lost_zones_before_the_bucket_has_it() {
    let ledger = setup();
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let leader = fleet.leader(0).await;
    let survivor = (1..=3).find(|n| *n != leader).unwrap();
    let session = fleet.create_in("worker", 0).await;
    fleet.say(&session, "go").await;
    fleet
        .until("the tool started", || async {
            executions(ledger, &session) >= 1
        })
        .await;
    // Every voter held the start before the call ran, without waiting for
    // the slowed store.
    assert!(ship_first(&fleet, "replicas").await >= 1);

    // The store is behind: its newest state has not seen the call start.
    let store = fleet.store();
    let bucket = everruns_agentd::restore::state_at(&store, 0, None)
        .await
        .unwrap();
    let in_bucket = bucket
        .state
        .sessions
        .iter()
        .find(|(id, _)| id.to_string() == session)
        .is_some_and(|(_, s)| s.events.iter().any(|e| e.event_type == "tool.started"));
    assert!(
        !in_bucket,
        "the shipped log already had the start; the test proves nothing"
    );

    let rebuilt = fleet.rebuild_with_survivor(0, survivor).await;
    assert!(rebuilt.from_survivor > 0, "{rebuilt:?}");
    fleet.turns(&session, 1).await;
    let events = fleet.events(&session).await;
    let kinds = types(&events);
    assert!(kinds.contains(&"tool.started"), "{kinds:?}");
    assert_eq!(
        executions(ledger, &session),
        kinds.iter().filter(|t| **t == "tool.started").count(),
        "the call ran again after the rebuild: {kinds:?}"
    );
}
