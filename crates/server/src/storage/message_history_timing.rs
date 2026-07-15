use std::time::Duration;

use tracing::warn;

/// Stable identifiers for message-history event-read families.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MessageHistoryQueryFamily {
    pub operation: &'static str,
    pub query_family: &'static str,
}

pub(crate) const MESSAGE_HISTORY_EVENTS: MessageHistoryQueryFamily = MessageHistoryQueryFamily {
    operation: "storage.list_message_events",
    query_family: "message_history_events",
};

pub(crate) const FILTERED_MESSAGE_HISTORY_EVENTS: MessageHistoryQueryFamily =
    MessageHistoryQueryFamily {
        operation: "storage.list_message_events_filtered",
        query_family: "filtered_message_history_events",
    };

#[derive(Debug, Clone, Default)]
struct MessageHistoryHostContext {
    environment: Option<String>,
    service_version: Option<String>,
}

impl MessageHistoryHostContext {
    fn from_env() -> Self {
        Self {
            environment: std::env::var("OTEL_ENVIRONMENT").ok(),
            service_version: std::env::var("OTEL_SERVICE_VERSION").ok(),
        }
    }
}

fn slow_threshold() -> Duration {
    const DEFAULT_MS: u64 = 1_000;
    let millis = std::env::var("MESSAGE_HISTORY_SLOW_THRESHOLD_MS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_MS);
    Duration::from_millis(millis.max(1))
}

fn warn_slow_message_history(
    family: MessageHistoryQueryFamily,
    elapsed: Duration,
    row_count: u64,
    limit: i64,
    threshold: Duration,
) {
    if elapsed < threshold {
        return;
    }

    let host = MessageHistoryHostContext::from_env();
    warn!(
        operation = family.operation,
        query_family = family.query_family,
        elapsed_ms = elapsed.as_millis() as u64,
        row_count,
        limit,
        environment = host.environment.as_deref(),
        service_version = host.service_version.as_deref(),
        "Message history event read exceeded slow threshold"
    );
}

/// Emit one structured warning when a message-history event read crosses the
/// slow threshold. Includes only stable operation metadata: no SQL, parameter
/// values, session ids, org ids, event payload, or message content.
pub(crate) fn maybe_warn_slow_message_history(
    family: MessageHistoryQueryFamily,
    elapsed: Duration,
    row_count: u64,
    limit: i64,
) {
    warn_slow_message_history(family, elapsed, row_count, limit, slow_threshold());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tracing::field::{Field, Visit};
    use tracing_subscriber::layer::{Context, SubscriberExt};
    use tracing_subscriber::{Layer, Registry};

    type CapturedFields = Vec<Vec<(String, String)>>;

    struct CaptureVisitor {
        fields: Vec<(String, String)>,
    }

    impl Visit for CaptureVisitor {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.fields
                .push((field.name().to_string(), format!("{value:?}")));
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            self.fields
                .push((field.name().to_string(), value.to_string()));
        }

        fn record_u64(&mut self, field: &Field, value: u64) {
            self.fields
                .push((field.name().to_string(), value.to_string()));
        }

        fn record_i64(&mut self, field: &Field, value: i64) {
            self.fields
                .push((field.name().to_string(), value.to_string()));
        }
    }

    struct CaptureLayer {
        events: Arc<Mutex<CapturedFields>>,
    }

    impl<S> Layer<S> for CaptureLayer
    where
        S: tracing::Subscriber,
    {
        fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
            let mut visitor = CaptureVisitor { fields: Vec::new() };
            event.record(&mut visitor);
            self.events.lock().unwrap().push(visitor.fields);
        }
    }

    fn field_map(fields: &[(String, String)]) -> std::collections::HashMap<&str, &str> {
        fields
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect()
    }

    #[test]
    fn slow_warning_includes_required_fields_and_redacts_sensitive_payload() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let layer = CaptureLayer {
            events: Arc::clone(&events),
        };
        let subscriber = Registry::default().with(layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        unsafe {
            std::env::set_var("OTEL_ENVIRONMENT", "test");
            std::env::set_var("OTEL_SERVICE_VERSION", "1.2.3");
        }

        warn_slow_message_history(
            MESSAGE_HISTORY_EVENTS,
            Duration::from_millis(50),
            2_000,
            2_000,
            Duration::from_millis(1),
        );

        let captured = events.lock().unwrap();
        assert_eq!(captured.len(), 1);
        let fields = field_map(&captured[0]);
        assert_eq!(
            fields.get("operation"),
            Some(&"storage.list_message_events")
        );
        assert_eq!(fields.get("query_family"), Some(&"message_history_events"));
        assert_eq!(fields.get("elapsed_ms"), Some(&"50"));
        assert_eq!(fields.get("row_count"), Some(&"2000"));
        assert_eq!(fields.get("limit"), Some(&"2000"));
        assert_eq!(fields.get("environment"), Some(&"test"));
        assert_eq!(fields.get("service_version"), Some(&"1.2.3"));
        assert!(!fields.contains_key("session_id"));
        assert!(!fields.contains_key("org_id"));
        assert!(!fields.contains_key("sql"));
        assert!(!fields.contains_key("params"));
        assert!(!fields.contains_key("content"));
        assert!(!fields.contains_key("data"));

        unsafe {
            std::env::remove_var("OTEL_ENVIRONMENT");
            std::env::remove_var("OTEL_SERVICE_VERSION");
        }
    }

    #[test]
    fn fast_read_emits_no_warning() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let layer = CaptureLayer {
            events: Arc::clone(&events),
        };
        let subscriber = Registry::default().with(layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        warn_slow_message_history(
            MESSAGE_HISTORY_EVENTS,
            Duration::from_millis(10),
            100,
            2_000,
            Duration::from_millis(50),
        );

        assert!(events.lock().unwrap().is_empty());
    }
}
