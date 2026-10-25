// Composite Event Listener (EVE-876)
//
// Fan-out implementation of `crate::EventListener` that forwards each
// event to multiple inner listeners with panic isolation. Moved here from
// `crate::event_listeners` so core keeps only the neutral
// event/listener contracts while the runtime fan-out behavior (task spawning,
// failure containment, ordering) lives with the observability implementations.

use std::sync::Arc;

use crate::EventListener;
use crate::events::Event;
use async_trait::async_trait;

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
    fn prepare_causal_context(&self, request: &crate::events::EventRequest) -> Option<String> {
        for listener in &self.listeners {
            if let Ok(Some(carrier)) =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    listener.prepare_causal_context(request)
                }))
            {
                return Some(carrier);
            }
        }
        None
    }
    fn turn_causal_context(
        &self,
        context: &crate::events::EventContext,
    ) -> crate::causal::CausalContext {
        let mut causes = crate::causal::CausalContext::default();
        for listener in &self.listeners {
            if let Ok(value) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                listener.turn_causal_context(context)
            })) {
                causes.merge(&value);
            }
        }
        causes
    }
    fn restore_turn_context(
        &self,
        context: &crate::events::EventContext,
        metadata: Option<&serde_json::Value>,
    ) {
        for listener in &self.listeners {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                listener.restore_turn_context(context, metadata)
            }));
        }
    }

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

    async fn flush(&self) -> Result<(), String> {
        let mut failures = Vec::new();
        for listener in &self.listeners {
            let name = listener.name();
            let listener = listener.clone();
            match tokio::spawn(async move { listener.flush().await }).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => failures.push(format!("{name}: {error}")),
                Err(_) => failures.push(format!("{name}: flush panicked or was cancelled")),
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("; "))
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
    use crate::NoopEventListener;
    use crate::events::{EventContext, EventData, InputMessageData};
    use crate::message::RuntimeMessage;
    use everruns_contracts::typed_id::SessionId;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn create_test_event() -> Event {
        Event::new(
            SessionId::new(),
            EventContext::empty(),
            EventData::InputMessage(InputMessageData {
                message: RuntimeMessage::user("Hello"),
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

#[cfg(test)]
mod flush_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct FlushListener {
        calls: Arc<AtomicUsize>,
        fail: bool,
    }
    #[async_trait]
    impl EventListener for FlushListener {
        async fn on_event(&self, _: &Event) {}
        async fn flush(&self) -> Result<(), String> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if self.fail {
                Err("synthetic failure".into())
            } else {
                Ok(())
            }
        }
    }
    #[tokio::test]
    async fn composite_flush_reports_failure_and_flushes_other_listeners() {
        let calls = Arc::new(AtomicUsize::new(0));
        let listener = CompositeEventListener::new(vec![
            Arc::new(FlushListener {
                calls: calls.clone(),
                fail: true,
            }),
            Arc::new(FlushListener {
                calls: calls.clone(),
                fail: false,
            }),
        ]);
        assert!(
            listener
                .flush()
                .await
                .unwrap_err()
                .contains("synthetic failure")
        );
        assert_eq!(calls.load(Ordering::Relaxed), 2);
    }
}
