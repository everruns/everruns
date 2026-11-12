//! The Slack Web API envelope: one call path, typed failures, retry advice.
//!
//! Every Web API method answers the same way: `ok: false` with an `error`
//! code, and a `Retry-After` header on a rate limit. Deciding what each code
//! means lives here once (EVE-974), shared by the serve Slack driver and the
//! server's Slack delivery, actions and interactivity.
//!
//! Decisions:
//! - One `reqwest::Client` per process, so calls reuse connections.
//! - Codes are matched exactly, never by substring of a rendered message
//!   (EVE-968).

use std::sync::OnceLock;

use tracing::{Level, debug, error, warn};

fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

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
    // `slack_api_call` retries a Markdown post once as plain text instead.
    "invalid_blocks",
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
        matches!(self, Self::Permanent(_))
    }

    /// The Slack `error` code this failure was built from, when it came from an
    /// `ok: false` body rather than from transport trouble.
    ///
    /// Lets a caller act on a specific code without re-deriving Slack's
    /// classification or substring-matching a rendered message.
    pub fn code(&self) -> Option<&str> {
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
pub fn retry_wait(error: &SlackApiError, backoff: std::time::Duration) -> std::time::Duration {
    match error {
        SlackApiError::RateLimited {
            retry_after: Some(advice),
        } => (*advice).min(MAX_RETRY_AFTER),
        _ => backoff,
    }
}

/// Slack sends `Retry-After` in whole seconds.
pub fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<std::time::Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(std::time::Duration::from_secs)
}

/// Slack Web API base.
pub const SLACK_API_BASE: &str = "https://slack.com/api";

/// Post one message and return its `ts`.
///
/// The status summary needs the `ts` back so it can update that message rather
/// than posting a second one; the delivery path's own post loop discards it
/// because it may split a reply across several calls.
pub async fn post_slack_message_returning_ts(
    bot_token: &str,
    channel: &str,
    thread_ts: &str,
    text: &str,
) -> Result<String, SlackApiError> {
    let text = super::markdown::neutralize_broadcasts(text);
    let mut payload = serde_json::json!({
        "channel": channel,
        "text": text,
        "blocks": [super::markdown::markdown_block(&text)],
    });
    if !thread_ts.is_empty() {
        payload["thread_ts"] = serde_json::json!(thread_ts);
    }
    let body = slack_api_call(SLACK_API_BASE, bot_token, "chat.postMessage", payload).await?;
    body.get("ts")
        .and_then(|ts| ts.as_str())
        .map(str::to_string)
        .ok_or_else(|| SlackApiError::Transient("chat.postMessage returned no ts".to_string()))
}

/// Rewrite a message's text in place.
pub async fn update_slack_message_text(
    bot_token: &str,
    channel: &str,
    ts: &str,
    text: &str,
) -> Result<(), SlackApiError> {
    let text = super::markdown::neutralize_broadcasts(text);
    slack_api_call(
        SLACK_API_BASE,
        bot_token,
        "chat.update",
        serde_json::json!({
            "channel": channel,
            "ts": ts,
            "text": text,
            "blocks": [super::markdown::markdown_block(&text)],
        }),
    )
    .await?;
    Ok(())
}

/// Call one Slack Web API method and return its body when `ok` is true.
///
/// A failure comes back classified: a rate limit with Slack's `Retry-After`
/// advice, a permanent code, or transport trouble.
///
/// A Markdown post Slack rejects as `invalid_blocks` (one bad token, such as
/// an unknown user-group mention, fails the whole block) is sent once more as
/// plain `text`, so the person still gets the reply. Hand-built Block Kit is
/// never retried this way; see [`super::markdown::plain_text_retry`].
pub async fn slack_api_call(
    base_url: &str,
    bot_token: &str,
    method: &str,
    payload: serde_json::Value,
) -> Result<serde_json::Value, SlackApiError> {
    match slack_api_call_once(base_url, bot_token, method, &payload).await {
        Err(error) if error.code() == Some("invalid_blocks") => {
            let Some(retry) = super::markdown::plain_text_retry(method, &payload) else {
                return Err(error);
            };
            warn!(
                method = method,
                "Slack rejected the Markdown blocks; resending as plain text"
            );
            slack_api_call_once(base_url, bot_token, method, &retry).await
        }
        result => result,
    }
}

async fn slack_api_call_once(
    base_url: &str,
    bot_token: &str,
    method: &str,
    payload: &serde_json::Value,
) -> Result<serde_json::Value, SlackApiError> {
    let response = client()
        .post(format!("{}/{}", base_url, method))
        .header("Authorization", format!("Bearer {}", bot_token))
        .header("Content-Type", "application/json")
        .json(payload)
        .send()
        .await
        .map_err(|e| SlackApiError::Transient(e.to_string()))?;

    let status = response.status();
    // Read the header before the body is consumed: a 429 carries its advice here,
    // not in the JSON.
    let retry_after = parse_retry_after(response.headers());

    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| SlackApiError::Transient(e.to_string()))?;

    if !body.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        let error = body
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("unknown");

        let failure = SlackApiError::from_code(error, retry_after);

        // Rate limits and `already_reacted` are routine, and workspace install
        // state is the admin's to fix, so only the rest pages as an error.
        match failure_log_level(&failure) {
            Level::DEBUG => debug!(
                method = method,
                error = error,
                retry_after_secs = ?retry_after.map(|d| d.as_secs()),
                "Slack API call declined"
            ),
            Level::WARN => warn!(
                method = method,
                error = error,
                status = %status,
                "Slack API call failed"
            ),
            _ => error!(
                method = method,
                error = error,
                status = %status,
                "Slack API call failed"
            ),
        }
        return Err(failure);
    }

    Ok(body)
}

/// Post a Block Kit message into a thread.
///
/// `text` is the notification/fallback string: a blocks-only post reaches push
/// notifications and screen readers as an empty message.
pub async fn post_slack_blocks(
    bot_token: &str,
    channel: &str,
    thread_ts: &str,
    text: &str,
    blocks: &serde_json::Value,
) -> Result<(), SlackApiError> {
    let mut payload = serde_json::json!({
        "channel": channel,
        "text": text,
        "blocks": blocks,
    });
    if !thread_ts.is_empty() {
        payload["thread_ts"] = serde_json::json!(thread_ts);
    }
    slack_api_call(SLACK_API_BASE, bot_token, "chat.postMessage", payload).await?;
    Ok(())
}

/// Deliver an approval only to its requester. Never fall back to chat.postMessage.
pub async fn post_slack_ephemeral_blocks(
    bot_token: &str,
    channel: &str,
    thread_ts: &str,
    user: &str,
    text: &str,
    blocks: &serde_json::Value,
) -> Result<(), SlackApiError> {
    let payload = ephemeral_blocks_payload(channel, thread_ts, user, text, blocks)?;
    post_ephemeral_payload(SLACK_API_BASE, bot_token, payload).await
}

async fn post_ephemeral_payload(
    base: &str,
    token: &str,
    payload: serde_json::Value,
) -> Result<(), SlackApiError> {
    slack_api_call(base, token, "chat.postEphemeral", payload).await?;
    Ok(())
}

fn ephemeral_blocks_payload(
    channel: &str,
    thread_ts: &str,
    user: &str,
    text: &str,
    blocks: &serde_json::Value,
) -> Result<serde_json::Value, SlackApiError> {
    if user.is_empty() {
        return Err(SlackApiError::Permanent(
            "Private message requires a recipient".into(),
        ));
    }
    let mut payload =
        serde_json::json!({"channel": channel, "user": user, "text": text, "blocks": blocks});
    if !thread_ts.is_empty() {
        payload["thread_ts"] = serde_json::json!(thread_ts);
    }
    Ok(payload)
}

/// Rewrite a message's blocks, e.g. to retire an answered approval card.
///
/// `text` is the notification/fallback string; `blocks` is what the thread
/// renders. Both are required — a `chat.update` that sends blocks without text
/// leaves push notifications and accessibility clients with nothing to read.
pub async fn update_slack_message_blocks(
    bot_token: &str,
    channel: &str,
    ts: &str,
    text: &str,
    blocks: &serde_json::Value,
) -> Result<(), SlackApiError> {
    slack_api_call(
        SLACK_API_BASE,
        bot_token,
        "chat.update",
        serde_json::json!({
            "channel": channel,
            "ts": ts,
            "text": text,
            "blocks": blocks,
        }),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn private_delivery_does_not_fall_back_on_slack_failure() {
        use wiremock::matchers::{body_partial_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat.postEphemeral"))
            .and(body_partial_json(
                serde_json::json!({"user": "U1", "channel": "C1", "thread_ts": "1.0"}),
            ))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({"ok": false, "error": "user_not_in_channel"}),
                ),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path("/chat.postMessage"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        let payload =
            ephemeral_blocks_payload("C1", "1.0", "U1", "private", &serde_json::json!([])).unwrap();
        assert!(
            post_ephemeral_payload(&server.uri(), "test-token", payload)
                .await
                .is_err()
        );
    }

    #[test]
    fn private_message_payload_is_recipient_and_thread_bound() {
        let blocks = serde_json::json!([]);
        let payload = ephemeral_blocks_payload("C1", "1.0", "U1", "private", &blocks).unwrap();
        assert_eq!(payload["user"], "U1");
        assert_eq!(payload["channel"], "C1");
        assert_eq!(payload["thread_ts"], "1.0");
        assert_eq!(payload["text"], "private");
        assert!(ephemeral_blocks_payload("C1", "", "", "private", &blocks).is_err());
        assert!(
            ephemeral_blocks_payload("C1", "", "U1", "private", &blocks)
                .unwrap()
                .get("thread_ts")
                .is_none()
        );
    }

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
