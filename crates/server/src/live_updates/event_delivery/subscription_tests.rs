//! Subscription ordering and SSE isolation regressions.

use super::tests::{TestNatsServer, make_test_event};
use super::*;

#[tokio::test]
async fn nats_subscription_filters_other_sessions_and_preserves_order() {
    let broker = TestNatsServer::start().await;
    let publisher = NatsEventDelivery::connect(&broker.url()).await.unwrap();
    let session = Uuid::new_v4();
    let mut subscriber = publisher.subscribe(session).await.unwrap();
    let other = make_test_event(Uuid::new_v4());
    publisher.publish(&other).await.unwrap();
    let expected: Vec<_> = (0..3).map(|_| make_test_event(session)).collect();
    for event in &expected {
        publisher.publish(event).await.unwrap();
    }
    for event in expected {
        let received = tokio::time::timeout(std::time::Duration::from_secs(5), subscriber.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received.session_id.uuid(), session);
        assert_eq!(received.id, event.id);
    }
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), subscriber.recv())
            .await
            .is_err(),
        "other-session events and duplicates must not reach this subscription"
    );
    publisher.shutdown().await;
}

#[tokio::test]
async fn nats_sse_reconnect_filters_spoofed_payloads_and_delivers_completion() {
    use crate::api::events::{SessionEventStream, session_event_sse, sse_frame};
    use crate::api::sse::{SseConnectionLimits, SseConnectionTracker};
    use crate::services::EventService;
    use crate::storage::StorageBackend;
    use axum::response::IntoResponse;
    use everruns_core::{EventContext, EventRequest, OutputMessageCompletedData, RuntimeMessage};
    use futures::StreamExt;

    let mut broker = TestNatsServer::start().await;
    let publisher = Arc::new(NatsEventDelivery::connect(&broker.url()).await.unwrap());
    let db = Arc::new(StorageBackend::test_database());
    let session = db.create_test_session().await;
    let service = Arc::new(EventService::new(
        db,
        EventDelivery::Nats(publisher.clone()),
    ));
    broker.restart(&publisher).await;
    let tracker = Arc::new(SseConnectionTracker::new(SseConnectionLimits {
        global_max: 10,
        per_org_max: 10,
        per_session_max: 1,
    }));
    let response = session_event_sse(SessionEventStream {
        event_service: service.clone(),
        session_id: session.uuid(),
        since_id: None,
        after_sequence: None,
        filter_types: Vec::new(),
        exclude_types: Vec::new(),
        guard: tracker.try_acquire(1, session.uuid()).unwrap(),
        render: Arc::new(|event, retry| {
            Some(sse_frame(
                event,
                serde_json::to_string(event).unwrap(),
                retry,
            ))
        }),
    })
    .await;
    let mut body = response.into_response().into_body().into_data_stream();
    let connected = body.next().await.unwrap().unwrap();
    assert!(
        std::str::from_utf8(&connected)
            .unwrap()
            .contains("event: connected")
    );

    // A trusted publisher can name any subject. The SSE boundary must
    // still refuse a different session in the envelope and malformed data.
    let foreign = make_test_event(Uuid::new_v4());
    for (subject, payload) in [
        (
            format!("events.{}", session.uuid()),
            serde_json::to_vec(&foreign).unwrap(),
        ),
        (format!("events.{}", session.uuid()), b"not-json".to_vec()),
        (
            format!("events.{}.extra", session.uuid()),
            serde_json::to_vec(&foreign).unwrap(),
        ),
    ] {
        publisher
            .client
            .publish(subject, payload.into())
            .await
            .unwrap();
    }
    let delta = service
        .emit(EventRequest::new(
            session,
            EventContext::empty(),
            make_test_event(session.uuid()).data,
        ))
        .await
        .unwrap();
    let completed = service
        .emit(EventRequest::new(
            session,
            EventContext::empty(),
            OutputMessageCompletedData::new(RuntimeMessage::assistant("hello")),
        ))
        .await
        .unwrap();
    for event in [&delta, &completed] {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(5), body.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let text = std::str::from_utf8(&frame).unwrap();
        assert!(
            text.contains(&format!("event: {}", event.event_type)),
            "{text}"
        );
        assert!(text.contains(&event.id.to_string()), "{text}");
        assert!(!text.contains(&foreign.id.to_string()), "{text}");
        assert_eq!(text.contains("\nid:"), event.sequence.is_some(), "{text}");
    }
    let durable = service
        .list(session.uuid(), None, None, &[], &[], None, None)
        .await
        .unwrap();
    assert_eq!(durable.len(), 1, "ephemeral deltas stay out of PostgreSQL");
    assert_eq!(durable[0].id, completed.id);
    drop(body);
    tracker
        .try_acquire(1, session.uuid())
        .expect("disconnect releases its slot");
    publisher.shutdown().await;
}
