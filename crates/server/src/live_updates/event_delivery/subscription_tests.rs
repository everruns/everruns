//! Subscription latency, failure and SSE isolation regressions.

use super::tests::{TestNatsServer, make_test_event};
use super::*;

/// Loopback-only transport fault/latency injection; never contacts an external broker.
struct TestProxy {
    url: String,
    drop_responses: Arc<std::sync::atomic::AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}

impl TestProxy {
    async fn start(port: u16, delay: std::time::Duration) -> Self {
        use std::sync::atomic::{AtomicBool, Ordering};
        use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

        async fn forward(
            mut reader: impl AsyncRead + Unpin,
            mut writer: impl AsyncWrite + Unpin,
            delay: std::time::Duration,
            drop_bytes: Arc<AtomicBool>,
        ) -> std::io::Result<()> {
            let mut buffer = [0; 16_384];
            loop {
                let length = reader.read(&mut buffer).await?;
                if length == 0 {
                    return writer.shutdown().await;
                }
                if drop_bytes.load(Ordering::SeqCst) {
                    continue;
                }
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
                writer.write_all(&buffer[..length]).await?;
            }
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("nats://{}", listener.local_addr().unwrap());
        let drop_responses = Arc::new(AtomicBool::new(false));
        let drop_upstream = drop_responses.clone();
        let task = tokio::spawn(async move {
            let (downstream, _) = listener.accept().await.unwrap();
            let upstream = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            let (down_read, down_write) = downstream.into_split();
            let (up_read, up_write) = upstream.into_split();
            let _ = tokio::try_join!(
                forward(down_read, up_write, delay, Arc::new(AtomicBool::new(false))),
                forward(up_read, down_write, delay, drop_upstream),
            );
        });
        Self {
            url,
            drop_responses,
            task,
        }
    }
}

impl Drop for TestProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[tokio::test]
async fn nats_subscription_does_not_retry_an_uncertain_create_timeout() {
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    let broker = TestNatsServer::start().await;
    let proxy = TestProxy::start(broker.port, Duration::ZERO).await;
    let client = async_nats::ConnectOptions::new()
        .request_timeout(Some(Duration::from_millis(250)))
        .connect(&proxy.url)
        .await
        .unwrap();
    let publisher = NatsEventDelivery {
        jetstream: async_nats::jetstream::new(client.clone()),
        client,
    };
    publisher.ensure_stream().await.unwrap();
    proxy.drop_responses.store(true, Ordering::SeqCst);
    assert!(publisher.subscribe(Uuid::new_v4()).await.is_err());
    proxy.drop_responses.store(false, Ordering::SeqCst);
    let stream = publisher
        .jetstream
        .get_stream(NATS_STREAM_NAME)
        .await
        .unwrap();
    assert_eq!(
        stream.cached_info().state.consumer_count,
        1,
        "the consumer was created despite the lost reply; retrying would create another"
    );
    publisher.shutdown().await;
}

#[tokio::test]
async fn nats_subscription_does_not_fetch_stream_info_on_the_healthy_path() {
    use futures::StreamExt;

    let broker = TestNatsServer::start().await;
    let publisher = NatsEventDelivery::connect(&broker.url()).await.unwrap();
    let mut lookups = publisher
        .client
        .subscribe("$JS.API.STREAM.INFO.EVENTS")
        .await
        .unwrap();
    publisher.client.flush().await.unwrap();
    let _subscriber = publisher.subscribe(Uuid::new_v4()).await.unwrap();
    publisher.client.flush().await.unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), lookups.next())
            .await
            .is_err(),
        "a healthy SSE subscription needs only consumer creation, not stream discovery"
    );
    publisher.shutdown().await;
}

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
async fn nats_subscription_does_not_repair_a_consumer_limit_rejection() {
    let broker = TestNatsServer::start().await;
    let publisher = NatsEventDelivery::connect(&broker.url()).await.unwrap();
    let mut config = publisher
        .jetstream
        .get_stream(NATS_STREAM_NAME)
        .await
        .unwrap()
        .cached_info()
        .config
        .clone();
    config.max_consumers = 1;
    publisher.jetstream.update_stream(&config).await.unwrap();
    let _first = publisher.subscribe(Uuid::new_v4()).await.unwrap();
    assert!(publisher.subscribe(Uuid::new_v4()).await.is_err());
    let stream = publisher
        .jetstream
        .get_stream(NATS_STREAM_NAME)
        .await
        .unwrap();
    assert_eq!(stream.cached_info().config.max_consumers, 1);
    assert_eq!(stream.cached_info().state.consumer_count, 1);
    publisher.shutdown().await;
}

#[tokio::test]
async fn nats_subscription_setup_benchmark() {
    use std::time::{Duration, Instant};
    let broker = TestNatsServer::start().await;
    let proxy = TestProxy::start(broker.port, Duration::from_millis(10)).await;
    let publisher = NatsEventDelivery::connect(&proxy.url).await.unwrap();
    let mut baseline = Vec::new();
    let mut optimized = Vec::new();
    for iteration in 0..20 {
        // Alternate order to avoid always giving one path the warm socket.
        for legacy in [iteration % 2 == 0, iteration % 2 != 0] {
            let started = Instant::now();
            if legacy {
                publisher.ensure_stream().await.unwrap();
            }
            let subscription = publisher.subscribe(Uuid::new_v4()).await.unwrap();
            let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
            if legacy {
                baseline.push(elapsed_ms);
            } else {
                optimized.push(elapsed_ms);
            }
            drop(subscription);
        }
    }
    baseline.sort_by(f64::total_cmp);
    optimized.sort_by(f64::total_cmp);
    println!(
        "{}",
        serde_json::json!({
            "benchmark": "nats_sse_subscription_setup",
            "injected_delay_ms_per_direction": 10,
            "samples_per_path": baseline.len(),
            "baseline_median_ms": (baseline[9] + baseline[10]) / 2.0,
            "optimized_median_ms": (optimized[9] + optimized[10]) / 2.0,
            "baseline_p95_ms": baseline[18],
            "optimized_p95_ms": optimized[18],
            "baseline_samples_ms": baseline,
            "optimized_samples_ms": optimized,
            "scope": "subscription setup only; no model or end-to-end turn latency claim"
        })
    );
    publisher.shutdown().await;
    drop(proxy);
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
        event_broadcaster: None,
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
            .jetstream
            .publish(subject, payload.into())
            .await
            .unwrap()
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
