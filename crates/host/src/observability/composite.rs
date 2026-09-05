// Composite Event Listener (EVE-876)
//
// Fan-out implementation of `everruns_core::EventListener` that forwards each
// event to multiple inner listeners with panic isolation. Moved here from
// `everruns_core::event_listeners` so core keeps only the neutral
// event/listener contracts while the runtime fan-out behavior (task spawning,
// failure containment, ordering) lives with the observability implementations.

use std::sync::Arc;

use async_trait::async_trait;
use everruns_core::EventListener;
use everruns_core::events::Event;

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
    use everruns_core::NoopEventListener;
    use everruns_core::events::{EventContext, EventData, InputMessageData};
    use everruns_core::message::Message;
    use everruns_provider::typed_id::SessionId;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn create_test_event() -> Event {
        Event::new(
            SessionId::new(),
            EventContext::empty(),
            EventData::InputMessage(InputMessageData {
                message: Message::user("Hello"),
            }),
        )
    }

    struct CountingListener {
        count: Arc<AtomicU32>,
        expected: serde_json::Value,
        filter: Option<Vec<&'static str>>,
    }

    #[async_trait]
    impl EventListener for CountingListener {
        async fn on_event(&self, event: &Event) {
            // A wrong event must not count as successful notification, even
            // though the composite deliberately isolates callback panics.
            assert_eq!(serde_json::to_value(event).unwrap(), self.expected);
            self.count.fetch_add(1, Ordering::SeqCst);
        }

        fn event_types(&self) -> Option<Vec<&'static str>> {
            self.filter.clone()
        }
    }

    fn counting(
        event: &Event,
        count: &Arc<AtomicU32>,
        filter: Option<Vec<&'static str>>,
    ) -> Arc<dyn EventListener> {
        Arc::new(CountingListener {
            count: count.clone(),
            expected: serde_json::to_value(event).unwrap(),
            filter,
        })
    }

    #[tokio::test]
    async fn test_composite_listener_multiple() {
        let event = create_test_event();
        let first = Arc::new(AtomicU32::new(0));
        let second = Arc::new(AtomicU32::new(0));
        let composite = CompositeEventListener::new(vec![
            counting(&event, &first, None),
            counting(&event, &second, None),
        ]);
        composite.on_event(&event).await;
        assert_eq!(first.load(Ordering::SeqCst), 1);
        assert_eq!(second.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_composite_listener_with_filtering() {
        let event = create_test_event();
        let matched = Arc::new(AtomicU32::new(0));
        let other_type = Arc::new(AtomicU32::new(0));
        let empty_filter = Arc::new(AtomicU32::new(0));
        let composite = CompositeEventListener::new(vec![
            counting(&event, &matched, Some(vec!["input.message"])),
            counting(&event, &other_type, Some(vec!["llm.generation"])),
            counting(&event, &empty_filter, Some(vec![])),
        ]);
        composite.on_event(&event).await;
        assert_eq!(matched.load(Ordering::SeqCst), 1);
        assert_eq!(other_type.load(Ordering::SeqCst), 0);
        assert_eq!(empty_filter.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_composite_listener_add() {
        let event = create_test_event();
        let first = Arc::new(AtomicU32::new(0));
        let second = Arc::new(AtomicU32::new(0));
        let mut composite = CompositeEventListener::new(vec![]);
        assert!(composite.is_empty());
        composite.on_event(&event).await;

        composite.add(Arc::new(NoopEventListener));
        composite.add(counting(&event, &first, None));
        assert!(!composite.is_empty());
        composite.on_event(&event).await;
        assert_eq!(first.load(Ordering::SeqCst), 1);

        composite.add(counting(&event, &second, None));
        assert_eq!(composite.len(), 3);
        composite.on_event(&event).await;
        assert_eq!(first.load(Ordering::SeqCst), 2);
        assert_eq!(second.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_composite_listener_isolates_panics() {
        struct PanickingListener;

        #[async_trait]
        impl EventListener for PanickingListener {
            async fn on_event(&self, _event: &Event) {
                panic!("listener failure");
            }
        }

        let event = create_test_event();
        for include_before in [false, true] {
            let before = Arc::new(AtomicU32::new(0));
            let after = Arc::new(AtomicU32::new(0));
            let mut listeners: Vec<Arc<dyn EventListener>> = Vec::new();
            if include_before {
                listeners.push(counting(&event, &before, None));
            }
            listeners.push(Arc::new(PanickingListener));
            listeners.push(counting(&event, &after, None));
            CompositeEventListener::new(listeners)
                .on_event(&event)
                .await;
            assert_eq!(before.load(Ordering::SeqCst), u32::from(include_before));
            assert_eq!(after.load(Ordering::SeqCst), 1);
        }
    }
}
