//! Completion-wait authorization, prompt wakeups and durable reconciliation.
use super::*;

/// Regression: the direct platform store's command ctx must carry the
/// event service — wait_for_idle probes terminal turn events through the
/// list_events command, and without it every subagent wait failed with
/// "Event service not configured".
#[tokio::test]
async fn platform_store_wait_for_idle_reaches_event_service() {
    use everruns_core::DEFAULT_ORG_ID;

    let adapters = test_adapters();
    let user_id = seed_platform_owner(&adapters.db, DEFAULT_ORG_ID, "waiter@example.com").await;
    let harness_id =
        seed_harness_for_platform_store(&adapters.db, DEFAULT_ORG_ID, "wait-harness", false).await;
    let session_id =
        seed_platform_session(&adapters.db, DEFAULT_ORG_ID, harness_id, Some(user_id)).await;
    let store = adapters
        .platform_store(DEFAULT_ORG_ID, session_id)
        .for_execution(session_id.uuid())
        .unwrap();

    // No terminal turn events exist, so a zero-timeout wait reports a
    // timeout — the point is it must NOT fail on a missing event service.
    let status = store
        .wait_for_idle(session_id, Some(0))
        .await
        .expect("wait_for_idle should reach the event service");
    assert!(status.starts_with("timeout"), "unexpected status: {status}");
}

/// Subscription precedes the durable probe, and a terminal notification wakes
/// the server wait without another half-second worker polling round trip.
#[tokio::test]
async fn platform_store_wait_for_idle_wakes_on_terminal_notification() {
    use everruns_contracts::typed_id::TurnId;
    use everruns_core::{DEFAULT_ORG_ID, EventContext, EventRequest, TurnCompletedData};
    let adapters = test_adapters();
    let user = seed_platform_owner(&adapters.db, DEFAULT_ORG_ID, "push-wait@example.com").await;
    let harness =
        seed_harness_for_platform_store(&adapters.db, DEFAULT_ORG_ID, "push-wait", false).await;
    let session = seed_platform_session(&adapters.db, DEFAULT_ORG_ID, harness, Some(user)).await;
    let store = adapters
        .platform_store(DEFAULT_ORG_ID, session)
        .for_execution(session.uuid())
        .unwrap();
    let waiter = tokio::spawn(async move { store.wait_for_idle(session, Some(5)).await });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    adapters
        .db
        .update_session(
            DEFAULT_ORG_ID,
            session,
            crate::storage::UpdateSession {
                status: Some("idle".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let started = std::time::Instant::now();
    adapters
        .event_service
        .emit(EventRequest::new(
            session,
            EventContext::empty(),
            TurnCompletedData {
                turn_id: TurnId::new(),
                iterations: 1,
                ..Default::default()
            },
        ))
        .await
        .unwrap();
    let status = tokio::time::timeout(std::time::Duration::from_millis(300), waiter)
        .await
        .expect("notification must beat the old 500ms poll")
        .unwrap()
        .unwrap();
    assert_eq!(status, "completed");
    assert!(started.elapsed() < std::time::Duration::from_millis(300));
}

#[tokio::test]
async fn platform_store_wait_for_idle_rejects_another_org() {
    use everruns_core::DEFAULT_ORG_ID;
    let adapters = test_adapters();
    let user = seed_platform_owner(&adapters.db, DEFAULT_ORG_ID, "scoped-wait@example.com").await;
    let harness =
        seed_harness_for_platform_store(&adapters.db, DEFAULT_ORG_ID, "scoped-wait", false).await;
    let session = seed_platform_session(&adapters.db, DEFAULT_ORG_ID, harness, Some(user)).await;
    let store = adapters.platform_store(DEFAULT_ORG_ID + 1, session);
    assert!(store.wait_for_idle(session, Some(0)).await.is_err());
}

#[tokio::test]
async fn platform_store_wait_for_idle_recovers_a_missed_notification() {
    use everruns_contracts::typed_id::TurnId;
    use everruns_core::{DEFAULT_ORG_ID, EventContext, EventRequest, TurnCompletedData};
    let adapters = test_adapters();
    let user = seed_platform_owner(&adapters.db, DEFAULT_ORG_ID, "lost-wait@example.com").await;
    let harness =
        seed_harness_for_platform_store(&adapters.db, DEFAULT_ORG_ID, "lost-wait", false).await;
    let session = seed_platform_session(&adapters.db, DEFAULT_ORG_ID, harness, Some(user)).await;
    let store = adapters
        .platform_store(DEFAULT_ORG_ID, session)
        .for_execution(session.uuid())
        .unwrap();
    let waiter = tokio::spawn(async move { store.wait_for_idle(session, Some(7)).await });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    adapters
        .db
        .update_session(
            DEFAULT_ORG_ID,
            session,
            crate::storage::UpdateSession {
                status: Some("idle".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    // Store the terminal event through another delivery instance: the waiter's
    // subscription misses it, as after a publish failure or replica-local loss.
    let isolated = EventService::new(
        adapters.db.clone(),
        crate::live_updates::event_delivery::EventDelivery::in_memory(),
    );
    isolated
        .emit(EventRequest::new(
            session,
            EventContext::empty(),
            TurnCompletedData {
                turn_id: TurnId::new(),
                iterations: 1,
                ..Default::default()
            },
        ))
        .await
        .unwrap();
    let status = tokio::time::timeout(std::time::Duration::from_secs(6), waiter)
        .await
        .expect("durable reconciliation after loss")
        .unwrap()
        .unwrap();
    assert_eq!(status, "completed");
}
