//! Bounding how often one call site reports a failure that keeps repeating.
//!
//! EVE-1328: a Valkey/Postgres saturation window turned every request and
//! poll tick into its own error event and exhausted the embedder's reporting
//! budget. The first occurrence at a site is reported immediately; after that
//! the site reports at most once per interval, naming how many occurrences it
//! suppressed. Suppressed occurrences are still logged, at `info`: the
//! embedder's error reporter treats `warn` as a reported issue too.
//!
//! Decision: this is a local copy of `everruns_core::report_throttle`, not a
//! dependency on it, for the same reason as `persistence::db_failure`:
//! `everruns-durable` takes no `everruns-*` dependencies. Keep the two in step;
//! the rationale lives in the core module.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a repeating failure at one site stays suppressed before it is
/// reported again.
///
/// Ten minutes caps one site at six reports an hour through any outage, while
/// still restating a standing fault several times an hour.
pub(crate) const REPEATED_FAILURE_REPORT_INTERVAL: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, Copy)]
struct SiteState {
    reported_at: Instant,
    suppressed: u64,
}

/// Decides whether an occurrence of a repeating failure is reported, per
/// `(site, class)`.
#[derive(Debug)]
pub(crate) struct ReportThrottle {
    interval: Duration,
    sites: Mutex<BTreeMap<(&'static str, &'static str), SiteState>>,
}

impl ReportThrottle {
    /// A throttle usable as a `static`.
    pub(crate) const fn new(interval: Duration) -> Self {
        Self {
            interval,
            sites: Mutex::new(BTreeMap::new()),
        }
    }

    /// The re-report interval, for the report's own fields.
    pub(crate) fn interval(&self) -> Duration {
        self.interval
    }

    /// Note one occurrence now. See [`Self::admit_at`].
    pub(crate) fn admit(&self, site: &'static str, class: &'static str) -> Option<u64> {
        self.admit_at(site, class, Instant::now())
    }

    /// Note one occurrence at `now`.
    ///
    /// `Some(suppressed)` means report it, naming how many occurrences were
    /// suppressed since the previous report. `None` means it is suppressed and
    /// counted towards the next report.
    pub(crate) fn admit_at(
        &self,
        site: &'static str,
        class: &'static str,
        now: Instant,
    ) -> Option<u64> {
        // A poisoned lock must not silence reporting: the map holds plain
        // counters with no invariant a panic could break.
        let mut sites = self
            .sites
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match sites.get_mut(&(site, class)) {
            Some(state) if now.saturating_duration_since(state.reported_at) < self.interval => {
                state.suppressed += 1;
                None
            }
            Some(state) => {
                let suppressed = state.suppressed;
                *state = SiteState {
                    reported_at: now,
                    suppressed: 0,
                };
                Some(suppressed)
            }
            None => {
                sites.insert(
                    (site, class),
                    SiteState {
                        reported_at: now,
                        suppressed: 0,
                    },
                );
                Some(0)
            }
        }
    }
}

/// The process-wide throttle behind keyed reports such as
/// [`crate::persistence::db_failure::log_database_failure`].
pub(crate) static REPEATED_FAILURES: ReportThrottle =
    ReportThrottle::new(REPEATED_FAILURE_REPORT_INTERVAL);

/// Log a failure that may repeat on every poll tick, bounded per call site.
/// Same contract as `everruns_core::report_repeated_failure!`.
macro_rules! report_repeated_failure {
    ($level:ident, $($arg:tt)+) => {{
        static THROTTLE: $crate::report_throttle::ReportThrottle =
            $crate::report_throttle::ReportThrottle::new(
                $crate::report_throttle::REPEATED_FAILURE_REPORT_INTERVAL,
            );
        match THROTTLE.admit("", "") {
            ::core::option::Option::Some(suppressed) => ::tracing::$level!(
                suppressed_since_last_report = suppressed,
                report_interval_seconds = THROTTLE.interval().as_secs(),
                $($arg)+
            ),
            ::core::option::Option::None => ::tracing::info!(
                report_suppressed = true,
                $($arg)+
            ),
        }
    }};
}
pub(crate) use report_repeated_failure;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::SubscriberExt;

    #[test]
    fn matches_the_shared_core_interval() {
        // Drift guard for the local copy.
        assert_eq!(
            REPEATED_FAILURE_REPORT_INTERVAL,
            everruns_core::report_throttle::REPEATED_FAILURE_REPORT_INTERVAL
        );
    }

    #[test]
    fn a_sustained_outage_reports_once_per_interval_with_the_gap_counted() {
        let throttle = ReportThrottle::new(Duration::from_secs(600));
        let start = Instant::now();
        let reports: Vec<(u64, u64)> = (0..3600u64)
            .filter_map(|second| {
                throttle
                    .admit_at(
                        "durable.workers.heartbeat",
                        "",
                        start + Duration::from_secs(second),
                    )
                    .map(|suppressed| (second, suppressed))
            })
            .collect();
        assert_eq!(
            reports,
            vec![
                (0, 0),
                (600, 599),
                (1200, 599),
                (1800, 599),
                (2400, 599),
                (3000, 599)
            ]
        );
    }

    #[test]
    fn the_macro_reports_once_then_logs_the_rest_below_warn() {
        struct Levels(Arc<Mutex<Vec<tracing::Level>>>);
        impl<S: tracing::Subscriber> Layer<S> for Levels {
            fn on_event(
                &self,
                event: &tracing::Event<'_>,
                _ctx: tracing_subscriber::layer::Context<'_, S>,
            ) {
                self.0.lock().unwrap().push(*event.metadata().level());
            }
        }
        let levels = Arc::new(Mutex::new(Vec::new()));
        let subscriber =
            tracing_subscriber::registry::Registry::default().with(Levels(Arc::clone(&levels)));
        tracing::subscriber::with_default(subscriber, || {
            for _ in 0..50 {
                report_repeated_failure!(error, error = "timed out", "repeating failure");
            }
        });
        let levels = levels.lock().unwrap().clone();
        assert_eq!(levels.len(), 50, "suppressed occurrences are still logged");
        assert_eq!(levels[0], tracing::Level::ERROR);
        assert!(
            levels[1..].iter().all(|l| *l == tracing::Level::INFO),
            "{levels:?}"
        );
    }
}
