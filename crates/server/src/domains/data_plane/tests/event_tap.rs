//! The event tap's control plane half (`event_tap.rs`), on a real
//! database: the org's policy asks for what it uses, and a tapped event
//! reaches observers once, for the org's own sessions only.

use std::sync::Arc;

use everruns_contracts::typed_id::{ObserverId, SessionId, TurnId};
use everruns_core::TurnCompletedData;
use everruns_core::events::{Event, EventContext};

use super::*;
use crate::domains::data_plane::event_tap;
use crate::domains::observers::record::{ObserverScope, ObserverScorerConfig, ScorerMethod};
use crate::storage::{CreateObserverRow, ListTraceScoresParams};

async fn policy_events(cp: &ControlPlane) -> Vec<String> {
    let body = cp
        .db
        .dp_item(ORG, "policy", "policy")
        .await
        .unwrap()
        .and_then(|item| item.body)
        .unwrap();
    match ConfigChange::parse(&body) {
        Some(ConfigChange::Policy(policy)) => policy.events,
        other => panic!("{other:?}"),
    }
}

fn observer() -> CreateObserverRow {
    CreateObserverRow {
        system_kind: None,
        public_id: ObserverId::from_uuid(uuid::Uuid::now_v7()).to_string(),
        name: "every turn".into(),
        description: None,
        match_config: json!({}),
        sampling_rate: 1.0,
        scorers: serde_json::to_value(vec![ObserverScorerConfig {
            key: "greets".into(),
            scope: ObserverScope::Turn,
            tools: None,
            method: ScorerMethod::Rule {
                rule: crate::domains::evals::record::Scorer::Contains {
                    text: "hello".into(),
                    weight: 1.0,
                },
            },
        }])
        .unwrap(),
        alert: None,
    }
}

/// Counts what it is handed, under the trace export's name.
#[derive(Default)]
struct Counting(std::sync::atomic::AtomicUsize);

#[async_trait::async_trait]
impl everruns_core::EventListener for Counting {
    async fn on_event(&self, _event: &Event) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    fn name(&self) -> &'static str {
        "CustomerTracing"
    }
}

fn turn_completed(session: SessionId) -> Value {
    let turn = TurnId::new();
    let mut event = Event::new(
        session,
        EventContext::turn(turn, everruns_contracts::typed_id::MessageId::new()),
        serde_json::from_value::<TurnCompletedData>(
            json!({ "turn_id": turn, "iterations": 2, "duration_ms": 1200 }),
        )
        .unwrap(),
    );
    event.sequence = Some(7);
    serde_json::to_value(event).unwrap()
}

fn tapped(index: u64, org: i64, session: SessionId, event: Value) -> FeedRecord {
    FeedRecord {
        index,
        ordinal: 0,
        org_id: org_public(org),
        at_unix_ms: chrono::Utc::now().timestamp_millis(),
        labels: Default::default(),
        record: Record::Event {
            session_id: session.to_string(),
            event,
        },
    }
}

#[tokio::test]
async fn an_org_asks_for_what_it_uses_and_its_events_are_scored_once() {
    let (cp, _, _) = control_plane().await;
    let harness = harness(&cp.db, ORG).await;
    let mut row = crate::storage::test_database::test_session_row(ORG);
    row.harness_id = Some(harness);
    let session = cp.db.create_session(row).await.unwrap().id;
    let strangers = cp
        .db
        .create_session(crate::storage::test_database::test_session_row(3))
        .await
        .unwrap()
        .id;

    // An active observer adds its types to the policy, the moment it is
    // saved.
    let created = cp.db.create_observer(ORG, observer()).await.unwrap();
    event_tap::resync(&cp.db, ORG).await;
    let events = policy_events(&cp).await;
    assert!(
        events.iter().any(|t| t == "session.idled"),
        "an observer's own type: {events:?}"
    );
    let policy = event_tap::policy(&cp.db, ORG).await.unwrap();
    assert_eq!(
        events,
        event_tap::wanted(&cp.db, ORG, &policy).await.unwrap()
    );

    // A tapped turn is scored once, retries and all; one naming another
    // org's session is dropped.
    let listener: Arc<dyn everruns_core::EventListener> = Arc::new(
        crate::domains::observers::ObserverMatchListener::new(cp.db.clone(), Default::default()),
    );
    let counting = Arc::new(Counting::default());
    let budgets = BudgetService::new(cp.db.clone());
    let event = turn_completed(session);
    let source = format!("shard-tap-{}", uuid::Uuid::new_v4().simple());
    let batch = FeedBatch {
        source: source.clone(),
        data_plane: "dp-1".into(),
        incarnation: 1,
        records: vec![
            tapped(1, ORG, session, event.clone()),
            tapped(2, ORG, strangers, turn_completed(strangers)),
        ],
    };
    for _ in 0..2 {
        let ack = feed::ingest(
            &cp,
            &budgets,
            &[listener.clone(), counting.clone()],
            batch.clone(),
        )
        .await
        .unwrap();
        assert_eq!(ack.stored_through, (2, 0));
    }
    let scores = cp
        .db
        .list_trace_scores(
            ORG,
            ListTraceScoresParams {
                observer_id: created.id,
                session_id: Some(session.uuid()),
                limit: 10,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(scores.len(), 1, "scored once: {scores:?}");
    assert_eq!(
        counting.0.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "once, and never another org's session"
    );
    cp.db.delete_observer(ORG, created.id).await.unwrap();
}

/// The warehouse export: every event of the org, in hourly files named by
/// feed position, so a retried batch writes its own file again.
#[tokio::test]
async fn a_warehouse_org_gets_every_event_in_hourly_files() {
    let (plain, _, _) = control_plane().await;
    let refused = warehouse::set(&plain.db, ORG, true).await;
    assert!(refused.is_err(), "no object storage");
    let store: crate::storage::blob_store::SharedBlobStore =
        Arc::new(crate::storage::blob_store::ObjectStoreBlobStore::in_memory());
    let (cp, _, _) =
        control_plane_on(StorageBackend::test_database().with_blob_store(Some(store.clone())))
            .await;
    warehouse::set(&cp.db, ORG, true).await.unwrap();
    let policy = event_tap::policy(&cp.db, ORG).await.unwrap();
    assert_eq!(policy.events, vec!["*".to_string()]);

    let mut row = crate::storage::test_database::test_session_row(ORG);
    row.harness_id = Some(harness(&cp.db, ORG).await);
    let session = cp.db.create_session(row).await.unwrap().id;
    let event = turn_completed(session);
    let source = format!("shard-wh-{}", uuid::Uuid::new_v4().simple());
    let batch = FeedBatch {
        source: source.clone(),
        data_plane: "dp-1".into(),
        incarnation: 1,
        records: vec![tapped(5, ORG, session, event.clone())],
    };
    let budgets = BudgetService::new(cp.db.clone());
    feed::ingest(&cp, &budgets, &[], batch).await.unwrap();
    let hour = chrono::DateTime::parse_from_rfc3339(event["ts"].as_str().unwrap())
        .unwrap()
        .format("date=%Y-%m-%d/hour=%H")
        .to_string();
    let key = warehouse::key(&org_public(ORG), &hour, &source, 1, (5, 0));
    let gz = store.get(&key).await.unwrap().expect("the hour's file");
    let mut text = String::new();
    std::io::Read::read_to_string(&mut flate2::read::GzDecoder::new(&gz[..]), &mut text).unwrap();
    let line: Value = serde_json::from_str(text.trim()).unwrap();
    assert_eq!(line["id"], event["id"]);

    warehouse::set(&cp.db, ORG, false).await.unwrap();
    let policy = event_tap::policy(&cp.db, ORG).await.unwrap();
    assert!(!policy.events.contains(&"*".to_string()), "{policy:?}");
}
