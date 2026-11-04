// Bounding how often one call site reports a failure that keeps repeating.
//
// EVE-1328: during a Valkey/Postgres saturation window, server and durable
// paths reported every occurrence of the same transient failure, one event per
// request or per poll tick (2,890 "Valkey rate limit check failed" events in
// about 20 hours). That exhausted the embedder's error-reporting budget and left
// production blind to everything else for days.
//
// Decision: bound the *report*, never the signal. The first occurrence at a
//   site is reported immediately at its own level; after that the site reports
//   at most once per interval, naming how many occurrences it suppressed in
//   between. Suppressed occurrences are still logged, at `info`, so a fail-open
//   rate limiter or a stalled poller stays visible in logs while it lasts.
// Decision: suppressed occurrences go to `info`, not `warn`. The embedder's
//   error reporter turns `warn` events into reported issues too (EVERRUNS-4B
//   was a `warn!`), so `warn` would not bound anything.
// Decision: time-bounded, not once-per-process. A standing fault must keep
//   saying so (the EVE-956 lesson, see `host::observability::braintrust_delivery`).
// Decision: keyed by `(site, class)`, both `&'static str`, so the key space is
//   bounded by the source code. A second, different failure class at the same
//   site is reported on its own schedule rather than hiding behind the first.
//
// `everruns-durable` keeps a local copy (`durable::report_throttle`) because
// it takes no `everruns-*` dependencies; keep the two in step.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[doc(hidden)]
pub use tracing as __tracing;

/// How long a repeating failure at one site stays suppressed before it is
/// reported again.
///
/// Ten minutes caps one site at six reports an hour through any outage, while
/// still restating a standing fault several times an hour.
pub const REPEATED_FAILURE_REPORT_INTERVAL: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, Copy)]
struct SiteState {
    reported_at: Instant,
    suppressed: u64,
}

/// Decides whether an occurrence of a repeating failure is reported, per
/// `(site, class)`.
#[derive(Debug)]
pub struct ReportThrottle {
    interval: Duration,
    sites: Mutex<BTreeMap<(&'static str, &'static str), SiteState>>,
}

impl ReportThrottle {
    /// A throttle usable as a `static`.
    pub const fn new(interval: Duration) -> Self {
        Self {
            interval,
            sites: Mutex::new(BTreeMap::new()),
        }
    }

    /// The re-report interval, for the report's own fields.
    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// Note one occurrence now. See [`Self::admit_at`].
    pub fn admit(&self, site: &'static str, class: &'static str) -> Option<u64> {
        self.admit_at(site, class, Instant::now())
    }

    /// Note one occurrence at `now`.
    ///
    /// `Some(suppressed)` means report it, naming how many occurrences were
    /// suppressed since the previous report. `None` means it is suppressed and
    /// counted towards the next report.
    pub fn admit_at(&self, site: &'static str, class: &'static str, now: Instant) -> Option<u64> {
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
/// [`crate::database_failure::log_database_failure`].
pub static REPEATED_FAILURES: ReportThrottle =
    ReportThrottle::new(REPEATED_FAILURE_REPORT_INTERVAL);

/// Log a failure that may repeat on every request or poll tick, bounded per
/// call site by [`REPEATED_FAILURE_REPORT_INTERVAL`].
///
/// The first argument is the level of an admitted report (`error` or `warn`);
/// the rest is what that `tracing` macro would take. An admitted report adds
/// `suppressed_since_last_report` and `report_interval_seconds`; a suppressed
/// occurrence is logged at `info` with `report_suppressed = true`.
///
/// ```ignore
/// everruns_core::report_repeated_failure!(error, error = %e, "Session schedule poll failed");
/// ```
#[macro_export]
macro_rules! report_repeated_failure {
    ($level:ident, $($arg:tt)+) => {{
        static THROTTLE: $crate::report_throttle::ReportThrottle =
            $crate::report_throttle::ReportThrottle::new(
                $crate::report_throttle::REPEATED_FAILURE_REPORT_INTERVAL,
            );
        match THROTTLE.admit("", "") {
            ::core::option::Option::Some(suppressed) => $crate::report_throttle::__tracing::$level!(
                suppressed_since_last_report = suppressed,
                report_interval_seconds = THROTTLE.interval().as_secs(),
                $($arg)+
            ),
            ::core::option::Option::None => $crate::report_throttle::__tracing::info!(
                report_suppressed = true,
                $($arg)+
            ),
        }
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    const INTERVAL: Duration = Duration::from_secs(600);

    #[test]
    fn a_sustained_outage_reports_once_per_interval_with_the_gap_counted() {
        let throttle = ReportThrottle::new(INTERVAL);
        let start = Instant::now();

        // One occurrence a second for an hour: what a per-request failure does.
        let reports: Vec<(u64, u64)> = (0..3600u64)
            .filter_map(|second| {
                throttle
                    .admit_at("valkey.rate_limit", "", start + Duration::from_secs(second))
                    .map(|suppressed| (second, suppressed))
            })
            .collect();

        // First occurrence immediately, then one per interval, each naming the
        // occurrences it stood in for.
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
    fn sites_and_classes_are_bounded_independently() {
        let throttle = ReportThrottle::new(INTERVAL);
        let now = Instant::now();

        assert_eq!(throttle.admit_at("a", "pool", now), Some(0));
        assert_eq!(throttle.admit_at("a", "pool", now), None);
        // A different site, or a different failure at the same site, is not
        // hidden behind the first.
        assert_eq!(throttle.admit_at("b", "pool", now), Some(0));
        assert_eq!(throttle.admit_at("a", "other", now), Some(0));
        assert_eq!(throttle.admit_at("a", "pool", now + INTERVAL), Some(1));
    }

    // The macro's levels are pinned in `everruns-server`, which can hold a
    // subscriber (EVE-876 keeps one out of this crate).
}
