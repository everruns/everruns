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
    /// Create from environment. Uses NATS if `NATS_URL` is set, otherwise in-memory.
    pub async fn from_env() -> Self {
        match std::env::var("NATS_URL") {
            Ok(url) => match NatsEventDelivery::connect(&url).await {
                Ok(nats) => {
                    info!(url = %url, "Event delivery: NATS JetStream");
                    Self::Nats(Arc::new(nats))
                }
                Err(e) => {
                    warn!(
                        error = %e,
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
            Self::Nats(stream) => {
                use futures::StreamExt;
                match stream.next().await {
                    Some(Ok(msg)) => {
                        let event: Event = match serde_json::from_slice(&msg.payload) {
                            Ok(e) => e,
                            Err(e) => {
                                warn!(error = %e, "Failed to deserialize NATS event");
                                return None;
                            }
                        };
                        // Ack the message so JetStream doesn't redeliver
                        if let Err(e) = msg.ack().await {
                            warn!(error = %e, "Failed to ack NATS message");
                        }
                        Some(event)
                    }
                    Some(Err(e)) => {
                        warn!(error = %e, "NATS subscription error");
                        None
                    }
                    None => None,
                }
            }
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
        let client = async_nats::connect(url)
            .await
            .context("Failed to connect to NATS")?;

        let jetstream = async_nats::jetstream::new(client.clone());

        // Create or update the stream for session events
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

        jetstream
            .get_or_create_stream(stream_config)
            .await
            .context("Failed to create NATS JetStream stream")?;

        info!("NATS JetStream stream '{}' ready", NATS_STREAM_NAME);

        Ok(Self { client, jetstream })
    }

    pub async fn publish(&self, event: &Event) -> Result<()> {
        let subject = format!("{}.{}", NATS_SUBJECT_PREFIX, event.session_id.uuid());
        let payload = serde_json::to_vec(event).context("Failed to serialize event")?;

        self.jetstream
            .publish(subject, payload.into())
            .await
            .context("Failed to publish event to NATS")?
            .await
            .context("Failed to confirm NATS publish")?;

        Ok(())
    }

    pub async fn subscribe(&self, session_id: Uuid) -> Result<EventSubscription> {
        let subject = format!("{NATS_SUBJECT_PREFIX}.{session_id}");

        let stream = self
            .jetstream
            .get_stream(NATS_STREAM_NAME)
            .await
            .context("Failed to get NATS stream")?;

        // Create an ephemeral push consumer for this SSE connection.
        // DeliverPolicy::New means we only get events from now on.
        // For reconnection replay, the SSE handler queries PG separately.
        let consumer = stream
            .create_consumer(async_nats::jetstream::consumer::pull::Config {
                filter_subject: subject,
                deliver_policy: async_nats::jetstream::consumer::DeliverPolicy::New,
                ack_policy: async_nats::jetstream::consumer::AckPolicy::Explicit,
                ..Default::default()
            })
            .await
            .context("Failed to create NATS consumer")?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::events::{EventData, OutputMessageDeltaData};
    use everruns_core::typed_id::{EventId, SessionId, TurnId};

    fn make_test_event(session_id: Uuid) -> Event {
        Event {
            id: EventId::new(),
            event_type: "output.message.delta".to_string(),
            ts: chrono::Utc::now(),
            session_id: SessionId::from_uuid(session_id),
            context: Default::default(),
            data: EventData::OutputMessageDelta(OutputMessageDeltaData {
                turn_id: TurnId::new(),
                delta: "hello".to_string(),
                accumulated: "hello".to_string(),
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

    #[tokio::test]
    async fn test_in_memory_partitioning() {
        let delivery = InMemoryEventDelivery::new();
        let session_a = Uuid::new_v4();
        let session_b = Uuid::new_v4();

        // Subscribe to session A
        let mut sub_a = delivery.subscribe(session_a);

        // Publish to session B — subscriber for A should not see it
        // (unless they happen to share a partition, but that's filtered by caller)
        let event_b = make_test_event(session_b);
        delivery.publish(&event_b).unwrap();

        // Publish to session A
        let event_a = make_test_event(session_a);
        delivery.publish(&event_a).unwrap();

        // The subscriber may receive event_b if same partition, but that's OK —
        // the SSE handler filters by session_id. Just verify event_a arrives.
        match &mut sub_a {
            EventSubscription::InMemory(rx) => {
                // Drain until we find event_a or timeout
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

    #[test]
    fn test_event_delivery_in_memory_constructor() {
        let delivery = EventDelivery::in_memory();
        assert!(matches!(delivery, EventDelivery::InMemory(_)));
    }
}
