//! What one Braintrust delivery attempt produced, and how a permanent
//! rejection is reported.
//!
// Split out of `braintrust.rs`, which is on the source-size ratchet's debt
// list and may not grow. The concern is self-contained — one attempt's outcome
// plus the small state machine deciding whether a repeating rejection is worth
// another error-level event — so it tests without a listener, a mock server or
// a runtime clock.

use std::sync::Mutex;
use tokio::time::{self, Duration};
use tracing::{debug, error};

/// What one delivery attempt produced.
pub(super) enum DeliveryAttempt {
    Success,
    Retryable(String),
    /// `class` is the part worth comparing across batches — an HTTP status, or
    /// that the transport itself failed. `reason` adds the response body, which
    /// differs between otherwise identical rejections and so cannot key
    /// suppression.
    Permanent {
        class: String,
        reason: String,
    },
}

/// How long an unchanged permanent rejection stays suppressed before it is
/// reported at error level again.
///
/// Long enough that one misconfiguration does not flood alerting, short enough
/// that a container living for days still says so more than once.
const PERMANENT_FAILURE_REPORT_INTERVAL: Duration = Duration::from_secs(900);

/// Error-level reporting state for permanent rejections.
///
/// A permanent rejection (auth, unknown project) repeats on every batch until
/// the deployment is reconfigured, so reporting every one floods error alerting
/// (Sentry EVERRUNS-1K). Reporting exactly once per process was the
/// over-correction: production carried the wrong Braintrust project for a week
/// (EVE-956) because the signature was one burst per container start and then
/// silence — indistinguishable from having nothing to export.
///
/// So suppression is time-bounded rather than permanent, and keyed by failure
/// class: a revoked key is never hidden behind an earlier wrong-project 403.
#[derive(Debug, Default)]
pub(super) struct PermanentFailureReporting {
    /// Failure class last reported at error level, e.g. `HTTP 403`.
    reported_class: Option<String>,
    /// When that report happened, for the re-report interval.
    reported_at: Option<time::Instant>,
    /// Rejections suppressed since then. Named in the next report so the gap is
    /// stated rather than inferred from its absence.
    suppressed: u64,
}

impl PermanentFailureReporting {
    /// Decide whether this permanent rejection is reported at error level.
    ///
    /// `Some(suppressed)` means report it, naming how many rejections were
    /// suppressed since the last error-level report. `None` means suppress and
    /// count it.
    ///
    /// Reports when nothing has been reported yet, when the failure class
    /// differs from the one last reported, or when the re-report interval has
    /// elapsed. The class check keeps a second, different fault from hiding
    /// behind the first; the interval keeps a standing misconfiguration from
    /// going quiet (EVE-956).
    pub(super) fn note(&mut self, class: &str, now: time::Instant) -> Option<u64> {
        let due = match (self.reported_class.as_deref(), self.reported_at) {
            (Some(reported_class), Some(reported_at)) => {
                reported_class != class
                    || now.saturating_duration_since(reported_at)
                        >= PERMANENT_FAILURE_REPORT_INTERVAL
            }
            // Nothing reported yet, so there is nothing to suppress behind.
            _ => true,
        };

        if due {
            let suppressed = self.suppressed;
            self.reported_class = Some(class.to_string());
            self.reported_at = Some(now);
            self.suppressed = 0;
            Some(suppressed)
        } else {
            self.suppressed += 1;
            None
        }
    }
}

/// Log a permanent rejection at error level, or suppress and count it.
pub(super) fn report_permanent_failure(
    reporting: &Mutex<PermanentFailureReporting>,
    class: &str,
    reason: &str,
) {
    // A poisoned lock must not silence reporting: this mutex guards three plain
    // fields with no invariant a panic could have broken.
    let decision = reporting
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .note(class, time::Instant::now());

    match decision {
        Some(suppressed) => error!(
            reason = %reason,
            failure_class = %class,
            suppressed_since_last_report = suppressed,
            report_interval_seconds = PERMANENT_FAILURE_REPORT_INTERVAL.as_secs(),
            "Braintrust rejected batch; export stays off until the deployment is reconfigured"
        ),
        None => debug!(
            reason = %reason,
            failure_class = %class,
            "Braintrust batch rejected again"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A standing misconfiguration must not go quiet.
    ///
    /// EVE-956: production carried the wrong Braintrust project for a week. The
    /// exporter reported the 403 once per container start and then never again,
    /// so "export is broken" looked exactly like "nothing to export".
    /// Suppression is time-bounded, so the next report after the interval
    /// carries the count of everything suppressed in between.
    #[tokio::test]
    async fn a_standing_rejection_is_reported_again_after_the_interval() {
        let mut reporting = PermanentFailureReporting::default();
        let start = time::Instant::now();

        assert_eq!(
            reporting.note("HTTP 403", start),
            Some(0),
            "the first rejection is always reported"
        );
        assert_eq!(
            reporting.note("HTTP 403", start + Duration::from_secs(60)),
            None,
            "still inside the interval: suppressed"
        );
        assert_eq!(
            reporting.note("HTTP 403", start + Duration::from_secs(300)),
            None,
            "still inside the interval: suppressed"
        );
        assert_eq!(
            reporting.note("HTTP 403", start + Duration::from_secs(1000)),
            Some(2),
            "past the interval it reports again, naming the two it swallowed"
        );
        assert_eq!(
            reporting.note("HTTP 403", start + Duration::from_secs(1001)),
            None,
            "and the interval restarts from the report, not from the first failure"
        );
    }

    /// A second, different fault must not hide behind the first.
    ///
    /// The one-shot latch this replaces was global across reasons: a
    /// wrong-project 403 silenced error-level reporting for the whole process,
    /// so a later revoked key (401) or deleted project (404) never surfaced at
    /// all. One misconfiguration masking every other is worse than either alone.
    #[tokio::test]
    async fn a_different_rejection_is_not_masked_by_an_earlier_one() {
        let mut reporting = PermanentFailureReporting::default();
        let start = time::Instant::now();

        assert_eq!(reporting.note("HTTP 403", start), Some(0));
        assert_eq!(
            reporting.note("HTTP 401", start + Duration::from_secs(1)),
            Some(0),
            "a changed failure class is reported immediately, interval or not"
        );
        assert_eq!(
            reporting.note("HTTP 401", start + Duration::from_secs(2)),
            None,
            "and then it is the one being suppressed"
        );
        assert_eq!(
            reporting.note("HTTP 403", start + Duration::from_secs(3)),
            Some(1),
            "switching back reports too, carrying the 401 it suppressed"
        );
    }
}
