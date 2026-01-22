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
    /// Return `Some(vec!["llm.generation", "tool.call_completed"])` to filter.
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

// ============================================================================
// CompositeEventListener
// ============================================================================

use std::sync::Arc;

/// Composite listener that forwards events to multiple listeners.
///
/// This is useful when you want to combine multiple listeners into one,
/// such as OTel + metrics + audit logging.
pub struct CompositeEventListener {
    listeners: Vec<Arc<dyn EventListener>>,
}

impl CompositeEventListener {
    /// Create a new composite listener with multiple inner listeners.
    pub fn new(listeners: Vec<Arc<dyn EventListener>>) -> Self {
        Self { listeners }
    }

    /// Add a listener to the composite.
    pub fn add(&mut self, listener: Arc<dyn EventListener>) {
        self.listeners.push(listener);
    }

    /// Get the number of registered listeners.
    pub fn len(&self) -> usize {
        self.listeners.len()
    }

    /// Check if there are no registered listeners.
    pub fn is_empty(&self) -> bool {
        self.listeners.is_empty()
    }
}

#[async_trait]
impl EventListener for CompositeEventListener {
    /// Forward event to all inner listeners with error isolation.
    ///
    /// Each listener is called in isolation - if a listener panics,
    /// other listeners are still notified. This ensures misbehaving
    /// listeners cannot disrupt event processing.
    async fn on_event(&self, event: &Event) {
        for listener in &self.listeners {
            // Check if listener wants this event type
            if let Some(types) = listener.event_types()
                && !types.contains(&event.event_type.as_str())
            {
                continue;
            }

            let listener_name = listener.name();
            let listener = listener.clone();
            let event = event.clone();

            // Spawn listener in isolated task to catch panics
            let handle = tokio::spawn(async move {
                listener.on_event(&event).await;
            });

            // Wait for completion, log but don't propagate panics
            if let Err(e) = handle.await {
                tracing::error!(
                    listener = listener_name,
                    error = %e,
                    "EventListener panicked or was cancelled"
                );
            }
        }
    }

    fn name(&self) -> &'static str {
        "CompositeEventListener"
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{EventContext, EventData, MessageUserData};
    use crate::message::Message;
    use crate::typed_id::SessionId;
    use std::sync::atomic::{AtomicU32, Ordering};
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

    #[tokio::test]
    async fn test_event_listener_with_filtered_types() {
        struct FilteredListener;

        #[async_trait]
        impl EventListener for FilteredListener {
            async fn on_event(&self, _event: &Event) {}

            fn event_types(&self) -> Option<Vec<&'static str>> {
                Some(vec!["message.user", "llm.generation"])
            }

            fn name(&self) -> &'static str {
                "FilteredListener"
            }
        }

        let listener = FilteredListener;
        let types = listener.event_types().unwrap();
        assert_eq!(types.len(), 2);
        assert!(types.contains(&"message.user"));
        assert!(types.contains(&"llm.generation"));
        assert_eq!(listener.name(), "FilteredListener");
    }

    #[tokio::test]
    async fn test_composite_listener_empty() {
        let composite = CompositeEventListener::new(vec![]);
        assert!(composite.is_empty());
        assert_eq!(composite.len(), 0);
        assert_eq!(composite.name(), "CompositeEventListener");

        // Should not panic with empty listeners
        let event = create_test_event();
        composite.on_event(&event).await;
    }

    #[tokio::test]
    async fn test_composite_listener_multiple() {
        // Counter to track how many times listeners are called
        struct CountingListener {
            count: Arc<AtomicU32>,
            name: &'static str,
        }

        #[async_trait]
        impl EventListener for CountingListener {
            async fn on_event(&self, _event: &Event) {
                self.count.fetch_add(1, Ordering::SeqCst);
            }

            fn name(&self) -> &'static str {
                self.name
            }
        }

        let count1 = Arc::new(AtomicU32::new(0));
        let count2 = Arc::new(AtomicU32::new(0));

        let listener1 = Arc::new(CountingListener {
            count: count1.clone(),
            name: "Listener1",
        });
        let listener2 = Arc::new(CountingListener {
            count: count2.clone(),
            name: "Listener2",
        });

        let composite = CompositeEventListener::new(vec![listener1, listener2]);
        assert_eq!(composite.len(), 2);
        assert!(!composite.is_empty());

        let event = create_test_event();
        composite.on_event(&event).await;

        assert_eq!(count1.load(Ordering::SeqCst), 1);
        assert_eq!(count2.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_composite_listener_with_filtering() {
        struct SelectiveListener {
            count: Arc<AtomicU32>,
            filter: Vec<&'static str>,
        }

        #[async_trait]
        impl EventListener for SelectiveListener {
            async fn on_event(&self, _event: &Event) {
                self.count.fetch_add(1, Ordering::SeqCst);
            }

            fn event_types(&self) -> Option<Vec<&'static str>> {
                Some(self.filter.clone())
            }
        }

        let count1 = Arc::new(AtomicU32::new(0));
        let count2 = Arc::new(AtomicU32::new(0));

        // Listener 1 wants message.user events
        let listener1 = Arc::new(SelectiveListener {
            count: count1.clone(),
            filter: vec!["message.user"],
        });

        // Listener 2 wants llm.generation events (won't match our test event)
        let listener2 = Arc::new(SelectiveListener {
            count: count2.clone(),
            filter: vec!["llm.generation"],
        });

        let composite = CompositeEventListener::new(vec![listener1, listener2]);

        // Send a message.user event
        let event = create_test_event();
        composite.on_event(&event).await;

        // Only listener1 should have been called
        assert_eq!(count1.load(Ordering::SeqCst), 1);
        assert_eq!(count2.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_composite_listener_add() {
        let mut composite = CompositeEventListener::new(vec![]);
        assert!(composite.is_empty());

        composite.add(Arc::new(NoopEventListener));
        assert_eq!(composite.len(), 1);

        composite.add(Arc::new(NoopEventListener));
        assert_eq!(composite.len(), 2);
    }

    fn create_test_event() -> Event {
        Event::new(
            SessionId::from_uuid(Uuid::now_v7()),
            EventContext::empty(),
            EventData::MessageUser(MessageUserData {
                message: Message::user("Hello"),
            }),
        )
    }

    #[tokio::test]
    async fn test_composite_listener_isolates_panics() {
        // A listener that always panics
        struct PanickingListener;

        #[async_trait]
        impl EventListener for PanickingListener {
            async fn on_event(&self, _event: &Event) {
                panic!("This listener always panics!");
            }

            fn name(&self) -> &'static str {
                "PanickingListener"
            }
        }

        // A counter listener to verify it still gets called
        struct CountingListener {
            count: Arc<AtomicU32>,
        }

        #[async_trait]
        impl EventListener for CountingListener {
            async fn on_event(&self, _event: &Event) {
                self.count.fetch_add(1, Ordering::SeqCst);
            }

            fn name(&self) -> &'static str {
                "CountingListener"
            }
        }

        let count = Arc::new(AtomicU32::new(0));

        // Put panicking listener BEFORE counting listener
        let panicking = Arc::new(PanickingListener) as Arc<dyn EventListener>;
        let counting = Arc::new(CountingListener {
            count: count.clone(),
        }) as Arc<dyn EventListener>;

        let composite = CompositeEventListener::new(vec![panicking, counting]);

        let event = create_test_event();

        // This should NOT panic - the panicking listener should be isolated
        composite.on_event(&event).await;

        // The counting listener should still have been called
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "Listener after panicking listener should still execute"
        );
    }

    #[tokio::test]
    async fn test_composite_listener_continues_after_panic() {
        // Multiple listeners with a panicking one in the middle
        struct PanickingListener;

        #[async_trait]
        impl EventListener for PanickingListener {
            async fn on_event(&self, _event: &Event) {
                panic!("Middle listener panics!");
            }

            fn name(&self) -> &'static str {
                "PanickingListener"
            }
        }

        struct CountingListener {
            count: Arc<AtomicU32>,
            name: &'static str,
        }

        #[async_trait]
        impl EventListener for CountingListener {
            async fn on_event(&self, _event: &Event) {
                self.count.fetch_add(1, Ordering::SeqCst);
            }

            fn name(&self) -> &'static str {
                self.name
            }
        }

        let count_before = Arc::new(AtomicU32::new(0));
        let count_after = Arc::new(AtomicU32::new(0));

        let listener_before = Arc::new(CountingListener {
            count: count_before.clone(),
            name: "BeforeListener",
        }) as Arc<dyn EventListener>;

        let panicking = Arc::new(PanickingListener) as Arc<dyn EventListener>;

        let listener_after = Arc::new(CountingListener {
            count: count_after.clone(),
            name: "AfterListener",
        }) as Arc<dyn EventListener>;

        let composite =
            CompositeEventListener::new(vec![listener_before, panicking, listener_after]);

        let event = create_test_event();
        composite.on_event(&event).await;

        // Both before and after listeners should have been called
        assert_eq!(
            count_before.load(Ordering::SeqCst),
            1,
            "Listener before panicking listener should execute"
        );
        assert_eq!(
            count_after.load(Ordering::SeqCst),
            1,
            "Listener after panicking listener should execute"
        );
    }
}
