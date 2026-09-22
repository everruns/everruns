//! Turning a persistence failure into a `StoreError`, logged so that one
//! infrastructure incident reads as one incident.
//!
//! EVE-1071: a pool exhaustion in prod became four separate Sentry issues
//! because each background poller rendered the same underlying failure into
//! its own message, and grouping keys on the message. The shared wording and
//! classification live in [`everruns_core::database_failure`]; this module is
//! how the durable crate reaches them, and is where that behaviour is proven
//! — EVE-876 keeps a tracing subscriber, and therefore this test, out of core.

use everruns_core::log_database_failure;

use super::store::StoreError;

/// Log a persistence failure under the wording its kind deserves, then wrap it
/// as a [`StoreError`].
///
/// `fallback` is the message for an ordinary failure — a query bug, a
/// constraint violation — where the call site is the interesting thing. A
/// shared incident (pool exhaustion, a dropped connection) overrides it with
/// the canonical wording so every starved subsystem groups together, keeping
/// its identity in the `subsystem` field.
pub(crate) fn store_failure(
    subsystem: &'static str,
    fallback: &'static str,
    error: sqlx::Error,
) -> StoreError {
    let rendered = error.to_string();
    log_database_failure(subsystem, fallback, &rendered);
    StoreError::Database(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::DatabaseFailureKind;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::{Context, SubscriberExt};
    use tracing_subscriber::registry::Registry;

    /// What a call actually emitted: the rendered message plus its fields.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Emitted {
        message: String,
        subsystem: String,
        db_failure: String,
    }

    #[derive(Default)]
    struct Visitor {
        fields: std::collections::BTreeMap<String, String>,
    }

    impl tracing::field::Visit for Visitor {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.fields
                .insert(field.name().to_string(), format!("{value:?}"));
        }

        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.fields
                .insert(field.name().to_string(), value.to_string());
        }
    }

    struct CaptureLayer(Arc<Mutex<Vec<Emitted>>>);

    impl<S: tracing::Subscriber> Layer<S> for CaptureLayer {
        fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
            let mut visitor = Visitor::default();
            event.record(&mut visitor);
            let get = |key: &str| visitor.fields.get(key).cloned().unwrap_or_default();
            self.0.lock().unwrap().push(Emitted {
                message: get("message"),
                subsystem: get("subsystem"),
                db_failure: get("db_failure"),
            });
        }
    }

    fn capture(emit: impl FnOnce()) -> Vec<Emitted> {
        let events = Arc::new(Mutex::new(Vec::new()));
        let subscriber = Registry::default().with(CaptureLayer(Arc::clone(&events)));
        let guard = tracing::subscriber::set_default(subscriber);
        emit();
        drop(guard);
        events.lock().unwrap().clone()
    }

    #[test]
    fn every_starved_subsystem_reports_one_incident_under_one_message() {
        // The regression EVE-1071 describes: four pollers, one pool timeout,
        // four Sentry issues. Grouping keys on the message, so the four must
        // emit identical wording and differ only in `subsystem`.
        let emitted = capture(|| {
            for (subsystem, fallback) in [
                ("durable.schedules.claim", "claim due schedules failed"),
                ("durable.tasks.claim", "Failed to claim tasks"),
                ("durable.scheduler.poll", "failed to process due schedules"),
                ("server.observers.scoring", "Observer scoring batch failed"),
            ] {
                log_database_failure(subsystem, fallback, &sqlx::Error::PoolTimedOut.to_string());
            }
        });

        assert_eq!(emitted.len(), 4);
        let messages: std::collections::BTreeSet<_> =
            emitted.iter().map(|e| e.message.as_str()).collect();
        assert_eq!(
            messages,
            std::collections::BTreeSet::from(["database connection pool exhausted"]),
            "one incident must render as one message: {emitted:#?}"
        );
        // The identity the shared message gives up is carried in a field.
        let subsystems: std::collections::BTreeSet<_> =
            emitted.iter().map(|e| e.subsystem.as_str()).collect();
        assert_eq!(subsystems.len(), 4, "{emitted:#?}");
        assert!(
            emitted
                .iter()
                .all(|e| e.db_failure == "database.pool_exhausted"),
            "{emitted:#?}"
        );
    }

    #[test]
    fn an_ordinary_failure_keeps_the_call_sites_own_message() {
        let emitted = capture(|| {
            store_failure(
                "durable.tasks.claim",
                "Failed to claim tasks",
                sqlx::Error::RowNotFound,
            );
        });

        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].message, "Failed to claim tasks");
        assert_eq!(emitted[0].db_failure, "database.other");
    }

    #[test]
    fn the_wrapped_error_keeps_the_drivers_own_text() {
        let error = store_failure(
            "durable.schedules.claim",
            "claim due schedules failed",
            sqlx::Error::PoolTimedOut,
        );
        let StoreError::Database(rendered) = error else {
            panic!("expected StoreError::Database, got {error:?}");
        };
        // Only the *log* message is canonicalised; the error a caller sees
        // still names what actually happened.
        assert_eq!(rendered, sqlx::Error::PoolTimedOut.to_string());
        assert_eq!(
            DatabaseFailureKind::classify(&rendered),
            DatabaseFailureKind::PoolExhausted
        );
    }
}
