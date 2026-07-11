//! Structured slow-query warnings for hot storage reads (EVE-730).
//!
//! When a bounded-but-still-large storage read crosses a latency threshold we
//! emit exactly one structured `tracing::warn!` that a SaaS host can map to an
//! alerting backend (e.g. Sentry). The warning carries only operational
//! metadata: a stable query-family identifier, elapsed time, row count, the
//! caller-supplied row limit, and host-supplied environment/release context.
//!
//! # Redaction contract
//!
//! The public API accepts only scalar operational values
//! (`operation: &'static str`, `elapsed`, `row_count`, `limit`). It is
//! structurally impossible to pass SQL parameters (e.g. `session_id`), event
//! `data`, message content, or any other PII through this path, so none can
//! leak into logs. Do not extend [`SlowQueryWarning`] with row/content fields.

use std::sync::OnceLock;
use std::time::Duration;

/// Default slow-query warning threshold in milliseconds.
///
/// Chosen well below the SQLx built-in 1s slow-statement log so a large
/// message-history read is flagged as a domain event before it becomes a
/// user-visible stall. Overridable via `SLOW_QUERY_WARN_MS`.
pub const DEFAULT_SLOW_QUERY_WARN_MS: u64 = 500;

/// Stable query-family identifier for the per-turn message-history read.
///
/// Kept stable across refactors so host alerting rules can group on it.
pub const OP_MESSAGE_HISTORY_READ: &str = "message_history_read";

/// Host-supplied environment/release context attached to slow-query warnings.
///
/// Resolved once from the same environment variables the telemetry layer uses
/// (`OTEL_ENVIRONMENT`, `OTEL_SERVICE_VERSION`) so the deployment/release shown
/// in a slow-query alert matches the deployment/release shown in traces. The
/// release falls back to the compiled crate version when the host does not set
/// `OTEL_SERVICE_VERSION`.
#[derive(Debug, Clone)]
pub struct HostContext {
    pub environment: Option<String>,
    pub release: Option<String>,
}

impl HostContext {
    fn from_env() -> Self {
        let environment = std::env::var("OTEL_ENVIRONMENT")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let release = std::env::var("OTEL_SERVICE_VERSION")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| Some(env!("CARGO_PKG_VERSION").to_string()));
        Self {
            environment,
            release,
        }
    }
}

static HOST_CONTEXT: OnceLock<HostContext> = OnceLock::new();

/// Process-wide host context, resolved once from the environment.
pub fn host_context() -> &'static HostContext {
    HOST_CONTEXT.get_or_init(HostContext::from_env)
}

static THRESHOLD_MS: OnceLock<u64> = OnceLock::new();

/// Configured slow-query threshold, resolved once from `SLOW_QUERY_WARN_MS`.
pub fn threshold() -> Duration {
    let ms = *THRESHOLD_MS.get_or_init(|| {
        std::env::var("SLOW_QUERY_WARN_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(DEFAULT_SLOW_QUERY_WARN_MS)
    });
    Duration::from_millis(ms)
}

/// A structured slow-query warning. Fields are operational only — see the
/// module-level redaction contract.
#[derive(Debug, Clone)]
pub struct SlowQueryWarning {
    /// Stable query-family / operation identifier.
    pub operation: &'static str,
    /// Wall-clock time the query took.
    pub elapsed_ms: u64,
    /// Number of rows the query returned.
    pub row_count: u64,
    /// Caller-supplied row limit, when one was provided.
    pub limit: Option<i64>,
}

impl SlowQueryWarning {
    pub fn new(
        operation: &'static str,
        elapsed: Duration,
        row_count: usize,
        limit: Option<i64>,
    ) -> Self {
        Self {
            operation,
            elapsed_ms: elapsed.as_millis().min(u128::from(u64::MAX)) as u64,
            row_count: row_count as u64,
            limit,
        }
    }

    /// Emit exactly one structured warning with the operational fields plus
    /// host-supplied environment/release context.
    pub fn emit(&self) {
        let host = host_context();
        tracing::warn!(
            operation = self.operation,
            query_family = self.operation,
            elapsed_ms = self.elapsed_ms,
            row_count = self.row_count,
            limit = self.limit,
            environment = host.environment.as_deref(),
            release = host.release.as_deref(),
            "slow storage query crossed warning threshold"
        );
    }
}

/// Emit a slow-query warning when `elapsed` reaches the configured threshold.
///
/// Returns whether a warning was emitted (useful for tests / metrics).
pub fn maybe_warn(
    operation: &'static str,
    elapsed: Duration,
    row_count: usize,
    limit: Option<i64>,
) -> bool {
    if elapsed < threshold() {
        return false;
    }
    SlowQueryWarning::new(operation, elapsed, row_count, limit).emit();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warning_carries_only_operational_fields() {
        let warning = SlowQueryWarning::new(
            OP_MESSAGE_HISTORY_READ,
            Duration::from_millis(1_120),
            3_031,
            Some(5_000),
        );

        assert_eq!(warning.operation, "message_history_read");
        assert_eq!(warning.elapsed_ms, 1_120);
        assert_eq!(warning.row_count, 3_031);
        assert_eq!(warning.limit, Some(5_000));

        // Redaction: the rendered warning must expose only the operational and
        // host-context fields — never SQL params, event content, or PII. A
        // debug render is a proxy for "everything this type can surface". The
        // markers below cannot collide with the stable operation identifier.
        let rendered = format!("{warning:?}");
        for banned in [
            "session_id",
            "tool_call",
            "arguments",
            "content",
            "SELECT",
            "WHERE",
            "$1",
            "$2",
        ] {
            assert!(
                !rendered.contains(banned),
                "slow-query warning leaked `{banned}`: {rendered}"
            );
        }
    }

    #[test]
    fn host_context_reports_a_release() {
        // Release always resolves — either the host-supplied OTEL_SERVICE_VERSION
        // or the compiled crate version fallback — so alerts are never
        // release-less.
        let host = HostContext::from_env();
        assert!(host.release.is_some());
    }

    #[test]
    fn elapsed_below_threshold_does_not_warn() {
        // A fast query must not emit. Threshold default is 500ms.
        assert!(!maybe_warn(
            OP_MESSAGE_HISTORY_READ,
            Duration::from_millis(1),
            10,
            Some(200),
        ));
    }

    #[test]
    fn elapsed_at_or_above_threshold_warns() {
        // Force a low threshold for the assertion to be independent of env.
        let elapsed = threshold();
        assert!(maybe_warn(OP_MESSAGE_HISTORY_READ, elapsed, 3_031, None,));
    }
}
