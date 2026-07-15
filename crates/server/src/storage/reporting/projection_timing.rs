use std::time::Duration;

use tracing::warn;

/// Stable identifiers for reporting projection SQL families.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ProjectionQueryFamily {
    pub operation: &'static str,
    pub query_family: &'static str,
}

pub(crate) const FACT_SESSION_UPSERT: ProjectionQueryFamily = ProjectionQueryFamily {
    operation: "reporting.project_session",
    query_family: "fact_session_upsert",
};

/// Host-supplied deployment context for slow-projection warnings.
#[derive(Debug, Clone, Default)]
struct ProjectionHostContext {
    environment: Option<String>,
    service_version: Option<String>,
}

impl ProjectionHostContext {
    fn from_env() -> Self {
        Self {
            environment: std::env::var("OTEL_ENVIRONMENT").ok(),
            service_version: std::env::var("OTEL_SERVICE_VERSION").ok(),
        }
    }
}

fn slow_threshold() -> Duration {
    const DEFAULT_MS: u64 = 1_000;
    let millis = std::env::var("REPORTING_PROJECTION_SLOW_THRESHOLD_MS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_MS);
    Duration::from_millis(millis.max(1))
}

fn warn_slow_projection(
    family: ProjectionQueryFamily,
    elapsed: Duration,
    rows_affected: u64,
    source_type: &str,
    event_kind: Option<&str>,
    org_id: i64,
    threshold: Duration,
) {
    if elapsed < threshold {
        return;
    }

    let host = ProjectionHostContext::from_env();
    warn!(
        operation = family.operation,
        query_family = family.query_family,
        elapsed_ms = elapsed.as_millis() as u64,
        rows_affected,
        source_type,
        event_kind,
        org_id,
        environment = host.environment.as_deref(),
        service_version = host.service_version.as_deref(),
        "Reporting projection query exceeded slow threshold"
    );
}

/// Emit one structured warning when projection work crosses the slow threshold.
/// Includes only stable operation metadata — no tenant content, SQL parameters,
/// or PII.
pub(crate) fn maybe_warn_slow_projection(
    family: ProjectionQueryFamily,
    elapsed: Duration,
    rows_affected: u64,
    source_type: &str,
    event_kind: Option<&str>,
    org_id: i64,
) {
    warn_slow_projection(
        family,
        elapsed,
        rows_affected,
        source_type,
        event_kind,
        org_id,
        slow_threshold(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tracing::field::{Field, Visit};
    use tracing_subscriber::layer::{Context, SubscriberExt};
    use tracing_subscriber::{Layer, Registry};

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

    type CapturedEvents = Arc<Mutex<Vec<Vec<(String, String)>>>>;

    struct CaptureLayer {
        events: CapturedEvents,
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

    fn field_map(fields: &[(String, String)]) -> std::collections::BTreeMap<&str, &str> {
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

        warn_slow_projection(
            FACT_SESSION_UPSERT,
            Duration::from_millis(50),
            1,
            "session",
            Some("session_snapshot"),
            42,
            Duration::from_millis(1),
        );

        let captured = events.lock().unwrap();
        assert_eq!(captured.len(), 1);
        let fields = field_map(&captured[0]);
        assert_eq!(fields.get("operation"), Some(&"reporting.project_session"));
        assert_eq!(fields.get("query_family"), Some(&"fact_session_upsert"));
        assert_eq!(fields.get("elapsed_ms"), Some(&"50"));
        assert_eq!(fields.get("rows_affected"), Some(&"1"));
        assert_eq!(fields.get("source_type"), Some(&"session"));
        assert_eq!(fields.get("event_kind"), Some(&"session_snapshot"));
        assert_eq!(fields.get("org_id"), Some(&"42"));
        assert_eq!(fields.get("environment"), Some(&"test"));
        assert_eq!(fields.get("service_version"), Some(&"1.2.3"));
        assert!(!fields.contains_key("session_id"));
        assert!(!fields.contains_key("source_id"));
        assert!(!fields.contains_key("sql"));
        assert!(!fields.contains_key("title"));

        unsafe {
            std::env::remove_var("OTEL_ENVIRONMENT");
            std::env::remove_var("OTEL_SERVICE_VERSION");
        }
    }

    #[test]
    fn fast_projection_emits_no_warning() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let layer = CaptureLayer {
            events: Arc::clone(&events),
        };
        let subscriber = Registry::default().with(layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        warn_slow_projection(
            FACT_SESSION_UPSERT,
            Duration::from_millis(10),
            1,
            "session",
            None,
            1,
            Duration::from_millis(1_000),
        );

        assert!(events.lock().unwrap().is_empty());
    }
}
