//! Reporting facts from the listener feed: capability snapshots once per
//! turn, and the repair job agrees on what is already projected.

use super::*;
use crate::live_updates::event_delivery::EventDelivery;
use crate::services::EventService;
use crate::storage::test_database::test_session_row;
use everruns_contracts::typed_id::{MessageId, SessionId, TurnId};
use everruns_core::DEFAULT_ORG_ID;
use everruns_core::events::{
    CapabilityUsageData, CapabilityUsageKind, CapabilityUsageRecord, EventContext, EventRequest,
};

fn snapshot(session: SessionId, turn: Option<TurnId>) -> EventRequest {
    let record = |capability_id: &str, usage_kind, tool_name: Option<&str>| CapabilityUsageRecord {
        capability_id: capability_id.to_string(),
        capability_name: None,
        usage_kind,
        tool_name: tool_name.map(str::to_string),
        usage_count: Some(1),
        duration_ms: None,
    };
    let context = match turn {
        Some(turn) => EventContext::turn(turn, MessageId::new()),
        None => EventContext::empty(),
    };
    EventRequest::new(
        session,
        context,
        CapabilityUsageData {
            records: vec![
                record("cap-a", CapabilityUsageKind::Resolved, None),
                record("cap-a", CapabilityUsageKind::Exposed, Some("search")),
            ],
        },
    )
}

async fn fixture() -> (EventService, Arc<StorageBackend>, SessionId) {
    let db = Arc::new(StorageBackend::test_database());
    let service = EventService::with_listeners(
        db.clone(),
        EventDelivery::in_memory(),
        vec![Arc::new(ReportingProjectionListener::new(db.clone()))],
    );
    let session = db
        .create_session(test_session_row(DEFAULT_ORG_ID))
        .await
        .unwrap()
        .id;
    (service, db, session)
}

async fn capability_keys(db: &StorageBackend, session: SessionId) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT source_key FROM fact_capability_usage WHERE session_id = $1 ORDER BY source_key",
    )
    .bind(session.uuid())
    .fetch_all(db.pool())
    .await
    .unwrap()
}

#[tokio::test]
async fn a_turn_writes_its_capability_snapshot_once() {
    let (service, db, session) = fixture().await;
    let first = TurnId::new();
    let second = TurnId::new();
    // Three model calls in the first turn, one in the second.
    for turn in [first, first, first, second] {
        service.emit(snapshot(session, Some(turn))).await.unwrap();
    }
    service.listeners_settled().await;

    let keys = capability_keys(&db, session).await;
    assert_eq!(
        keys.len(),
        4,
        "two records per turn, not per call: {keys:?}"
    );
    assert!(keys.iter().all(|key| key.starts_with("turn:")));
    for turn in [first, second] {
        assert_eq!(
            keys.iter()
                .filter(|key| key.contains(&turn.to_string()))
                .count(),
            2
        );
    }
}

#[tokio::test]
async fn an_event_without_a_turn_keeps_its_own_rows() {
    let (service, db, session) = fixture().await;
    for _ in 0..2 {
        service.emit(snapshot(session, None)).await.unwrap();
    }
    service.listeners_settled().await;

    let keys = capability_keys(&db, session).await;
    assert_eq!(keys.len(), 4, "{keys:?}");
    assert!(keys.iter().all(|key| key.starts_with("event:")));
}

#[tokio::test]
async fn the_feed_writes_no_outbox_rows_and_repair_finds_nothing_missing() {
    let (service, db, session) = fixture().await;
    let turn = TurnId::new();
    let mut ids = Vec::new();
    for _ in 0..2 {
        ids.push(
            service
                .emit(snapshot(session, Some(turn)))
                .await
                .unwrap()
                .id,
        );
    }
    service.listeners_settled().await;

    let outbox: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM reporting_outbox WHERE source_type = 'event' AND source_id = ANY($1)",
    )
    .bind(
        ids.iter()
            .map(|id| id.uuid().to_string())
            .collect::<Vec<_>>(),
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(outbox, 0);

    // The second call's snapshot counts as projected: the turn already has
    // its rows, so the repair job enqueues neither event.
    let first_created: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT MIN(created_at) FROM events WHERE session_id = $1")
            .bind(session.uuid())
            .fetch_one(db.pool())
            .await
            .unwrap();
    sqlx::query(
        "UPDATE reporting_event_repair_cursor \
            SET last_event_created_at = $1, \
                last_event_id = '00000000-0000-0000-0000-000000000000' \
          WHERE singleton",
    )
    .bind(first_created - chrono::Duration::seconds(1))
    .execute(db.pool())
    .await
    .unwrap();
    let projector = PostgresReportingProjector::new(db.pool().clone());
    projector
        .repair_missing_event_projections(1000)
        .await
        .unwrap();
    let repaired: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM reporting_outbox WHERE source_type = 'event' AND source_id = ANY($1)",
    )
    .bind(
        ids.iter()
            .map(|id| id.uuid().to_string())
            .collect::<Vec<_>>(),
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(repaired, 0, "nothing of the turn is missing");

    // A turn whose rows are gone is found again.
    sqlx::query("DELETE FROM fact_capability_usage WHERE session_id = $1")
        .bind(session.uuid())
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query(
        "UPDATE reporting_event_repair_cursor \
            SET last_event_created_at = $1, \
                last_event_id = '00000000-0000-0000-0000-000000000000' \
          WHERE singleton",
    )
    .bind(first_created - chrono::Duration::seconds(1))
    .execute(db.pool())
    .await
    .unwrap();
    projector
        .repair_missing_event_projections(1000)
        .await
        .unwrap();
    let repaired: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM reporting_outbox WHERE source_type = 'event' AND source_id = ANY($1)",
    )
    .bind(
        ids.iter()
            .map(|id| id.uuid().to_string())
            .collect::<Vec<_>>(),
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(repaired, 2);
}
