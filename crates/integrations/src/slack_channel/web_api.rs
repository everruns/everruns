//! Compatibility error codes and a narrowly scoped Channelkit API escape hatch.
//! Typed messaging, rendering, history and media operations live in Channelkit.
//! Hosted callers retain installation identity, credential revision and egress.

/// Slack API error codes that retrying cannot fix.
///
/// The single source of truth for transient-vs-permanent (EVE-972), matched as
/// exact codes rather than substrings of a formatted message (EVE-968).
pub const PERMANENT_SLACK_ERRORS: &[&str] = &[
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
    // Slack refused the blocks themselves; the same payload fails again.
    // Typed Channelkit posts may use full plain text after a definite refusal.
    "invalid_blocks",
    "unknown_method",
    "channel_type_not_supported",
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
pub fn failure_log_level(error: &SlackApiError) -> tracing::Level {
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
pub const SLACK_ERROR_MESSAGE_PREFIX: &str = "Slack API error: ";

/// Ceiling on an honoured `Retry-After`.
///
/// Slack's advice is normally seconds, but a delivery task must not be pinned by
/// a pathological value. Worst case is `max_attempts` waits at this cap.
pub const MAX_RETRY_AFTER: std::time::Duration = std::time::Duration::from_secs(60);

/// A failed Slack API call, typed so the retry loop can act on the reason.
///
/// Before EVE-968 every failure was flattened into a string and re-examined by
/// substring match, which threw away the one thing a 429 actually tells us:
/// how long to wait.
#[derive(Debug)]
pub enum SlackApiError {
    /// Slack asked us to slow down, and said for how long when it could.
    RateLimited {
        retry_after: Option<std::time::Duration>,
    },
    /// Retrying cannot help: bad token, missing channel, empty message.
    Permanent(String),
    /// Network trouble, a 5xx, or an error code we do not recognise.
    Transient(String),
    /// Typed provider outcome; accepted or uncertain writes must never be retried blindly.
    Delivery(everruns_contracts::runtime::channel::DeliveryResult),
}

impl SlackApiError {
    /// Classify a Slack `error` code from an `ok: false` body.
    pub fn from_code(code: &str, retry_after: Option<std::time::Duration>) -> Self {
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
    pub fn is_permanent(&self) -> bool {
        matches!(
            self,
            Self::Permanent(_)
                | Self::Delivery(
                    everruns_contracts::runtime::channel::DeliveryResult::PermanentError(_)
                )
        )
    }

    /// The Slack `error` code this failure was built from, when it came from an
    /// `ok: false` body rather than from transport trouble.
    ///
    /// Lets a caller act on a specific code without re-deriving Slack's
    /// classification or substring-matching a rendered message.
    pub fn code(&self) -> Option<&str> {
        match self {
            Self::RateLimited { .. } => Some("ratelimited"),
            Self::Delivery(_) => None,
            Self::Permanent(message) | Self::Transient(message) => message
                .strip_prefix(SLACK_ERROR_MESSAGE_PREFIX)
                .or_else(|| message.strip_prefix("provider rejected operation: ")),
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
            Self::Delivery(outcome) => write!(f, "{outcome}"),
        }
    }
}

impl std::error::Error for SlackApiError {}

/// How long to wait before the next attempt.
///
/// Slack's own advice beats our guess. Retrying a 429 on a 1s backoff burns the
/// remaining attempts before the rate-limit window has even opened, which is how
/// a burst drops replies that would otherwise have gone through (EVE-968).
pub fn retry_wait(error: &SlackApiError, backoff: std::time::Duration) -> std::time::Duration {
    match error {
        SlackApiError::RateLimited {
            retry_after: Some(advice),
        } => (*advice).min(MAX_RETRY_AFTER),
        _ => backoff,
    }
}

/// Slack Web API base.
pub const SLACK_API_BASE: &str = "https://slack.com/api";

/// Call one Slack Web API method and return its body when `ok` is true.
///
/// A failure comes back classified: a rate limit with Slack's `Retry-After`
/// advice, a permanent code, or transport trouble.
///
/// Native messaging/rendering uses Channelkit typed methods; this escape hatch
/// retains provider error codes for narrowly projected action reads and explicit posts.
pub async fn slack_api_call(
    base_url: &str,
    bot_token: &str,
    method: &str,
    payload: serde_json::Value,
) -> Result<serde_json::Value, SlackApiError> {
    slack_api_call_scoped(base_url, &tool_context(bot_token, "", ""), method, payload).await
}

/// Trusted provider escape hatch using the same host scope as native actions.
pub async fn slack_api_call_scoped(
    base_url: &str,
    context: &everruns_contracts::runtime::channel::DeliveryContext,
    method: &str,
    payload: serde_json::Value,
) -> Result<serde_json::Value, SlackApiError> {
    let client = super::bridge::direct_factory()
        .map_err(provider_error)?
        .for_context(context, base_url)
        .await
        .map_err(provider_error)?;
    client
        .api_call(
            method,
            payload,
            !matches!(
                method,
                "auth.test"
                    | "users.info"
                    | "conversations.replies"
                    | "conversations.history"
                    | "files.info"
            ),
        )
        .await
        .map_err(provider_error)
}

/// Compatibility for explicit SDK tools that hold their own credential. Hosted
/// delivery instead supplies a row identity and scoped egress through its context.
pub async fn client_for_token(
    base: &str,
    token: &str,
) -> Result<std::sync::Arc<channelkit_slack::Slack>, SlackApiError> {
    let installation = tool_installation(token);
    super::bridge::direct_factory()
        .map_err(provider_error)?
        .for_credentials(&installation, None, token, base, None)
        .await
        .map_err(provider_error)
}

pub fn tool_installation(token: &str) -> String {
    use sha2::{Digest, Sha256};
    format!(
        "slack-tool-{}",
        hex::encode(Sha256::digest(token.as_bytes()))
    )
}

pub fn tool_context(
    token: &str,
    channel: &str,
    thread: &str,
) -> everruns_contracts::runtime::channel::DeliveryContext {
    let mut context =
        everruns_contracts::runtime::channel::DeliveryTarget::new(channel, thread).context(token);
    context.extra.insert(
        super::bridge::INSTALLATION_ID.into(),
        tool_installation(token),
    );
    context
}

pub fn provider_error(error: channelkit::Error) -> SlackApiError {
    match error {
        channelkit::Error::Rejected { ref code } => return SlackApiError::from_code(code, None),
        channelkit::Error::Observed(ref cause) => return provider_error_observed(cause),
        _ => {}
    }
    delivery_error(super::bridge::outcome(&error))
}

pub fn delivery_error(
    outcome: everruns_contracts::runtime::channel::DeliveryResult,
) -> SlackApiError {
    use everruns_contracts::runtime::channel::DeliveryResult;
    match outcome {
        DeliveryResult::RateLimited { retry_after } => SlackApiError::RateLimited {
            retry_after: Some(retry_after),
        },
        DeliveryResult::PermanentError(message) => SlackApiError::Permanent(message),
        DeliveryResult::TransientError(message) => SlackApiError::Transient(message),
        other => SlackApiError::Delivery(other),
    }
}

fn provider_error_observed(error: &channelkit::Error) -> SlackApiError {
    match error {
        channelkit::Error::Rejected { code } => SlackApiError::from_code(code, None),
        channelkit::Error::Observed(cause) => provider_error_observed(cause),
        _ => SlackApiError::Delivery(super::bridge::outcome(error)),
    }
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
