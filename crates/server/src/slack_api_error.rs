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
    "not_authed",
    "invalid_auth",
    "token_revoked",
    "account_inactive",
    "no_text",
    // `reactions.add` on an emoji the bot already placed. Retrying can never
    // succeed, and the caller reads it as the already-satisfied outcome it is
    // rather than a failure (EVE-1024).
    "already_reacted",
];

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
