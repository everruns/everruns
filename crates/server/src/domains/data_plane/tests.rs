//! The control plane's side of /dp/v1 on a real database, against a stub
//! data plane that answers config the way a cell does.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::routing::put;
use axum::{Json, Router};
use everruns_contracts::dataplane::{
    AllowanceRequest, ConfigAck, ConfigChange, ConfigUpdate, FeedBatch, FeedRecord,
    GrantSettlement, Record,
};
use everruns_contracts::dataplane_seal as seal;
use parking_lot::Mutex;
use serde_json::{Value, json};

use super::*;
use crate::domains::budgets::BudgetService;

const ORG: i64 = 2;

/// What the stub data plane saw.
#[derive(Default)]
struct Seen {
    enabled: Vec<dp::OrgEnable>,
    updates: Vec<(Instant, ConfigUpdate)>,
    applied: u64,
    epoch: u64,
    /// Answer the next incremental update with NEED_FULL_SNAPSHOT.
    want_full: bool,
}

type Stub = Arc<Mutex<Seen>>;

async fn stub() -> (String, Stub) {
    let seen: Stub = Default::default();
    async fn enable(State(seen): State<Stub>, Json(body): Json<dp::OrgEnable>) -> Json<Value> {
        let id = body.command_id.clone();
        seen.lock().enabled.push(body);
        Json(json!({ "command_id": id, "accepted": true }))
    }
    async fn config(
        State(seen): State<Stub>,
        Path((_org, _epoch, _version)): Path<(String, u64, u64)>,
        Json(update): Json<ConfigUpdate>,
    ) -> Json<ConfigAck> {
        let mut seen = seen.lock();
        seen.updates.push((Instant::now(), update.clone()));
        let full = update.base_version == 0;
        if update.epoch < seen.epoch {
            return Json(ConfigAck::StaleEpoch { epoch: seen.epoch });
        }
        if !full
            && (seen.want_full || update.base_version != seen.applied || update.epoch > seen.epoch)
        {
            seen.want_full = false;
            return Json(ConfigAck::NeedFullSnapshot {
                applied_version: seen.applied,
            });
        }
        seen.epoch = update.epoch;
        seen.applied = update.version;
        Json(ConfigAck::Ok {
            applied_version: update.version,
        })
    }
    let app = Router::new()
        .route("/dp/v1/orgs/{org}", put(enable))
        .route("/dp/v1/orgs/{org}/config/{epoch}/{version}", put(config))
        .with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await });
    (url, seen)
}

async fn control_plane() -> (ControlPlane, Stub, seal::Key) {
    let db = Arc::new(StorageBackend::test_database());
    let dp_key = seal::random_key();
    let link = Link {
        token: None,
        dp_key: Some(dp_key),
    };
    let cp = ControlPlane::new(db, None, link);
    let (url, seen) = stub().await;
    cp.db
        .dp_upsert_data_plane("dp-1", &url, &url)
        .await
        .unwrap();
    enable_org(&cp, ORG, "dp-1").await.unwrap();
    (cp, seen, dp_key)
}

fn kinds(update: &ConfigUpdate) -> Vec<String> {
    update
        .changes
        .iter()
        .map(|c| c["kind"].as_str().unwrap_or_default().to_owned())
        .collect()
}

#[tokio::test]
async fn a_v1_org_has_no_data_plane() {
    let db = Arc::new(StorageBackend::test_database());
    let cp = ControlPlane::new(db, None, Link::default());
    assert!(for_org(&cp, 3).await.unwrap().id().is_none());
    // A v1 org's agents never reach the outbox.
    agent_published(&cp.db, "ak_x", 3, "r1").await.unwrap();
    assert!(cp.db.dp_items_live(3).await.unwrap().is_empty());
}

#[tokio::test]
async fn an_org_is_enabled_then_gets_its_config_once_and_in_order() {
    let (cp, seen, dp_key) = control_plane().await;
    let epoch = cp.db.dp_epoch().await.unwrap();
    assert_eq!(outbox::deliver_due(&cp).await.unwrap(), 1);
    let org = cp.db.dp_org(ORG).await.unwrap().unwrap();
    {
        let seen = seen.lock();
        assert_eq!(seen.enabled.len(), 1);
        // The data plane can open the data key with its own key.
        let enable = &seen.enabled[0];
        let key = seal::unwrap_data_key(
            &dp_key,
            &org_public(ORG),
            &enable.data_key_id,
            &enable.wrapped_data_key,
        )
        .unwrap();
        assert_eq!(key, cp.data_key(&org).unwrap());
        let (_, first) = &seen.updates[0];
        assert_eq!(
            first.base_version, 0,
            "the first delivery is a full snapshot"
        );
        assert_eq!(kinds(first), ["Policy"]);
    }
    assert_eq!(
        outbox::deliver_due(&cp).await.unwrap(),
        0,
        "nothing is behind"
    );

    // Two edits coalesce into one update with its base.
    put_secret(&cp, ORG, "github", b"ghp_1").await.unwrap();
    put_secret(&cp, ORG, "github", b"ghp_2").await.unwrap();
    outbox::deliver_due(&cp).await.unwrap();
    let (secret, version) = {
        let seen = seen.lock();
        let (_, update) = seen.updates.last().unwrap();
        assert_eq!(update.base_version, 1);
        assert_eq!(kinds(update), ["Secret"]);
        let ConfigChange::Secret(sealed) = ConfigChange::parse(&update.changes[0]).unwrap() else {
            panic!("a secret");
        };
        (sealed, update.version)
    };
    assert_eq!(secret.revision, 2);
    let org = cp.db.dp_org(ORG).await.unwrap().unwrap();
    let opened = seal::unseal(&cp.data_key(&org).unwrap(), &org_public(ORG), &secret).unwrap();
    assert_eq!(opened, b"ghp_2");

    // A removal is sent as one; a data plane that lost its place gets a
    // full snapshot of what is live.
    remove(&cp.db, ORG, "secret", "github").await.unwrap();
    seen.lock().want_full = true;
    outbox::deliver_due(&cp).await.unwrap();
    {
        let seen = seen.lock();
        let n = seen.updates.len();
        assert_eq!(seen.updates[n - 2].1.base_version, version);
        assert_eq!(kinds(&seen.updates[n - 2].1), ["Removal"]);
        assert_eq!(seen.updates[n - 1].1.base_version, 0);
        assert_eq!(kinds(&seen.updates[n - 1].1), ["Policy"]);
    }

    // A new epoch (a restore) sends a full snapshot in it.
    let next = cp.db.dp_bump_epoch().await.unwrap();
    assert_eq!(next, epoch + 1);
    outbox::deliver_due(&cp).await.unwrap();
    let seen = seen.lock();
    let (_, last) = seen.updates.last().unwrap();
    assert_eq!((last.epoch, last.base_version), (next as u64, 0));
}

/// The gate's propagation number: an edit reaches the data plane through
/// NOTIFY, well under a second.
#[tokio::test]
async fn config_reaches_the_data_plane_in_under_a_second() {
    let (cp, seen, _) = control_plane().await;
    tokio::spawn(outbox::run_dispatcher(cp.clone()));
    let wait_for = |n: usize| {
        let seen = seen.clone();
        async move {
            let deadline = Instant::now() + Duration::from_secs(10);
            while seen.lock().updates.len() < n {
                assert!(Instant::now() < deadline, "no delivery");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            seen.lock().updates[n - 1].0
        }
    };
    wait_for(1).await;
    let mut lags = Vec::new();
    for i in 0..40 {
        let n = seen.lock().updates.len();
        let started = Instant::now();
        put_secret(&cp, ORG, &format!("s{i}"), b"x").await.unwrap();
        let arrived = wait_for(n + 1).await;
        lags.push(arrived.duration_since(started));
    }
    lags.sort();
    let p99 = lags[lags.len() * 99 / 100 - 1];
    println!(
        "config propagation p50 {:?} p99 {:?}",
        lags[lags.len() / 2],
        p99
    );
    assert!(p99 < Duration::from_secs(1), "p99 {p99:?}");
}

fn record(index: u64, record: Record) -> FeedRecord {
    FeedRecord {
        index,
        ordinal: 0,
        org_id: org_public(ORG),
        at_unix_ms: chrono::Utc::now().timestamp_millis(),
        record,
    }
}

fn usage(attempt: &str, cost: i64) -> Record {
    Record::Usage {
        session_id: "s1".into(),
        turn_id: "t1".into(),
        attempt_id: attempt.into(),
        unresolved: false,
        agent_id: "ak_1".into(),
        agent_hash: String::new(),
        provider: "openrouter".into(),
        model: "muse-1.3-contributor".into(),
        input_tokens: 10,
        output_tokens: 5,
        cached_tokens: 0,
        cost_micros: cost,
        kind: "model".into(),
    }
}

fn summary(version: u64, turns: u64) -> Record {
    Record::Session {
        session_id: "s1".into(),
        session_version: version,
        agent_id: "ak_1".into(),
        title: "A session".into(),
        status: "idle".into(),
        created_unix_ms: 0,
        last_activity_unix_ms: chrono::Utc::now().timestamp_millis(),
        turns,
        cost_micros: 0,
    }
}

#[tokio::test]
async fn the_feed_is_stored_once_through_retries_and_a_replay() {
    let (cp, _, _) = control_plane().await;
    let budgets = BudgetService::new(cp.db.clone());
    let batch = |incarnation: u64, records: Vec<FeedRecord>| FeedBatch {
        data_plane: "dp-1".into(),
        source: "cell-0".into(),
        incarnation,
        records,
    };
    let first = batch(
        1,
        vec![
            record(1, usage("a1", 1_000)),
            record(2, summary(2, 1)),
            record(
                3,
                Record::OrgEvent {
                    kind: "suspended".into(),
                    detail: String::new(),
                },
            ),
        ],
    );
    let ack = feed::ingest(&cp, &budgets, first.clone()).await.unwrap();
    assert_eq!(ack.stored_through, (3, 0));
    // The same batch again (a lost acknowledgement), then a replay under
    // another incarnation: positions and attempt ids keep it once.
    feed::ingest(&cp, &budgets, first).await.unwrap();
    let replay = batch(
        2,
        vec![record(1, usage("a1", 1_000)), record(2, summary(1, 0))],
    );
    feed::ingest(&cp, &budgets, replay).await.unwrap();
    let journaled: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM usage_journal WHERE org_id = $1 AND source_type = 'data_plane'",
    )
    .bind(ORG)
    .fetch_one(cp.db.pool())
    .await
    .unwrap();
    assert_eq!(journaled, 1);
    let feed_cost: i64 =
        sqlx::query_scalar("SELECT feed_cost_micros FROM dp_orgs WHERE org_id = $1")
            .bind(ORG)
            .fetch_one(cp.db.pool())
            .await
            .unwrap();
    assert_eq!(feed_cost, 1_000);
    let sessions = cp.db.dp_session_summaries(ORG, 10).await.unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(
        sessions[0].3, 1,
        "an older summary never replaces a newer one"
    );
    let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM dp_org_events WHERE org_id = $1")
        .bind(ORG)
        .fetch_one(cp.db.pool())
        .await
        .unwrap();
    assert_eq!(events, 1);
}

async fn org_policy(cp: &ControlPlane, limit_usd: f64) {
    cp.db
        .create_budget(crate::storage::CreateBudgetRow {
            org_id: ORG,
            subject_type: "org".into(),
            subject_id: org_public(ORG),
            currency: "usd".into(),
            limit: limit_usd,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();
}

fn ask(id: &str, want: i64, settlements: Vec<GrantSettlement>) -> AllowanceRequest {
    AllowanceRequest {
        request_id: id.into(),
        cell: "cell-0".into(),
        incarnation: 1,
        want_micros: want,
        settlements,
    }
}

#[tokio::test]
async fn a_retried_request_gets_the_same_grant_and_a_suspended_org_none() {
    let (cp, _, _) = control_plane().await;
    let first = allowances::answer(&cp, ORG, &ask("r1", 500_000, vec![]))
        .await
        .unwrap();
    assert_eq!(first.granted_micros, 500_000, "no policy: no cap");
    let again = allowances::answer(&cp, ORG, &ask("r1", 500_000, vec![]))
        .await
        .unwrap();
    assert_eq!(again, first);
    cp.db.dp_set_org_state(ORG, "suspended").await.unwrap();
    let refused = allowances::answer(&cp, ORG, &ask("r2", 500_000, vec![]))
        .await
        .unwrap();
    assert_eq!(refused.granted_micros, 0);
    assert_eq!(refused.reason.as_deref(), Some("suspended"));
}

/// The gate's money check: cells asking at once, retrying, settling late
/// and in any order never get more than the org's policy allows.
#[tokio::test]
async fn allowances_never_overspend_under_concurrency_and_retries() {
    let (cp, _, _) = control_plane().await;
    org_policy(&cp, 1.0).await; // $1 = 1_000_000 micros
    let cap = 1_000_000;
    let budgets = BudgetService::new(cp.db.clone());
    let mut tasks = tokio::task::JoinSet::new();
    for cell in 0..8 {
        let cp = cp.clone();
        tasks.spawn(async move {
            let mut grants: Vec<(String, i64, i64)> = Vec::new(); // (id, granted, used)
            for n in 0..12 {
                // Settle what was spent so far; close every other grant.
                let settlements = grants
                    .iter()
                    .enumerate()
                    .map(|(i, (id, _, used))| GrantSettlement {
                        grant_id: id.clone(),
                        used_micros: *used,
                        reserved_micros: 0,
                        closed: i % 2 == 0,
                    })
                    .collect();
                let request = ask(&format!("c{cell}-{n}"), 150_000, settlements);
                let grant = allowances::answer(&cp, ORG, &request).await.unwrap();
                // A lost reply: the retry must be the same grant.
                let retry = allowances::answer(&cp, ORG, &request).await.unwrap();
                assert_eq!(retry, grant);
                if grant.granted_micros > 0 {
                    // Spend most of it.
                    grants.push((
                        grant.grant_id,
                        grant.granted_micros,
                        grant.granted_micros * 3 / 4,
                    ));
                }
            }
            grants
        });
    }
    let mut all = Vec::new();
    while let Some(grants) = tasks.join_next().await {
        all.extend(grants.unwrap());
    }
    // Whatever the cells could spend: every grant whole (open grants could
    // still be spent), never above the cap.
    let ledger = cp.db.dp_grants(ORG).await.unwrap();
    let exposure: i64 = ledger
        .iter()
        .map(|g| {
            if g.closed {
                g.used_micros
            } else {
                g.granted_micros
            }
        })
        .sum();
    assert!(exposure <= cap, "exposure {exposure} over the cap {cap}");
    // The feed then charges what was spent: the policy is debited, and the
    // ledger still never lets more out.
    for (i, (_, _, used)) in all.iter().enumerate() {
        let batch = FeedBatch {
            data_plane: "dp-1".into(),
            source: "cell-x".into(),
            incarnation: 1,
            records: vec![record(i as u64 + 1, usage(&format!("u{i}"), *used))],
        };
        feed::ingest(&cp, &budgets, batch).await.unwrap();
    }
    let more = allowances::answer(&cp, ORG, &ask("after", cap, vec![]))
        .await
        .unwrap();
    let spent: i64 = all.iter().map(|(_, _, used)| used).sum();
    let open: i64 = cp
        .db
        .dp_grants(ORG)
        .await
        .unwrap()
        .iter()
        .filter(|g| !g.closed && g.grant_id != more.grant_id)
        .map(|g| g.granted_micros - g.used_micros)
        .sum();
    assert!(
        spent + open + more.granted_micros <= cap,
        "spent {spent} + open {open} + new {} over {cap}",
        more.granted_micros
    );
}

#[tokio::test]
async fn an_org_token_verifies_with_the_published_keys() {
    let (cp, _, _) = control_plane().await;
    let (token, exp) = tokens::issue(&cp, ORG, "user-1").await.unwrap();
    assert!(exp > chrono::Utc::now().timestamp());
    let jwks: jsonwebtoken::jwk::JwkSet =
        serde_json::from_value(tokens::jwks(&cp).await.unwrap()).unwrap();
    let kid = jsonwebtoken::decode_header(&token).unwrap().kid.unwrap();
    let key = jsonwebtoken::DecodingKey::from_jwk(jwks.find(&kid).unwrap()).unwrap();
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::EdDSA);
    validation.validate_aud = false;
    let claims = jsonwebtoken::decode::<dp::OrgTokenClaims>(&token, &key, &validation)
        .unwrap()
        .claims;
    assert_eq!(claims.org, org_public(ORG));
    assert_eq!(claims.dp, "dp-1");
    assert_eq!(claims.scope, tokens::SESSIONS_SCOPE);
    // One key, however many tokens.
    tokens::issue(&cp, ORG, "user-2").await.unwrap();
    assert_eq!(cp.db.dp_signing_keys().await.unwrap().len(), 1);
    // An org that is not on a data plane gets none.
    assert!(tokens::issue(&cp, 3, "user-1").await.is_err());
}

#[test]
fn org_ids_round_trip_and_strangers_do_not_parse() {
    assert_eq!(org_internal(&org_public(ORG)), Some(ORG));
    assert_eq!(org_internal("org_nonsense"), None);
    assert_eq!(org_internal(""), None);
}
