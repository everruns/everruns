// Event delivery abstraction for real-time SSE push
//
// Decision: Enum dispatch (same pattern as StorageBackend) over trait objects.
// Two backends:
//   - InMemory: broadcast::channel per session partition, for dev mode / single instance
//   - Nats: NATS publish/subscribe, for multi-instance production
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

mod polling;
pub use polling::PollingEventDelivery;

/// Event delivery backend for real-time SSE push.
///
/// Follows the same enum dispatch pattern as `StorageBackend`.
/// - `InMemory`: dev mode, single process, no external deps
/// - `Nats`: production, multi-instance, NATS
/// - `Polling`: several servers without a reachable NATS (degraded)
#[derive(Clone)]
pub enum EventDelivery {
    /// In-process broadcast channels, partitioned by session_id.
    /// Works for single-instance deployments and dev mode only: a client sees
    /// only what its own process published.
    InMemory(Arc<InMemoryEventDelivery>),
    /// NATS for multi-instance production deployments.
    Nats(Arc<NatsEventDelivery>),
    /// Subscribers poll the event log in PostgreSQL (see `polling`).
    Polling(Arc<PollingEventDelivery>),
}

impl EventDelivery {
    /// Whether this backend supports ephemeral-only delivery (skip PG).
    /// Only NATS reaches every server's subscribers; in-memory delivery
    /// persists everything to PG as fallback.
    pub fn supports_ephemeral_skip(&self) -> bool {
        matches!(self, Self::Nats(_))
    }

    /// Backend name for health/metrics reporting
    pub fn backend_name(&self) -> &'static str {
        match self {
            Self::InMemory(_) => "in_memory",
            Self::Nats(_) => "nats",
            Self::Polling(_) => "polling",
        }
    }

    /// Whether live delivery runs below what this deployment asked for: NATS
    /// is configured but unreachable, or several servers run without it.
    pub fn is_degraded(&self) -> bool {
        matches!(self, Self::Polling(_))
    }

    /// Create from environment.
    ///
    /// - `NATS_URL` set and reachable: NATS.
    /// - `NATS_URL` set but unreachable: PostgreSQL polling. Decision: degrade
    ///   rather than refuse to start, so a NATS outage does not keep servers
    ///   down; in-memory delivery here would silently split live updates
    ///   between servers.
    /// - No `NATS_URL` and `EXPECTED_INSTANCES` above 1: PostgreSQL polling,
    ///   for the same reason.
    /// - Otherwise (one server): in-memory.
    pub async fn from_env(db: Arc<crate::storage::StorageBackend>) -> Self {
        let delivery = Self::select_from_env(db).await;
        // Heartbeats carry it, so the system doctor sees every server's choice.
        let _ = ACTIVE_BACKEND.set(delivery.backend_name());
        delivery
    }

    async fn select_from_env(db: Arc<crate::storage::StorageBackend>) -> Self {
        let polling = |db| Self::Polling(Arc::new(PollingEventDelivery::new(db)));
        match std::env::var("NATS_URL") {
            Ok(url) => match NatsEventDelivery::connect(&url).await {
                Ok(nats) => {
                    info!(
                        url = %crate::live_updates::nats::redact_server_urls(&url),
                        "Event delivery: NATS"
                    );
                    Self::Nats(Arc::new(nats))
                }
                Err(e) => {
                    warn!(
                        error = %crate::live_updates::nats::error_chain(&e),
                        "Failed to connect to NATS, so live session events are polled from \
                         PostgreSQL (degraded) until this server restarts."
                    );
                    polling(db)
                }
            },
            Err(_) if expected_instances() > 1 => {
                warn!(
                    "NATS_URL not set and EXPECTED_INSTANCES > 1, so live session events are \
                     polled from PostgreSQL (degraded). More than one server needs NATS."
                );
                polling(db)
            }
            Err(_) => {
                info!(
                    "NATS_URL not set: in-memory event delivery, which serves one server only. \
                     More than one server needs NATS."
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
            // Subscribers read the stored event from PostgreSQL.
            Self::Polling(_) => Ok(()),
        }
    }

    /// Subscribe to events for a specific session.
    /// Returns a receiver that yields events as they are published.
    pub async fn subscribe(&self, session_id: Uuid) -> Result<EventSubscription> {
        match self {
            Self::InMemory(inner) => Ok(inner.subscribe(session_id)),
            Self::Nats(inner) => inner.subscribe(session_id).await,
            Self::Polling(inner) => inner.subscribe(session_id).await,
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
    Nats(Box<async_nats::Subscriber>),
    Polling(Box<polling::PollingSubscription>),
}

impl EventSubscription {
    /// Receive the next event. Returns None if the subscription is closed.
    pub async fn recv(&mut self) -> Option<Event> {
        match self {
            Self::Polling(subscription) => subscription.recv().await,
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
            Self::Nats(subscriber) => loop {
                use futures::StreamExt;
                let message = subscriber.next().await?;
                match serde_json::from_slice(&message.payload) {
                    Ok(event) => return Some(event),
                    Err(e) => warn!(error = %e, "Failed to deserialize NATS event, skipping"),
                }
            },
        }
    }
}

static ACTIVE_BACKEND: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();

/// The backend this server chose at startup (`EventDelivery::from_env`), if
/// it did.
pub fn active_backend() -> Option<&'static str> {
    ACTIVE_BACKEND.get().copied()
}

/// `EXPECTED_INSTANCES`: how many servers the operator runs (default 1).
pub(crate) fn expected_instances() -> usize {
    std::env::var("EXPECTED_INSTANCES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1)
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
// NATS backend
// ============================================================

/// Subject prefix for session events: events.<session_id>
const NATS_SUBJECT_PREFIX: &str = "events";
/// The JetStream stream earlier versions kept every event in for an hour.
const RETIRED_STREAM_NAME: &str = "EVENTS";
/// How often a server removes that stream again, in case a server of an
/// earlier version recreated it during a rolling deploy.
const RETIRE_STREAM_EVERY: std::time::Duration = std::time::Duration::from_secs(600);

/// NATS event delivery: plain publish and subscribe, one subject per session
/// (`events.<session_id>`).
///
/// Decision (fit 8k agents into today's prod): no JetStream. Subscribers
/// always started at new messages and reconnecting streams replay from
/// PostgreSQL, so the memory stream that kept every event, deltas included,
/// for an hour was never read. At 8k agents it would have needed about
/// 100 MB/s of NATS memory, and every publish and delivery waited for an ack.
/// A server drops that stream when it starts and every
/// [`RETIRE_STREAM_EVERY`] after, because an earlier server recreates it on
/// its next publish while a rolling deploy runs. The subject is unchanged, so
/// the two versions deliver to each other's subscribers during the deploy.
///
/// Plain NATS drops messages for a subscriber that falls too far behind. A
/// durable event that arrives past the next sequence makes the SSE stream
/// read the gap from PostgreSQL (`api::events::PushCursor`).
pub struct NatsEventDelivery {
    client: async_nats::Client,
}

impl NatsEventDelivery {
    pub async fn connect(url: &str) -> Result<Self> {
        let client = crate::live_updates::nats::connect(url)
            .await
            .context("Failed to connect to NATS")?;
        tokio::spawn(retire_replay_stream(client.clone()));
        Ok(Self { client })
    }

    pub async fn publish(&self, event: &Event) -> Result<()> {
        let subject = format!("{}.{}", NATS_SUBJECT_PREFIX, event.session_id.uuid());
        let payload = live_payload(event, self.client.max_payload())?;
        self.client
            .publish(subject, payload.into())
            .await
            .context("Failed to publish event to NATS")
    }

    pub async fn subscribe(&self, session_id: Uuid) -> Result<EventSubscription> {
        let subject = format!("{NATS_SUBJECT_PREFIX}.{session_id}");
        let subscriber = self
            .client
            .subscribe(subject)
            .await
            .context("Failed to subscribe to NATS")?;
        // The broker handles a connection's messages in order, so once this
        // round trip returns it has the subscription: an event published
        // after `subscribe` returns reaches it.
        self.client
            .flush()
            .await
            .context("Failed to confirm NATS subscription")?;
        Ok(EventSubscription::Nats(Box::new(subscriber)))
    }

    /// Shut down the NATS connection gracefully.
    pub async fn shutdown(&self) {
        let _ = self.client.drain().await;
    }
}

/// Remove the JetStream stream earlier versions used, now and periodically.
/// A broker without JetStream, or without the stream, has nothing to remove.
async fn retire_replay_stream(client: async_nats::Client) {
    let jetstream = async_nats::jetstream::new(client);
    loop {
        match jetstream.delete_stream(RETIRED_STREAM_NAME).await {
            Ok(_) => info!("Removed the retired NATS JetStream event stream"),
            Err(error) => debug!(error = %error, "No retired NATS event stream to remove"),
        }
        tokio::time::sleep(RETIRE_STREAM_EVERY).await;
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
    // at an external broker.
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
            let store = tempfile::tempdir().unwrap();
            // The probed port is free only until it is released, so a parallel
            // test's socket can take it before the broker binds. That failed
            // the 8c05bead main batch; retry such a lost race on a fresh port.
            // (A Docker-assigned port would avoid the race, but it changes on
            // `docker restart`, which `restart` relies on keeping.)
            let mut attempt = 0;
            let (process, port) = loop {
                let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
                let port = listener.local_addr().unwrap().port();
                drop(listener);
                match Self::spawn(port, store.path()) {
                    Ok(process) => break (process, port),
                    Err(error) if attempt < 4 && is_port_conflict(&error) => attempt += 1,
                    Err(error) => panic!("isolated NATS container failed to start: {error}"),
                }
            };
            let mut broker = Self {
                process,
                store,
                port,
            };
            broker.ready().await;
            broker
        }

        /// Starts a broker on `port`. A Docker start that fails returns its
        /// stderr so the caller can retry a port conflict.
        fn spawn(port: u16, store: &std::path::Path) -> Result<TestBroker, String> {
            let process = std::process::Command::new("nats-server")
                .args(["-js", "-a", "127.0.0.1", "-p", &port.to_string(), "-sd"])
                .arg(store)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
            match process {
                Ok(child) => Ok(TestBroker::Native(child)),
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
                    if !output.status.success() {
                        // A container that lost its port race may still exist.
                        let _ = std::process::Command::new("docker")
                            .args(["rm", "--force", "--volumes", &name])
                            .output();
                        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
                    }
                    Ok(TestBroker::Container(name))
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
                    self.process = Self::spawn(self.port, self.store.path())
                        .expect("native brokers do not fail to spawn");
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
        }
    }

    /// Docker's wording when the host port is already bound or allocated.
    fn is_port_conflict(stderr: &str) -> bool {
        stderr.contains("port is already allocated") || stderr.contains("address already in use")
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

    #[test]
    fn a_taken_broker_port_is_reported_as_a_retryable_conflict() {
        let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = taken.local_addr().unwrap().port();
        let store = tempfile::tempdir().unwrap();
        match TestNatsServer::spawn(port, store.path()) {
            // Native nats-server exits on its own after failing to bind.
            Ok(TestBroker::Native(mut child)) => {
                let _ = child.kill();
                let _ = child.wait();
            }
            Ok(TestBroker::Container(name)) => {
                let _ = std::process::Command::new("docker")
                    .args(["rm", "--force", "--volumes", &name])
                    .output();
                panic!("Docker started a broker on a port that is already bound");
            }
            Err(error) => assert!(is_port_conflict(&error), "{error}"),
        }
    }

    #[tokio::test]
    async fn nats_delivers_to_the_sessions_subscriber_across_a_broker_restart() {
        let mut broker = TestNatsServer::start().await;
        let publisher = NatsEventDelivery::connect(&broker.url()).await.unwrap();
        let session = Uuid::new_v4();
        let mut subscriber = publisher.subscribe(session).await.unwrap();
        let before = make_test_event(session);
        publisher.publish(&before).await.unwrap();
        let received = tokio::time::timeout(std::time::Duration::from_secs(5), subscriber.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received.id, before.id);

        broker.restart(&publisher).await;
        // The client resubscribes on reconnect; a new subscription works too.
        let mut fresh = publisher.subscribe(session).await.unwrap();
        let after = make_test_event(session);
        publisher.publish(&after).await.unwrap();
        for subscription in [&mut subscriber, &mut fresh] {
            let received =
                tokio::time::timeout(std::time::Duration::from_secs(5), subscription.recv())
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(received.id, after.id);
        }
        publisher.shutdown().await;
    }

    // `subscribe` returns only once the broker has the subscription, so an
    // event another server publishes right after it is not lost.
    #[tokio::test]
    async fn an_event_published_from_another_server_right_after_subscribe_arrives() {
        let broker = TestNatsServer::start().await;
        let subscriber_side = NatsEventDelivery::connect(&broker.url()).await.unwrap();
        let publisher_side = NatsEventDelivery::connect(&broker.url()).await.unwrap();
        for _ in 0..20 {
            let session = Uuid::new_v4();
            let mut subscription = subscriber_side.subscribe(session).await.unwrap();
            let event = make_test_event(session);
            publisher_side.publish(&event).await.unwrap();
            let received =
                tokio::time::timeout(std::time::Duration::from_secs(5), subscription.recv())
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(received.id, event.id);
        }
        subscriber_side.shutdown().await;
        publisher_side.shutdown().await;
    }

    // The memory stream earlier versions kept every event in is removed, so
    // live events no longer accumulate in the broker.
    #[tokio::test]
    async fn connecting_retires_the_jetstream_replay_buffer() {
        let broker = TestNatsServer::start().await;
        let legacy = async_nats::connect(broker.url()).await.unwrap();
        let jetstream = async_nats::jetstream::new(legacy.clone());
        jetstream
            .create_stream(async_nats::jetstream::stream::Config {
                name: RETIRED_STREAM_NAME.to_string(),
                subjects: vec![format!("{NATS_SUBJECT_PREFIX}.>")],
                storage: async_nats::jetstream::stream::StorageType::Memory,
                ..Default::default()
            })
            .await
            .unwrap();

        let publisher = NatsEventDelivery::connect(&broker.url()).await.unwrap();
        let retired = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while jetstream.get_stream(RETIRED_STREAM_NAME).await.is_ok() {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await;
        assert!(retired.is_ok(), "the replay stream is removed");
        publisher
            .publish(&make_test_event(Uuid::new_v4()))
            .await
            .unwrap();
        publisher.client.flush().await.unwrap();
        assert!(jetstream.get_stream(RETIRED_STREAM_NAME).await.is_err());
        publisher.shutdown().await;
    }

    #[tokio::test]
    async fn nats_restart_keeps_real_event_service_delta_and_durable_delivery() {
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
        let mut subscriber = delivery.subscribe(session.uuid()).await.unwrap();
        let delta = service
            .emit(EventRequest::new(session, EventContext::empty(), data))
            .await
            .unwrap();
        assert!(delta.sequence.is_none());
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

    /// End-to-end check of EVERRUNS-32 against the real NATS server
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
        assert!(serde_json::to_vec(&event).unwrap().len() > nats.client.max_payload());

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
