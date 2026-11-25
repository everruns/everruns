//! Fault injection for the shard's durability promise (design.md, "Disaster
//! recovery"): the object store is slowed (`AGENTD_STORE_THROTTLE_MS`), and
//! a write answered as safe must still be in it: a tool that must not run
//! twice starts only once the bucket has its start, however slow the store
//! is, so a rebuild from the bucket alone knows the call started.

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
async fn a_started_call_is_in_the_slowed_bucket_before_it_runs() {
    let ledger = setup();
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let session = fleet.create_in("worker", 0).await;
    fleet.say(&session, "go").await;
    fleet
        .until("the tool started", || async {
            executions(ledger, &session) >= 1
        })
        .await;
    // The start waited for the slowed store before the call ran.
    assert!(ship_first(&fleet, "store").await >= 1);
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
    assert!(in_bucket, "the call ran before the bucket had its start");

    // Every disk of the shard goes: the bucket alone brings the start back.
    fleet.rebuild(0, None).await;
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
