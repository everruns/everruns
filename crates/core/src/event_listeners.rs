// Event Listeners
//
// This module provides the EventListener trait for pluggable observability backends.
// Event listeners are notified after events are persisted, enabling:
// - OpenTelemetry span generation (gen-ai semantics)
// - External observability integrations (Datadog, NewRelic, etc.)
// - Analytics and metrics collection
// - Audit logging

use async_trait::async_trait;

use crate::events::Event;

// ============================================================================
// EventListener Trait
// ============================================================================

/// Trait for listening to events after they are emitted and stored.
///
/// Event listeners are notified synchronously after an event is persisted.
/// They can be used for:
/// - OpenTelemetry span generation (gen-ai semantics)
/// - External observability integrations (Datadog, NewRelic, etc.)
/// - Analytics and metrics collection
/// - Audit logging
///
/// Listeners should be fast and non-blocking. For heavy processing,
/// consider spawning background tasks.
///
/// # Example
///
/// ```ignore
/// use everruns_core::EventListener;
/// use everruns_core::events::Event;
///
/// struct MetricsListener;
///
/// #[async_trait]
/// impl EventListener for MetricsListener {
///     async fn on_event(&self, event: &Event) {
///         // Record metrics based on event type
///         metrics::counter!("events", "type" => event.event_type.clone());
///     }
/// }
/// ```
#[async_trait]
pub trait EventListener: Send + Sync {
    /// Called after an event is persisted.
    ///
    /// The event has already been stored in the database with its
    /// assigned ID and sequence number.
    async fn on_event(&self, event: &Event);

    /// Optional: Filter which event types this listener cares about.
    ///
    /// Return `None` to receive all events (default).
    /// Return `Some(vec!["llm.generation", "tool.completed"])` to filter.
    fn event_types(&self) -> Option<Vec<&'static str>> {
        None // Receive all events by default
    }

    /// Human-readable name for logging/debugging.
    fn name(&self) -> &'static str {
        "EventListener"
    }
}

// ============================================================================
// NoopEventListener
// ============================================================================

/// No-op event listener for when event listening is not needed.
///
/// This is useful for testing or when event observability is disabled.
#[derive(Debug, Clone, Default)]
pub struct NoopEventListener;

#[async_trait]
impl EventListener for NoopEventListener {
    async fn on_event(&self, _event: &Event) {
        // Do nothing
    }

    fn name(&self) -> &'static str {
        "NoopEventListener"
    }
}

// Note: `CompositeEventListener` (fan-out with panic isolation) lives in the
// `everruns-host/observability` feature. Core keeps only the neutral
// listener contract and the no-op implementation.

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{EventContext, EventData, InputMessageData};
    use crate::message::Message;
    use crate::typed_id::SessionId;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_noop_listener() {
        let listener = NoopEventListener;
        assert_eq!(listener.name(), "NoopEventListener");
        assert!(listener.event_types().is_none());

        // Should not panic
        let event = create_test_event();
        listener.on_event(&event).await;
    }

    #[tokio::test]
    async fn test_event_listener_default_event_types() {
        struct TestListener;

        #[async_trait]
        impl EventListener for TestListener {
            async fn on_event(&self, _event: &Event) {}
        }

        let listener = TestListener;
        assert!(listener.event_types().is_none());
        assert_eq!(listener.name(), "EventListener");
    }

    fn create_test_event() -> Event {
        Event::new(
            SessionId::from_uuid(Uuid::now_v7()),
            EventContext::empty(),
            EventData::InputMessage(InputMessageData {
                message: Message::user("Hello"),
            }),
        )
    }
}
