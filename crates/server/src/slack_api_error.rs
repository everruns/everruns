//! How a Slack Web API failure is classified, and what to wait before retrying.
//!
//! Split out of `slack_delivery` (EVE-1024): both the delivery adapter and the
//! native Slack capability's action path speak this envelope, so the one
//! classification they share should not live inside either caller — and
//! `slack_delivery` is on the file-size ratchet.

/// Slack API error codes that retrying cannot fix.
///
/// The single source of truth for transient-vs-permanent (EVE-972), matched as
/// exact codes rather than substrings of a formatted message (EVE-968).
pub(crate) const PERMANENT_SLACK_ERRORS: &[&str] = &[
    "channel_not_found",
    "token_expired",
    "not_authed",
    "invalid_auth",
    "token_revoked",
    "account_inactive",
    "no_text",
    // Bad action arguments and missing permissions need correction, not an
    // infrastructure retry. Keep their codes visible to the agent.
    "message_not_found",
    "bad_timestamp",
    "invalid_name",
    "invalid_arguments",
    "missing_scope",
    "no_permission",
    // `reactions.add` on an emoji the bot already placed. Retrying can never
    // succeed, and the caller reads it as the already-satisfied outcome it is
    // rather than a failure (EVE-1024).
    "already_reacted",
];

/// Slack error codes that describe the workspace's own install, not a fault
/// in this server: a scope the admin has not approved, a revoked token, a
/// channel the bot is not in. Callers already surface these where they can be
/// fixed (health issues, tool errors that say "reconnect"), so the shared
/// call path reports them as warnings rather than paging as errors
/// (EVERRUNS-2A).
const WORKSPACE_STATE_SLACK_ERRORS: &[&str] = &[
    "missing_scope",
    "no_permission",
    "not_in_channel",
    "channel_not_found",
    "is_archived",
    "not_authed",
    "invalid_auth",
    "token_expired",
    "token_revoked",
    "account_inactive",
];

/// Level at which a failed Slack call is logged where it happens.
///
/// `already_reacted` is the satisfied outcome its caller reads it as, and a
/// rate limit is routine backpressure. Anything else outside the workspace
/// state list may be a fault here, so it stays an error.
pub(crate) fn failure_log_level(error: &SlackApiError) -> tracing::Level {
    match error.code() {
        Some("ratelimited" | "already_reacted") => tracing::Level::DEBUG,
        Some(code) if WORKSPACE_STATE_SLACK_ERRORS.contains(&code) => tracing::Level::WARN,
        _ => tracing::Level::ERROR,
    }
}

/// Prefix `SlackApiError::from_code` renders a Slack `error` code behind.
///
/// Kept beside `from_code` and `code` so the one place that writes the wrapping
/// is the one place that reads it back.
pub(crate) const SLACK_ERROR_MESSAGE_PREFIX: &str = "Slack API error: ";

/// Ceiling on an honoured `Retry-After`.
///
/// Slack's advice is normally seconds, but a delivery task must not be pinned by
/// a pathological value. Worst case is `max_attempts` waits at this cap.
pub(crate) const MAX_RETRY_AFTER: std::time::Duration = std::time::Duration::from_secs(60);

/// A failed Slack API call, typed so the retry loop can act on the reason.
///
/// Before EVE-968 every failure was flattened into a string and re-examined by
/// substring match, which threw away the one thing a 429 actually tells us:
/// how long to wait.
#[derive(Debug)]
pub(crate) enum SlackApiError {
    /// Slack asked us to slow down, and said for how long when it could.
    RateLimited {
        retry_after: Option<std::time::Duration>,
    },
    /// Retrying cannot help: bad token, missing channel, empty message.
    Permanent(String),
    /// Network trouble, a 5xx, or an error code we do not recognise.
    Transient(String),
}

impl SlackApiError {
    /// Classify a Slack `error` code from an `ok: false` body.
    pub(crate) fn from_code(code: &str, retry_after: Option<std::time::Duration>) -> Self {
        if code == "ratelimited" {
            return Self::RateLimited { retry_after };
        }
        let message = format!("{SLACK_ERROR_MESSAGE_PREFIX}{code}");
        if PERMANENT_SLACK_ERRORS.contains(&code) {
            Self::Permanent(message)
        } else {
            Self::Transient(message)
        }
    }

    /// Whether retrying this failure is pointless.
    pub(crate) fn is_permanent(&self) -> bool {
        matches!(self, Self::Permanent(_))
    }

    /// The Slack `error` code this failure was built from, when it came from an
    /// `ok: false` body rather than from transport trouble.
    ///
    /// Lets a caller act on a specific code without re-deriving Slack's
    /// classification or substring-matching a rendered message.
    pub(crate) fn code(&self) -> Option<&str> {
        match self {
            Self::RateLimited { .. } => Some("ratelimited"),
            Self::Permanent(message) | Self::Transient(message) => {
                message.strip_prefix(SLACK_ERROR_MESSAGE_PREFIX)
            }
        }
    }
}

impl std::fmt::Display for SlackApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RateLimited {
                retry_after: Some(d),
            } => {
                write!(f, "Slack API rate limited (retry after {}s)", d.as_secs())
            }
            Self::RateLimited { retry_after: None } => write!(f, "Slack API rate limited"),
            Self::Permanent(message) | Self::Transient(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for SlackApiError {}

/// How long to wait before the next attempt.
///
/// Slack's own advice beats our guess. Retrying a 429 on a 1s backoff burns the
/// remaining attempts before the rate-limit window has even opened, which is how
/// a burst drops replies that would otherwise have gone through (EVE-968).
pub(crate) fn retry_wait(
    error: &SlackApiError,
    backoff: std::time::Duration,
) -> std::time::Duration {
    match error {
        SlackApiError::RateLimited {
            retry_after: Some(advice),
        } => (*advice).min(MAX_RETRY_AFTER),
        _ => backoff,
    }
}

/// Slack sends `Retry-After` in whole seconds.
pub(crate) fn parse_retry_after(
    headers: &reqwest::header::HeaderMap,
) -> Option<std::time::Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(std::time::Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn actionable_slack_failures_are_not_internal_errors() {
        for code in [
            "message_not_found",
            "bad_timestamp",
            "invalid_name",
            "missing_scope",
            "no_permission",
            "invalid_arguments",
        ] {
            let error = SlackApiError::from_code(code, None);
            assert!(
                error.is_permanent(),
                "{code} requires corrected arguments or configuration"
            );
            assert_eq!(error.code(), Some(code));
        }
        assert!(!SlackApiError::from_code("internal_error", None).is_permanent());
    }

    #[test]
    fn workspace_state_failures_are_warnings_not_errors() {
        use tracing::Level;
        let level = |code: &str| failure_log_level(&SlackApiError::from_code(code, None));

        // EVERRUNS-2A: a workspace that has not approved `reactions:write`.
        assert_eq!(level("missing_scope"), Level::WARN);
        assert_eq!(level("not_in_channel"), Level::WARN);
        assert_eq!(level("token_revoked"), Level::WARN);
        assert_eq!(level("already_reacted"), Level::DEBUG);
        assert_eq!(level("ratelimited"), Level::DEBUG);
        // Codes that can mean this server sent something wrong stay errors.
        assert_eq!(level("invalid_arguments"), Level::ERROR);
        assert_eq!(level("no_text"), Level::ERROR);
        assert_eq!(level("internal_error"), Level::ERROR);
        assert_eq!(
            failure_log_level(&SlackApiError::Transient("connection reset".into())),
            Level::ERROR
        );
    }

    #[test]
    fn advice_wins_over_backoff() {
        let one_second = Duration::from_secs(1);
        assert_eq!(
            retry_wait(
                &SlackApiError::RateLimited {
                    retry_after: Some(Duration::from_secs(30))
                },
                one_second
            ),
            Duration::from_secs(30),
            "a 429 saying 30s must not be retried after 1s"
        );
    }

    #[test]
    fn backoff_applies_without_advice() {
        let backoff = Duration::from_secs(4);
        for error in [
            SlackApiError::RateLimited { retry_after: None },
            SlackApiError::Transient("boom".to_string()),
        ] {
            assert_eq!(retry_wait(&error, backoff), backoff, "{error:?}");
        }
    }

    #[test]
    fn pathological_advice_is_capped() {
        assert_eq!(
            retry_wait(
                &SlackApiError::RateLimited {
                    retry_after: Some(Duration::from_secs(86_400))
                },
                Duration::from_secs(1)
            ),
            MAX_RETRY_AFTER,
            "a delivery task must not be pinned for a day"
        );
    }
}
