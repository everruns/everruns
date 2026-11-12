//! The cells against a fake control plane over `/dp/v1` (data plane v2
//! step 6, "the CP protocol exercised early"): agents arrive as config,
//! turns are paid from allowances the control plane grants, and usage and
//! session summaries flow back on the feed. Then the protocol's failure
//! cases: lost acknowledgements, a control plane that is down, one rolled
//! back to an older epoch, expired grants, and a feed delayed across a
//! leader change.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::time::Duration;

use everruns_cell::dp::{ConfigAck, ConfigUpdate};
use everruns_cell_testkit::fake_cp::FakeCp;
use serde_json::{Value, json};
use support::{Fleet, types};

const ORG: &str = "org-a";

/// Revision `revision` of agent `id`: the test host's `worker`.
fn agent(id: &str, revision: u64) -> Value {
    let mut package = everruns_cell_testkit::definition("worker").unwrap();
    package["name"] = json!(id);
    package["instructions"] = json!(format!("Revision {revision}: do the work."));
    json!({ "kind": "Agent", "agent_id": id, "revision": revision, "package": package })
}

fn update(epoch: u64, version: u64, base: u64, changes: Vec<Value>) -> ConfigUpdate {
    ConfigUpdate {
        epoch,
        version,
        base_version: base,
        changes,
    }
}

async fn setup(balance: i64) -> (Fleet, FakeCp) {
    let cp = FakeCp::start().await.unwrap();
    cp.set_balance(ORG, balance);
    let mut fleet = Fleet::start_with(Some(cp.url.clone())).await;
    fleet.executor(1);
    let ack = cp
        .push_config(
            &fleet.base(),
            ORG,
            &update(1, 1, 0, vec![agent("helper", 1)]),
        )
        .await
        .unwrap();
    assert_eq!(ack, ConfigAck::Ok { applied_version: 1 });
    (fleet, cp)
}

async fn create(fleet: &Fleet, cell: u32) -> String {
    let created = fleet
        .post(
            "/v1/sessions",
            json!({ "org": ORG, "agent_ref": "helper", "cell": cell }),
        )
        .await;
    created["id"].as_str().unwrap().to_string()
}

fn generations(events: &[Value]) -> usize {
    types(events)
        .iter()
        .filter(|t| **t == "llm.generation")
        .count()
}

async fn feed_drained(fleet: &Fleet) -> bool {
    let mut pending = 0;
    for cell in [0, 1] {
        pending += fleet.stats(cell).await["feed_pending"]
            .as_u64()
            .unwrap_or(0);
    }
    pending == 0
}

#[tokio::test(flavor = "multi_thread")]
async fn agents_come_from_config_and_turns_bill_the_org() {
    let (fleet, cp) = setup(10_000_000).await;
    let first = create(&fleet, 0).await;
    // A new revision reaches new sessions only.
    let ack = cp
        .push_config(
            &fleet.base(),
            ORG,
            &update(1, 2, 1, vec![agent("helper", 2)]),
        )
        .await
        .unwrap();
    assert_eq!(ack, ConfigAck::Ok { applied_version: 2 });
    let second = create(&fleet, 1).await;
    for session in [&first, &second] {
        fleet.say(session, "go").await;
        fleet.turns(session, 1).await;
    }
    let old = fleet.get(&format!("/v1/sessions/{first}/agent")).await;
    let new = fleet.get(&format!("/v1/sessions/{second}/agent")).await;
    assert!(old.to_string().contains("Revision 1"), "{old}");
    assert!(new.to_string().contains("Revision 2"), "{new}");

    let mut calls = 0;
    for session in [&first, &second] {
        let events = fleet.events(session).await;
        assert!(
            types(&events).contains(&"turn.completed"),
            "{session}: {:?}",
            types(&events)
        );
        calls += generations(&events);
    }
    fleet
        .until("the feed is delivered", || feed_drained(&fleet))
        .await;
    // The step 7 gate's feed lag: from a record's commit to the control
    // plane, within the 2 s budget.
    let p99 = cp.feed_lag(99).expect("feed records arrived");
    println!(
        "feed lag p50 {:?} p99 {p99:?}",
        cp.feed_lag(50).unwrap_or_default()
    );
    assert!(p99 < Duration::from_secs(2), "feed lag p99 {p99:?}");
    let usage = cp.usage();
    assert_eq!(usage.len(), calls, "one usage record per provider call");
    assert!(usage.values().all(|(org, _)| org == ORG));
    let sessions = cp.sessions();
    for session in [&first, &second] {
        assert_eq!(sessions[session.as_str()].turns, 1, "{sessions:?}");
        assert_eq!(sessions[session.as_str()].status, "idle");
    }
    assert!(cp.config_applied().contains(&(ORG.to_string(), 2)));
    // Money only moved into grants: nothing was lost or made up.
    assert_eq!(cp.balance(ORG) + cp.outstanding(ORG), 10_000_000);

    // Removing the agent stops new sessions, not old ones.
    let removal = json!({ "kind": "Removal", "of": "agent", "id": "helper" });
    cp.push_config(&fleet.base(), ORG, &update(1, 3, 2, vec![removal]))
        .await
        .unwrap();
    let refused = fleet
        .http
        .post(format!("{}/v1/sessions", fleet.base()))
        .json(&json!({ "org": ORG, "agent_ref": "helper" }))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), 404);
    fleet.say(&first, "again").await;
    fleet.turns(&first, 2).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn config_keeps_order_through_retries_and_a_control_plane_rollback() {
    let (fleet, cp) = setup(10_000_000).await;
    let base = fleet.base();
    let push = |u: ConfigUpdate| {
        let cp = cp.clone();
        let base = base.clone();
        async move { cp.push_config(&base, ORG, &u).await.unwrap() }
    };
    assert_eq!(
        push(update(1, 2, 1, vec![agent("helper", 2)])).await,
        ConfigAck::Ok { applied_version: 2 }
    );
    assert_eq!(
        push(update(1, 2, 1, vec![agent("helper", 2)])).await,
        ConfigAck::AlreadyApplied { applied_version: 2 },
        "a retried update"
    );
    assert_eq!(
        push(update(1, 4, 3, vec![])).await,
        ConfigAck::NeedFullSnapshot { applied_version: 2 },
        "a gap"
    );
    // The control plane was restored from an older backup: a new epoch,
    // which starts only from a full snapshot.
    assert_eq!(
        push(update(2, 1, 1, vec![])).await,
        ConfigAck::NeedFullSnapshot { applied_version: 2 }
    );
    assert_eq!(
        push(update(2, 1, 0, vec![agent("helper", 1)])).await,
        ConfigAck::Ok { applied_version: 1 }
    );
    // A delayed update from before the restore is refused.
    assert_eq!(
        push(update(1, 3, 2, vec![agent("helper", 3)])).await,
        ConfigAck::StaleEpoch { epoch: 2 }
    );
    let position = fleet.get(&format!("/dp/v1/orgs/{ORG}/config")).await;
    assert_eq!(position, json!({ "epoch": 2, "applied_version": 1 }));
    // The restored control plane's snapshot is the config now (its
    // operator resolved what the data plane held beyond it first).
    let session = create(&fleet, 0).await;
    let definition = fleet.get(&format!("/v1/sessions/{session}/agent")).await;
    assert!(
        definition.to_string().contains("Revision 1"),
        "{definition}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn lost_acknowledgements_grant_and_bill_once() {
    let (fleet, cp) = setup(10_000_000).await;
    // The next replies are lost after the control plane acted on them:
    // the allowance request and the feed batches are each sent again.
    cp.lose_replies(4);
    let sessions = [create(&fleet, 0).await, create(&fleet, 1).await];
    let mut calls = 0;
    for session in &sessions {
        fleet.say(session, "go").await;
        fleet.turns(session, 1).await;
        calls += generations(&fleet.events(session).await);
    }
    fleet
        .until("the feed is delivered", || feed_drained(&fleet))
        .await;
    assert!(cp.requests() > cp.grants().len(), "requests were retried");
    assert_eq!(cp.grants().len(), 2, "one grant per cell, not per retry");
    assert_eq!(cp.usage().len(), calls);
    assert_eq!(cp.usage_repeats(), 0);
    assert_eq!(cp.balance(ORG) + cp.outstanding(ORG), 10_000_000);
}

#[tokio::test(flavor = "multi_thread")]
async fn with_the_control_plane_down_turns_wait_for_money() {
    let (fleet, cp) = setup(10_000_000).await;
    cp.set_down(true);
    let session = create(&fleet, 0).await;
    fleet.say(&session, "go").await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let view = fleet.get(&format!("/v1/sessions/{session}")).await;
    assert_eq!(view["turns_completed"], 0, "no money, no turn: {view}");
    assert!(
        !types(&fleet.events(&session).await).contains(&"turn.started"),
        "the turn waits, it does not fail"
    );
    cp.set_down(false);
    fleet.turns(&session, 1).await;
    assert!(types(&fleet.events(&session).await).contains(&"turn.completed"));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_exhausted_budget_ends_the_turn() {
    let (fleet, _cp) = setup(0).await;
    let session = create(&fleet, 0).await;
    fleet.say(&session, "go").await;
    fleet.turns(&session, 1).await;
    let events = fleet.events(&session).await;
    let kinds = types(&events);
    assert!(kinds.contains(&"budget.exhausted"), "{kinds:?}");
    assert!(!kinds.contains(&"llm.generation"), "{kinds:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn expired_grants_are_settled_and_replaced() {
    let (fleet, cp) = setup(10_000_000).await;
    cp.set_grant_ttl_ms(1_500);
    let session = create(&fleet, 0).await;
    fleet.say(&session, "go").await;
    fleet.turns(&session, 1).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    fleet.say(&session, "again").await;
    fleet.turns(&session, 2).await;
    let events = fleet.events(&session).await;
    assert_eq!(
        types(&events)
            .iter()
            .filter(|t| **t == "turn.completed")
            .count(),
        2
    );
    // The expired grant is reported closed and its rest returns.
    fleet
        .until("an expired grant is closed", || async {
            cp.grants().values().any(|g| g.closed)
        })
        .await;
    assert_eq!(cp.balance(ORG) + cp.outstanding(ORG), 10_000_000);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_delayed_feed_arrives_once_across_a_leader_change() {
    let (mut fleet, cp) = setup(10_000_000).await;
    let session = create(&fleet, 0).await;
    fleet.say(&session, "go").await;
    fleet.turns(&session, 1).await;
    // The feed backs up while the control plane is away...
    cp.set_down(true);
    fleet.say(&session, "again").await;
    fleet.turns(&session, 2).await;
    assert!(fleet.stats(0).await["feed_pending"].as_u64().unwrap() > 0);
    // ...and the leader holding it dies before it is delivered.
    let leader = fleet.leader(0).await;
    fleet.kill_node(0, leader);
    cp.set_down(false);
    fleet
        .until("the feed is delivered", || feed_drained(&fleet))
        .await;
    let calls = generations(&fleet.events(&session).await);
    assert_eq!(cp.usage().len(), calls);
    assert_eq!(cp.usage_repeats(), 0);
    assert_eq!(cp.sessions()[session.as_str()].turns, 2);
}

/// Step 8 gate: a config change while a turn runs leaves that turn, and its
/// session, on the revision it started with.
#[tokio::test(flavor = "multi_thread")]
async fn a_config_change_mid_turn_keeps_the_running_turns_revision() {
    let cp = FakeCp::start().await.unwrap();
    cp.set_balance(ORG, 10_000_000);
    let mut fleet = Fleet::start_with(Some(cp.url.clone())).await;
    fleet.executor(1);
    // Revision 1 is slow (`CELL_MODEL_DELAY_MS`, 3 s by default).
    let mut slow = everruns_cell_testkit::definition("slow").unwrap();
    slow["name"] = json!("helper");
    slow["instructions"] = json!("Revision 1: do the work.");
    let first = json!({ "kind": "Agent", "agent_id": "helper", "revision": 1, "package": slow });
    let ack = cp
        .push_config(&fleet.base(), ORG, &update(1, 1, 0, vec![first]))
        .await
        .unwrap();
    assert_eq!(ack, ConfigAck::Ok { applied_version: 1 });
    let session = create(&fleet, 0).await;
    fleet.say(&session, "go").await;
    fleet
        .until("the turn is running", || async {
            types(&fleet.events(&session).await).contains(&"turn.started")
        })
        .await;
    let ack = cp
        .push_config(
            &fleet.base(),
            ORG,
            &update(1, 2, 1, vec![agent("helper", 2)]),
        )
        .await
        .unwrap();
    assert_eq!(ack, ConfigAck::Ok { applied_version: 2 });
    assert!(
        !types(&fleet.events(&session).await).contains(&"turn.completed"),
        "the change landed while the turn ran"
    );
    fleet.turns(&session, 1).await;
    let events = fleet.events(&session).await;
    assert!(types(&events).contains(&"turn.completed"), "{:?}", types(&events));
    let running = fleet.get(&format!("/v1/sessions/{session}/agent")).await;
    assert!(running.to_string().contains("Revision 1"), "{running}");
    // Its next turn too; a new session gets revision 2.
    fleet.say(&session, "again").await;
    fleet.turns(&session, 2).await;
    let running = fleet.get(&format!("/v1/sessions/{session}/agent")).await;
    assert!(running.to_string().contains("Revision 1"), "{running}");
    let fresh = create(&fleet, 1).await;
    let fresh = fleet.get(&format!("/v1/sessions/{fresh}/agent")).await;
    assert!(fresh.to_string().contains("Revision 2"), "{fresh}");
}
