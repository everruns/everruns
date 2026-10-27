// Event delivery abstraction for real-time SSE push
//
// Decision: Enum dispatch (same pattern as StorageBackend) over trait objects.
// Two backends:
//   - InMemory: broadcast::channel per session partition, for dev mode / single instance
//   - Nats: NATS JetStream streams, for multi-instance production
//
// Ephemeral events (deltas) skip PostgreSQL entirely and flow through this
// layer only. Durable events are published here AND persisted to PG.
// SSE streams subscribe here instead of polling PG.

use anyhow::{Context, Result};
use everruns_core::Event;
use everruns_core::events::EventData;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Event delivery backend for real-time SSE push.
///
/// Follows the same enum dispatch pattern as `StorageBackend`.
/// - `InMemory`: dev mode, single process, no external deps
/// - `Nats`: production, multi-instance, NATS JetStream
#[derive(Clone)]
pub enum EventDelivery {
    /// In-process broadcast channels, partitioned by session_id.
    /// Works for single-instance deployments and dev mode.
    InMemory(Arc<InMemoryEventDelivery>),
    /// NATS JetStream for multi-instance production deployments.
    Nats(Arc<NatsEventDelivery>),
}

impl EventDelivery {
    /// Whether this backend supports ephemeral-only delivery (skip PG).
    /// Only NATS provides durable pub/sub with replay — InMemory events are
    /// lost on restart, so we must persist everything to PG as fallback.
    pub fn supports_ephemeral_skip(&self) -> bool {
        matches!(self, Self::Nats(_))
    }

    /// Backend name for health/metrics reporting
    pub fn backend_name(&self) -> &'static str {
        match self {
            Self::InMemory(_) => "in_memory",
            Self::Nats(_) => "nats",
        }
    }

    /// Create from environment. Uses NATS if `NATS_URL` is set, otherwise in-memory.
    pub async fn from_env() -> Self {
        match std::env::var("NATS_URL") {
            Ok(url) => match NatsEventDelivery::connect(&url).await {
                Ok(nats) => {
                    info!(
                        url = %crate::live_updates::nats::redact_server_urls(&url),
                        "Event delivery: NATS JetStream"
                    );
                    Self::Nats(Arc::new(nats))
                }
                Err(e) => {
                    warn!(
                        error = %crate::live_updates::nats::error_chain(&e),
                        "Failed to connect to NATS — falling back to in-memory event delivery. \
                         SSE will only work within this process instance."
                    );
                    Self::InMemory(Arc::new(InMemoryEventDelivery::new()))
                }
            },
            Err(_) => {
                info!(
                    "NATS_URL not set — using in-memory event delivery. \
                     Set NATS_URL for multi-instance SSE support."
                );
                Self::InMemory(Arc::new(InMemoryEventDelivery::new()))
            }
        }
    }

    /// Create an in-memory delivery backend (for dev mode and tests).
    pub fn in_memory() -> Self {
        Self::InMemory(Arc::new(InMemoryEventDelivery::new()))
    }

    /// Publish an event for real-time delivery to SSE subscribers.
    pub async fn publish(&self, event: &Event) -> Result<()> {
        match self {
            Self::InMemory(inner) => inner.publish(event),
            Self::Nats(inner) => inner.publish(event).await,
        }
    }

    /// Subscribe to events for a specific session.
    /// Returns a receiver that yields events as they are published.
    pub async fn subscribe(&self, session_id: Uuid) -> Result<EventSubscription> {
        match self {
            Self::InMemory(inner) => Ok(inner.subscribe(session_id)),
            Self::Nats(inner) => inner.subscribe(session_id).await,
        }
    }
}

// ============================================================
// Subscription handle
// ============================================================

/// A subscription to events for a specific session.
/// Callers receive events via `recv()`.
pub enum EventSubscription {
    InMemory(broadcast::Receiver<Event>),
    Nats(Box<async_nats::jetstream::consumer::pull::Stream>),
}

impl EventSubscription {
    /// Receive the next event. Returns None if the subscription is closed.
    pub async fn recv(&mut self) -> Option<Event> {
        match self {
            Self::InMemory(rx) => loop {
                match rx.recv().await {
                    Ok(event) => return Some(event),
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        debug!(lagged = n, "Event subscription lagged, continuing");
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            },
            Self::Nats(stream) => loop {
                use futures::StreamExt;
                match stream.next().await {
                    Some(Ok(msg)) => {
                        let event: Event = match serde_json::from_slice(&msg.payload) {
                            Ok(e) => e,
                            Err(e) => {
                                // Ack the bad message so it's not redelivered, then continue
                                warn!(error = %e, "Failed to deserialize NATS event, skipping");
                                if let Err(ack_err) = msg.ack().await {
                                    warn!(error = %ack_err, "Failed to ack malformed NATS message");
                                }
                                continue;
                            }
                        };
                        if let Err(e) = msg.ack().await {
                            warn!(error = %e, "Failed to ack NATS message");
                        }
                        return Some(event);
                    }
                    Some(Err(e)) => {
                        warn!(error = %e, "NATS subscription error");
                        return None;
                    }
                    None => return None,
                }
            },
        }
    }
}

// ============================================================
// In-memory backend
// ============================================================

const NUM_PARTITIONS: usize = 64;
const CHANNEL_CAPACITY: usize = 4096;

/// In-memory event delivery using partitioned broadcast channels.
/// Each session maps to a partition by hash. Subscribers filter by session_id.
pub struct InMemoryEventDelivery {
    partitions: Vec<broadcast::Sender<Event>>,
}

impl Default for InMemoryEventDelivery {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryEventDelivery {
    pub fn new() -> Self {
        let partitions = (0..NUM_PARTITIONS)
            .map(|_| broadcast::channel(CHANNEL_CAPACITY).0)
            .collect();
        Self { partitions }
    }

    fn partition_for(&self, session_id: Uuid) -> usize {
        (session_id.as_u128() % NUM_PARTITIONS as u128) as usize
    }

    pub fn publish(&self, event: &Event) -> Result<()> {
        let idx = self.partition_for(event.session_id.uuid());
        // Ignore send errors (no subscribers = event dropped, which is fine)
        let _ = self.partitions[idx].send(event.clone());
        Ok(())
    }

    pub fn subscribe(&self, session_id: Uuid) -> EventSubscription {
        let idx = self.partition_for(session_id);
        EventSubscription::InMemory(self.partitions[idx].subscribe())
    }
}

// ============================================================
// NATS JetStream backend
// ============================================================

/// NATS stream name for session events.
const NATS_STREAM_NAME: &str = "EVENTS";
/// Subject prefix for session events: events.<session_id>
const NATS_SUBJECT_PREFIX: &str = "events";

/// NATS JetStream event delivery.
/// Each session gets a subject: `events.<session_id>`.
/// A single JetStream stream captures all subjects matching `events.>`.
pub struct NatsEventDelivery {
    client: async_nats::Client,
    jetstream: async_nats::jetstream::Context,
}

impl NatsEventDelivery {
    pub async fn connect(url: &str) -> Result<Self> {
        let client = crate::live_updates::nats::connect(url)
            .await
            .context("Failed to connect to NATS")?;

        let jetstream = async_nats::jetstream::new(client.clone());

        let delivery = Self { client, jetstream };
        delivery.ensure_stream().await?;
        info!("NATS JetStream stream '{}' ready", NATS_STREAM_NAME);
        Ok(delivery)
    }

    // The replay buffer is memory-backed and disappears on a broker restart.
    // Existing API clients reconnect independently, so initialization cannot be
    // confined to API startup. Never rewrite an existing stream's policy here.
    async fn ensure_stream(&self) -> Result<async_nats::jetstream::stream::Stream> {
        let stream_config = async_nats::jetstream::stream::Config {
            name: NATS_STREAM_NAME.to_string(),
            subjects: vec![format!("{NATS_SUBJECT_PREFIX}.>")],
            max_age: std::time::Duration::from_secs(3600), // 1 hour retention
            storage: async_nats::jetstream::stream::StorageType::Memory,
            num_replicas: 1,
            discard: async_nats::jetstream::stream::DiscardPolicy::Old,
            max_bytes: 1_073_741_824, // 1 GB max
            ..Default::default()
        };

        self.jetstream
            .get_or_create_stream(stream_config)
            .await
            .context("Failed to ensure NATS JetStream event stream")
    }

    pub async fn publish(&self, event: &Event) -> Result<()> {
        let subject = format!("{}.{}", NATS_SUBJECT_PREFIX, event.session_id.uuid());
        let payload = live_payload(event, self.client.max_payload())?;

        let ack = self
            .jetstream
            .publish(subject.clone(), payload.into())
            .await
            .context("Failed to publish event to NATS")?;
        match ack.await {
            Ok(_) => Ok(()),
            Err(error)
                if error.kind()
                    == async_nats::jetstream::context::PublishErrorKind::StreamNotFound =>
            {
                // This rejection proves the event was not stored. Retry once
                // after restoring the buffer; timeouts and other ambiguous
                // failures must not be replayed, which could duplicate events.
                self.ensure_stream().await?;
                self.jetstream
                    .publish(
                        subject,
                        live_payload(event, self.client.max_payload())?.into(),
                    )
                    .await
                    .context("Failed to republish event to NATS")?
                    .await
                    .context("Failed to confirm recovered NATS publish")?;
                info!("NATS event delivery recovered a missing stream");
                Ok(())
            }
            Err(error) => Err(error).context("Failed to confirm NATS publish"),
        }
    }

    pub async fn subscribe(&self, session_id: Uuid) -> Result<EventSubscription> {
        let subject = format!("{NATS_SUBJECT_PREFIX}.{session_id}");
        // Create an ephemeral pull consumer for this SSE connection.
        // DeliverPolicy::New means we only get events from now on.
        // For reconnection replay, the SSE handler queries PG separately.
        let config = async_nats::jetstream::consumer::pull::Config {
            filter_subject: subject,
            deliver_policy: async_nats::jetstream::consumer::DeliverPolicy::New,
            ack_policy: async_nats::jetstream::consumer::AckPolicy::Explicit,
            ..Default::default()
        };
        // The stream was initialized at connect. Avoid a discovery round trip
        // on every SSE connection; recover only a definite missing-stream
        // rejection. A timeout may have created a consumer, so never retry it.
        let consumer = match self
            .jetstream
            .create_consumer_on_stream(config.clone(), NATS_STREAM_NAME)
            .await
        {
            Ok(consumer) => consumer,
            Err(error)
                if matches!(
                    error.kind(),
                    async_nats::jetstream::stream::ConsumerErrorKind::JetStream(ref error)
                        if error.error_code()
                            == async_nats::jetstream::ErrorCode::STREAM_NOT_FOUND
                ) =>
            {
                self.ensure_stream().await?;
                self.jetstream
                    .create_consumer_on_stream(config, NATS_STREAM_NAME)
                    .await
                    .context("Failed to create recovered NATS consumer")?
            }
            Err(error) => return Err(error).context("Failed to create NATS consumer"),
        };

        let messages = consumer
            .messages()
            .await
            .context("Failed to get NATS message stream")?;

        Ok(EventSubscription::Nats(Box::new(messages)))
    }

    /// Shut down the NATS connection gracefully.
    pub async fn shutdown(&self) {
        let _ = self.client.drain().await;
    }
}

/// `metadata` key set on a live copy whose bulky fields were dropped to fit
/// the server's `max_payload`. The stored event keeps them; a client that
/// needs them reads the event from the API.
pub const LIVE_PAYLOAD_TRIMMED_KEY: &str = "live_payload_trimmed";

/// Serialize `event` for live delivery within `max_payload` bytes.
///
/// Decision (Sentry EVERRUNS-32): `llm.generation` carries the whole prompt
/// twice (`messages` and the captured `request_body`), so on long sessions it
/// outgrows NATS `max_payload` (1 MiB by default) and every publish failed,
/// leaving live subscribers without the generation's usage and output. Its
/// copy on the wire drops the captured request body first, then the prompt
/// messages and tool list, keeping output and metadata (usage, cost, model)
/// that live views read. The PostgreSQL row is untouched. Other event types
/// have no such fields, so an oversize one is still an error, with sizes.
fn live_payload(event: &Event, max_payload: usize) -> Result<Vec<u8>> {
    let payload = serde_json::to_vec(event).context("Failed to serialize event")?;
    if payload.len() <= max_payload {
        return Ok(payload);
    }
    let full_size = payload.len();

    if let EventData::LlmGeneration(data) = &event.data {
        let mut trimmed = event.clone();
        let mut slim = data.clone();
        slim.request_body = None;
        trimmed.data = EventData::LlmGeneration(slim.clone());
        mark_trimmed(&mut trimmed);
        let payload = serde_json::to_vec(&trimmed).context("Failed to serialize event")?;
        if payload.len() <= max_payload {
            debug!(
                full_size,
                size = payload.len(),
                max_payload,
                "Dropped request_body from live llm.generation"
            );
            return Ok(payload);
        }
        slim.messages.clear();
        slim.tools.clear();
        trimmed.data = EventData::LlmGeneration(slim);
        let payload = serde_json::to_vec(&trimmed).context("Failed to serialize event")?;
        if payload.len() <= max_payload {
            debug!(
                full_size,
                size = payload.len(),
                max_payload,
                "Dropped prompt from live llm.generation"
            );
            return Ok(payload);
        }
    }

    anyhow::bail!(
        "event payload of {full_size} bytes exceeds NATS max_payload of {max_payload} bytes"
    )
}

fn mark_trimmed(event: &mut Event) {
    match &mut event.metadata {
        Some(serde_json::Value::Object(map)) => {
            map.insert(LIVE_PAYLOAD_TRIMMED_KEY.into(), true.into());
        }
        None => {
            event.metadata = Some(serde_json::json!({ LIVE_PAYLOAD_TRIMMED_KEY: true }));
        }
        // Non-object metadata is caller-owned; leave it as is.
        Some(_) => {}
    }
}

#[cfg(test)]
mod subscription_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::typed_id::{EventId, MessageId, SessionId, TurnId};
    use everruns_core::events::{EventData, OutputMessageDeltaData};

    pub(super) fn make_test_event(session_id: Uuid) -> Event {
        Event {
            id: EventId::new(),
            event_type: "output.message.delta".to_string(),
            ts: chrono::Utc::now(),
            session_id: SessionId::from_uuid(session_id),
            context: Default::default(),
            data: EventData::OutputMessageDelta(OutputMessageDeltaData {
                turn_id: TurnId::new(),
                message_id: MessageId::new(),
                delta: "hello".to_string(),
                accumulated: "hello".to_string(),
                phase: None,
            }),
            metadata: None,
            tags: None,
            sequence: None,
        }
    }

    #[tokio::test]
    async fn test_in_memory_publish_subscribe() {
        let delivery = InMemoryEventDelivery::new();
        let session_id = Uuid::new_v4();

        let mut sub = delivery.subscribe(session_id);
        let event = make_test_event(session_id);

        delivery.publish(&event).unwrap();

        match &mut sub {
            EventSubscription::InMemory(rx) => {
                let received = rx.recv().await.unwrap();
                assert_eq!(received.id, event.id);
                assert_eq!(received.event_type, "output.message.delta");
            }
            _ => panic!("Expected InMemory subscription"),
        }
    }

    /// Partitions are shared by hash, not one-per-session (see `partition_for`):
    /// a subscriber for session A can be handed a partition that also carries
    /// session B's events. `InMemoryEventDelivery` makes no isolation guarantee
    /// by itself — the caller (the SSE handler, see
    /// `api::events::event_passes_stream_filter`) is responsible for dropping
    /// events that belong to a different session. This test proves only the
    /// guarantee this layer does own: a session's own event is always
    /// delivered to its subscriber, even when another session's event shares
    /// the same partition and arrives first.
    #[tokio::test]
    async fn test_in_memory_delivery_delivers_own_session_despite_partition_sharing() {
        let delivery = InMemoryEventDelivery::new();
        let session_a = Uuid::new_v4();
        let session_b = Uuid::new_v4();

        let mut sub_a = delivery.subscribe(session_a);

        // Publish to session B first — may or may not land in A's partition.
        let event_b = make_test_event(session_b);
        delivery.publish(&event_b).unwrap();

        // Publish to session A — this MUST reach subscriber A regardless.
        let event_a = make_test_event(session_a);
        delivery.publish(&event_a).unwrap();

        match &mut sub_a {
            EventSubscription::InMemory(rx) => {
                // Drain until we find event_a or timeout. Any interleaved
                // event_b (partition collision) is expected and skipped here —
                // filtering it out is the SSE handler's job, not this layer's.
                let timeout = tokio::time::timeout(std::time::Duration::from_millis(100), async {
                    loop {
                        match rx.recv().await {
                            Ok(e) if e.session_id.uuid() == session_a => return e,
                            Ok(_) => continue,
                            Err(_) => panic!("Channel closed"),
                        }
                    }
                })
                .await;
                assert!(timeout.is_ok(), "Should receive event for session A");
            }
            _ => panic!("Expected InMemory subscription"),
        }
    }

    fn large_generation(prompt_bytes: usize) -> Event {
        let prompt = "x".repeat(prompt_bytes);
        let mut data = everruns_core::LlmGenerationData::success(
            vec![everruns_core::RuntimeMessage::user(prompt.clone())],
            vec![],
            Some("done".into()),
            vec![],
            "gpt-6".into(),
            Some("openai".into()),
            Some(everruns_core::TokenUsage {
                input_tokens: 4242,
                output_tokens: 7,
                ..Default::default()
            }),
            None,
            None,
        );
        data.request_body = Some(serde_json::json!({ "input": prompt }));
        Event::new(SessionId::new(), everruns_core::EventContext::empty(), data)
    }

    fn generation_of(payload: &[u8]) -> (Event, everruns_core::LlmGenerationData) {
        let event: Event = serde_json::from_slice(payload).expect("live payload deserializes");
        let EventData::LlmGeneration(data) = event.data.clone() else {
            panic!("expected llm.generation");
        };
        (event, data)
    }

    #[test]
    fn live_payload_passes_small_events_unchanged() {
        let event = large_generation(100);
        let payload = live_payload(&event, 1024 * 1024).unwrap();
        assert_eq!(payload, serde_json::to_vec(&event).unwrap());
    }

    /// Regression test for Sentry EVERRUNS-32: an `llm.generation` whose
    /// prompt (carried in both `messages` and `request_body`) exceeds NATS
    /// `max_payload` failed every publish instead of reaching live
    /// subscribers.
    #[test]
    fn live_payload_drops_request_body_from_oversize_generation() {
        let event = large_generation(600 * 1024);
        let max = 1024 * 1024;
        assert!(serde_json::to_vec(&event).unwrap().len() > max);

        let payload = live_payload(&event, max).unwrap();
        assert!(payload.len() <= max);
        let (live, data) = generation_of(&payload);
        assert_eq!(live.id, event.id);
        assert!(data.request_body.is_none());
        assert_eq!(data.messages.len(), 1, "prompt kept when it fits");
        assert_eq!(data.metadata.usage.unwrap().input_tokens, 4242);
        assert_eq!(live.metadata.unwrap()[LIVE_PAYLOAD_TRIMMED_KEY], true);
    }

    #[test]
    fn live_payload_drops_prompt_when_request_body_alone_is_not_enough() {
        let event = large_generation(2 * 1024 * 1024);
        let max = 1024 * 1024;

        let payload = live_payload(&event, max).unwrap();
        assert!(payload.len() <= max);
        let (live, data) = generation_of(&payload);
        assert!(data.request_body.is_none());
        assert!(data.messages.is_empty());
        assert_eq!(data.output.text.as_deref(), Some("done"));
        assert_eq!(data.metadata.usage.unwrap().output_tokens, 7);
        assert_eq!(live.metadata.unwrap()[LIVE_PAYLOAD_TRIMMED_KEY], true);
    }

    #[test]
    fn live_payload_reports_sizes_for_other_oversize_events() {
        let mut event = make_test_event(Uuid::new_v4());
        let EventData::OutputMessageDelta(delta) = &mut event.data else {
            unreachable!()
        };
        delta.accumulated = "y".repeat(4096);

        let error = live_payload(&event, 1024).unwrap_err().to_string();
        assert!(
            error.contains("exceeds NATS max_payload of 1024 bytes"),
            "{error}"
        );
    }

    // Each test owns a loopback-only broker and store. Never point restart tests
    // at an external broker: losing this memory stream is the failure under test.
    enum TestBroker {
        Native(std::process::Child),
        Container(String),
    }

    pub(super) struct TestNatsServer {
        process: TestBroker,
        store: tempfile::TempDir,
        pub(super) port: u16,
    }

    impl TestNatsServer {
        pub(super) async fn start() -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            drop(listener);
            let store = tempfile::tempdir().unwrap();
            let process = Self::spawn(port, store.path());
            let mut broker = Self {
                process,
                store,
                port,
            };
            broker.ready().await;
            broker
        }

        fn spawn(port: u16, store: &std::path::Path) -> TestBroker {
            let process = std::process::Command::new("nats-server")
                .args(["-js", "-a", "127.0.0.1", "-p", &port.to_string(), "-sd"])
                .arg(store)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
            match process {
                Ok(child) => TestBroker::Native(child),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    // Hosted CI already supplies Docker. Bind only loopback,
                    // mount no host paths and remove only this random-owned
                    // container. No workflow secret or production broker is used.
                    let name = format!("everruns-nats-test-{}", Uuid::new_v4());
                    let output = std::process::Command::new("docker")
                        .args([
                            "run",
                            "-d",
                            "--name",
                            &name,
                            "-p",
                            &format!("127.0.0.1:{port}:4222"),
                            "mirror.gcr.io/library/nats:2-alpine",
                            "--jetstream",
                            "--store_dir",
                            "/data",
                        ])
                        .output()
                        .expect("tests require native nats-server or Docker");
                    assert!(
                        output.status.success(),
                        "isolated NATS container failed to start"
                    );
                    TestBroker::Container(name)
                }
                Err(error) => panic!("could not start isolated NATS: {error}"),
            }
        }

        pub(super) fn url(&self) -> String {
            format!("nats://127.0.0.1:{}", self.port)
        }

        async fn ready(&mut self) {
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                loop {
                    if let TestBroker::Native(process) = &mut self.process {
                        assert!(process.try_wait().unwrap().is_none(), "broker exited");
                    }
                    // A forwarded Docker port can accept TCP before the broker
                    // speaks NATS. Require the application handshake and PONG.
                    let ready =
                        tokio::time::timeout(std::time::Duration::from_millis(250), async {
                            match async_nats::connect(self.url()).await {
                                Ok(client) => client.flush().await.is_ok(),
                                Err(_) => false,
                            }
                        })
                        .await
                        .unwrap_or(false);
                    if ready {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("broker ready");
        }

        pub(super) async fn restart(&mut self, delivery: &NatsEventDelivery) {
            match &mut self.process {
                TestBroker::Native(process) => {
                    process.kill().unwrap();
                    process.wait().unwrap();
                    self.process = Self::spawn(self.port, self.store.path());
                }
                TestBroker::Container(name) => {
                    let output = std::process::Command::new("docker")
                        .args(["restart", "--time", "0", name])
                        .output()
                        .unwrap();
                    assert!(output.status.success(), "owned broker restart failed");
                }
            }
            self.ready().await;
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                loop {
                    if delivery.client.connection_state()
                        == async_nats::connection::State::Connected
                        && delivery.client.flush().await.is_ok()
                    {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("original client reconnects");
            let error = delivery
                .jetstream
                .get_stream(NATS_STREAM_NAME)
                .await
                .expect_err("memory stream is absent after broker restart");
            assert!(error.to_string().contains("stream not found"), "{error}");
        }
    }

    impl Drop for TestNatsServer {
        fn drop(&mut self) {
            match &mut self.process {
                TestBroker::Native(process) => {
                    let _ = process.kill();
                    let _ = process.wait();
                }
                TestBroker::Container(name) => {
                    let _ = std::process::Command::new("docker")
                        .args(["rm", "--force", "--volumes", name])
                        .output();
                }
            }
        }
    }

    #[tokio::test]
    async fn nats_stream_restart_recovers_delta_publish_without_restarting_api() {
        let mut broker = TestNatsServer::start().await;
        let publisher = NatsEventDelivery::connect(&broker.url()).await.unwrap();
        broker.restart(&publisher).await;

        let event = make_test_event(Uuid::new_v4());
        publisher
            .publish(&event)
            .await
            .expect("first delta recovers missing stream");
        let stream = publisher
            .jetstream
            .get_stream(NATS_STREAM_NAME)
            .await
            .unwrap();
        assert_eq!(stream.cached_info().state.messages, 1, "no duplicate retry");
        let mut subscriber = publisher.subscribe(event.session_id.uuid()).await.unwrap();
        let next = make_test_event(event.session_id.uuid());
        publisher.publish(&next).await.unwrap();
        let received = tokio::time::timeout(std::time::Duration::from_secs(5), subscriber.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received.id, next.id);
        assert_eq!(received.event_type, "output.message.delta");
        assert_eq!(
            serde_json::to_value(received.data).unwrap(),
            serde_json::to_value(next.data).unwrap()
        );
        publisher.shutdown().await;
    }

    #[tokio::test]
    async fn nats_stream_restart_recovers_subscription_before_any_publish() {
        let mut broker = TestNatsServer::start().await;
        let publisher = NatsEventDelivery::connect(&broker.url()).await.unwrap();
        broker.restart(&publisher).await;
        let event = make_test_event(Uuid::new_v4());
        let mut subscriber = publisher
            .subscribe(event.session_id.uuid())
            .await
            .expect("SSE reconnect restores stream even before a new event");
        publisher.publish(&event).await.unwrap();
        let received = tokio::time::timeout(std::time::Duration::from_secs(5), subscriber.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received.id, event.id);
        assert!(
            received.sequence.is_none(),
            "ephemeral event stays ephemeral"
        );
        publisher.shutdown().await;
    }

    #[tokio::test]
    async fn nats_stream_recovery_does_not_overwrite_existing_subject_policy() {
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
        config.subjects = vec!["other.>".to_owned()];
        publisher.jetstream.update_stream(&config).await.unwrap();
        let error = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            publisher.publish(&make_test_event(Uuid::new_v4())),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert!(format!("{error:#}").contains("no stream found for given subject"));
        let stream = publisher
            .jetstream
            .get_stream(NATS_STREAM_NAME)
            .await
            .unwrap();
        assert_eq!(stream.cached_info().config.subjects, vec!["other.>"]);
        assert_eq!(stream.cached_info().state.messages, 0);
        publisher.shutdown().await;
    }

    #[tokio::test]
    async fn nats_stream_restart_restores_real_event_service_delta_and_durable_delivery() {
        use crate::services::EventService;
        use crate::storage::StorageBackend;
        use everruns_core::{
            EventContext, EventRequest, OutputMessageCompletedData, RuntimeMessage,
        };

        let mut broker = TestNatsServer::start().await;
        let publisher = Arc::new(NatsEventDelivery::connect(&broker.url()).await.unwrap());
        let delivery = EventDelivery::Nats(publisher.clone());
        let db = Arc::new(StorageBackend::test_database());
        let session = db.create_test_session().await;
        let service = EventService::new(db, delivery.clone());
        broker.restart(&publisher).await;

        let data = make_test_event(session.uuid()).data;
        let recovered = service
            .emit(EventRequest::new(
                session,
                EventContext::empty(),
                data.clone(),
            ))
            .await
            .unwrap();
        assert!(recovered.sequence.is_none());
        let stream = publisher
            .jetstream
            .get_stream(NATS_STREAM_NAME)
            .await
            .expect("emit_ephemeral really published, not merely returned Ok");
        assert_eq!(stream.cached_info().state.messages, 1);

        let mut subscriber = delivery.subscribe(session.uuid()).await.unwrap();
        let delta = service
            .emit(EventRequest::new(session, EventContext::empty(), data))
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
        for expected in [&delta, &completed] {
            let received =
                tokio::time::timeout(std::time::Duration::from_secs(5), subscriber.recv())
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(received.id, expected.id);
            assert_eq!(received.event_type, expected.event_type);
        }
        let durable = service
            .list(session.uuid(), None, None, &[], &[], None, None)
            .await
            .unwrap();
        assert_eq!(
            durable.len(),
            1,
            "deltas must not be persisted to PostgreSQL"
        );
        assert_eq!(durable[0].id, completed.id);
        assert!(durable[0].sequence.is_some());
        publisher.shutdown().await;
    }

    #[tokio::test]
    async fn nats_stream_recovery_does_not_retry_an_uncertain_ack_timeout() {
        let broker = TestNatsServer::start().await;
        let mut publisher = NatsEventDelivery::connect(&broker.url()).await.unwrap();
        let mut config = publisher
            .jetstream
            .get_stream(NATS_STREAM_NAME)
            .await
            .unwrap()
            .cached_info()
            .config
            .clone();
        // The broker stores the message but withholds its ack. Retrying this
        // timeout would duplicate an event that was already accepted.
        config.no_ack = true;
        publisher.jetstream.update_stream(&config).await.unwrap();
        publisher
            .jetstream
            .set_timeout(std::time::Duration::from_millis(250));
        let error = publisher
            .publish(&make_test_event(Uuid::new_v4()))
            .await
            .unwrap_err();
        let typed = error
            .downcast_ref::<async_nats::jetstream::context::PublishError>()
            .unwrap();
        assert_eq!(
            typed.kind(),
            async_nats::jetstream::context::PublishErrorKind::TimedOut
        );
        let stream = publisher
            .jetstream
            .get_stream(NATS_STREAM_NAME)
            .await
            .unwrap();
        assert_eq!(
            stream.cached_info().state.messages,
            1,
            "no duplicate after uncertain ack"
        );
        assert!(
            stream.cached_info().config.no_ack,
            "existing policy unchanged"
        );
        publisher.shutdown().await;
    }

    #[tokio::test]
    async fn nats_stream_restart_allows_concurrent_publishers_to_recover() {
        let mut broker = TestNatsServer::start().await;
        let publisher = Arc::new(NatsEventDelivery::connect(&broker.url()).await.unwrap());
        broker.restart(&publisher).await;
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..16 {
            let publisher = publisher.clone();
            tasks.spawn(async move {
                publisher
                    .publish(&make_test_event(Uuid::new_v4()))
                    .await
                    .unwrap();
            });
        }
        while let Some(result) = tasks.join_next().await {
            result.unwrap();
        }
        let stream = publisher
            .jetstream
            .get_stream(NATS_STREAM_NAME)
            .await
            .unwrap();
        assert_eq!(
            stream.cached_info().state.messages,
            16,
            "every delta exactly once"
        );
        publisher.shutdown().await;
    }

    /// End-to-end check of EVERRUNS-32 against the real JetStream server
    /// named by `EVERRUNS_TEST_NATS_URL`; the weekly `Integration Live Sweep`
    /// starts one (feature `live-tests`).
    #[cfg(feature = "live-tests")]
    #[tokio::test]
    async fn oversize_generation_reaches_nats_subscribers() {
        let url = std::env::var("EVERRUNS_TEST_NATS_URL").expect("EVERRUNS_TEST_NATS_URL");
        let nats = NatsEventDelivery::connect(&url).await.unwrap();
        let event = large_generation(nats.client.max_payload());
        let mut sub = nats.subscribe(event.session_id.uuid()).await.unwrap();

        // The untrimmed event is what failed in production.
        let raw = nats
            .jetstream
            .publish(
                format!("{NATS_SUBJECT_PREFIX}.{}", event.session_id.uuid()),
                serde_json::to_vec(&event).unwrap().into(),
            )
            .await
            .unwrap_err();
        assert_eq!(
            raw.kind(),
            async_nats::jetstream::context::PublishErrorKind::MaxPayloadExceeded
        );

        nats.publish(&event).await.unwrap();

        let received = tokio::time::timeout(std::time::Duration::from_secs(5), sub.recv())
            .await
            .expect("delivered")
            .expect("subscription open");
        assert_eq!(received.id, event.id);
        assert_eq!(received.metadata.unwrap()[LIVE_PAYLOAD_TRIMMED_KEY], true);
        nats.shutdown().await;
    }
}
