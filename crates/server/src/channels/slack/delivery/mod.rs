// Slack delivery: how one Slack message, stream, status line or title is
// posted. Which events become which posts is the shared channel runtime's
// `TurnDelivery`, run by the channel host (`channels::slack::host`), which also
// finds the events and resumes a turn's delivery after a restart.
//
// Design Decision: No durable queue for the Slack HTTP call. Events are
// durably stored and the host's pending deliveries resume after a restart.
// Retry on transient Slack API failures uses simple exponential backoff
// in-process.

use async_trait::async_trait;
use everruns_core::channel::{
    ChannelAgentSurface, ChannelDeliveryAdapter, ChannelStreamDelivery,
    DeliveryContext as ChannelDeliveryContext, DeliveryResult as ChannelDeliveryResult,
    OutboundChannelMessage,
};
pub(crate) mod turn_adapter;
use crate::domains::agent_channels::record::exposure;
use everruns_core::channel_runtime::DeliveryOptions;
use everruns_integrations::slack_channel::web_api::{SLACK_API_BASE, slack_api_call};
use everruns_integrations::slack_channel::web_api::{SlackApiError, parse_retry_after, retry_wait};
use exposure::{PublicToolVisibility, public_tool_activity_text};
use tracing::{debug, error, info, warn};

/// Which Slack surface a turn belongs to.
///
/// One app serves both at once: enabling Slack's Agents feature adds an assistant
/// container, it does not replace the channel bot. Which surface an event belongs
/// to is therefore read from the event at runtime, not from config (EVE-973).
/// Delivery style branches on this in the tickets that follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SlackSurface {
    /// An ordinary channel or group thread — the classic channel bot.
    #[default]
    Channel,
    /// The agent pane: a DM to an app with the agent surface enabled.
    Pane,
}

/// Pane or channel, from the only two signals Slack gives us.
///
/// A live event carries `channel_type` (`"im"` for the agent pane). Recovery has
/// no event to read — only the channel id stored in the turn's metadata — and
/// Slack DM channel ids start with `D`. Both paths go through here so the two
/// cannot drift into disagreeing about the same session.
///
/// With the surface disabled the app is a channel bot everywhere, DMs included,
/// which is exactly how it behaves today.
pub fn classify_surface(
    agent_surface_enabled: bool,
    channel_type: Option<&str>,
    channel_id: &str,
) -> SlackSurface {
    if !agent_surface_enabled {
        return SlackSurface::Channel;
    }

    let is_dm = match channel_type {
        Some(kind) => kind == "im",
        None => channel_id.starts_with('D'),
    };

    if is_dm {
        SlackSurface::Pane
    } else {
        SlackSurface::Channel
    }
}

/// Turn-level status: the agent is working and no tool is running.
///
/// Reveals that the turn is alive and nothing else, which is the point — most of
/// the "is it dead?" ambiguity goes away without narrating anything (EVE-975).
const SLACK_THINKING_STATUS: &str = "is thinking...";

/// How a Slack turn's replies look. Streaming, status and title are pane
/// behaviours: token-by-token into a shared channel is not wanted, and a
/// channel thread has no status line or title to set (EVE-973/974/975).
fn turn_options(
    surface: SlackSurface,
    tool_visibility: PublicToolVisibility,
    generic_tool_text: &str,
    session_link: Option<String>,
) -> DeliveryOptions {
    let pane = surface == SlackSurface::Pane;
    DeliveryOptions {
        stream: pane,
        agent_surface: pane,
        thinking_status: SLACK_THINKING_STATUS.to_string(),
        tool_status: public_tool_activity_text(tool_visibility, generic_tool_text)
            .map(str::to_string),
        session_link: session_link.map(|link| format!("<{link}|View the session>")),
        replies: true,
    }
}

/// [`turn_options`] for a channel's configuration.
pub(crate) fn slack_turn_options(
    surface: SlackSurface,
    config: &crate::domains::agent_channels::record::slack_channel::SlackChannelConfig,
    session_link: Option<String>,
) -> DeliveryOptions {
    turn_options(
        surface,
        config.tool_visibility,
        &config.generic_tool_text,
        session_link,
    )
}

// ============================================
// ChannelDeliveryAdapter implementation for Slack
// ============================================

/// Slack implementation of the generic `ChannelDeliveryAdapter` trait.
///
/// Translates `OutboundChannelMessage` into Slack `chat.postMessage` API calls.
/// The channel host delivers every Slack turn through it.
pub struct SlackDeliveryAdapter {
    /// Slack API base. Tests point this at a mock server.
    api_base: String,
}

impl SlackDeliveryAdapter {
    pub fn new() -> Self {
        Self {
            api_base: SLACK_API_BASE.to_string(),
        }
    }

    /// Adapter aimed at a different Slack API base — used by tests.
    pub fn with_api_base(api_base: String) -> Self {
        Self { api_base }
    }
}

impl Default for SlackDeliveryAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ChannelDeliveryAdapter for SlackDeliveryAdapter {
    fn platform(&self) -> &str {
        "slack"
    }

    async fn deliver(
        &self,
        message: &OutboundChannelMessage,
        context: &ChannelDeliveryContext,
    ) -> ChannelDeliveryResult {
        // Correlation comes from the message rather than the context: the
        // context is per-registration, the ids identify this one reply.
        let correlation =
            message
                .correlation_id
                .as_ref()
                .map(|input_message_id| SlackCorrelation {
                    session_id: message.session_id.to_string(),
                    input_message_id: input_message_id.clone(),
                });

        match post_to_slack_with_retry_base(
            &self.api_base,
            &context.auth_token,
            &context.channel_id,
            &context.thread_ref,
            &message.text,
            correlation.as_ref(),
        )
        .await
        {
            Ok(()) => ChannelDeliveryResult::Ok,
            Err(e) => classify_slack_failure(e),
        }
    }

    async fn send_ack(
        &self,
        thread_ref: &str,
        text: &str,
        context: &ChannelDeliveryContext,
    ) -> ChannelDeliveryResult {
        match post_to_slack_base(
            &self.api_base,
            &context.auth_token,
            &context.channel_id,
            thread_ref,
            text,
        )
        .await
        {
            Ok(()) => ChannelDeliveryResult::Ok,
            Err(e) => classify_slack_failure(e),
        }
    }

    fn streaming(&self) -> Option<&dyn ChannelStreamDelivery> {
        Some(self)
    }

    fn agent_surface(&self) -> Option<&dyn ChannelAgentSurface> {
        Some(self)
    }
}

/// Keys the Slack adapter reads out of `DeliveryContext::extra`.
///
/// `chat.startStream` refuses a channel stream without both and answers
/// `missing_recipient_team_id`. Neither is listed as required in Slack's
/// argument table; this was found by calling it (EVE-974).
pub(crate) const SLACK_RECIPIENT_USER_ID: &str = "slack_recipient_user_id";
pub(crate) const SLACK_RECIPIENT_TEAM_ID: &str = "slack_recipient_team_id";

mod streams;
use streams::classify_slack_failure;

/// Map a Slack transport failure onto a `ChannelDeliveryResult`.
/// Slack's agent-surface methods, in one place.
///
/// Slack documents `assistant.threads.setStatus` / `setTitle` as compatibility
/// bridges over these, so both name sets work today and one of them will not.
/// Pinning the `agents.sessions.*` set here — and nowhere else — makes a move a
/// two-line change rather than a search (EVE-975).
const SLACK_SET_STATUS_METHOD: &str = "agents.sessions.setStatus";
const SLACK_SET_TITLE_METHOD: &str = "agents.sessions.rename";

/// Slack rejects a title longer than this.
const SLACK_TITLE_MAX_CHARS: usize = 250;

#[async_trait]
impl ChannelAgentSurface for SlackDeliveryAdapter {
    async fn set_status(
        &self,
        status: &str,
        context: &ChannelDeliveryContext,
    ) -> ChannelDeliveryResult {
        // The pane keys status off the thread, so a thread_ts is required; a
        // channel post has none and has no status line to set either.
        if context.thread_ref.is_empty() {
            return ChannelDeliveryResult::Ok;
        }
        let payload = serde_json::json!({
            "channel_id": context.channel_id,
            "thread_ts": context.thread_ref,
            "status": status,
        });
        match slack_api_call(
            &self.api_base,
            &context.auth_token,
            SLACK_SET_STATUS_METHOD,
            payload,
        )
        .await
        {
            Ok(_) => ChannelDeliveryResult::Ok,
            Err(e) => classify_slack_failure(e),
        }
    }

    async fn set_title(
        &self,
        title: &str,
        context: &ChannelDeliveryContext,
    ) -> ChannelDeliveryResult {
        if context.thread_ref.is_empty() {
            return ChannelDeliveryResult::Ok;
        }
        let payload = serde_json::json!({
            "channel_id": context.channel_id,
            "thread_ts": context.thread_ref,
            "title": truncate_chars(title, SLACK_TITLE_MAX_CHARS),
        });
        match slack_api_call(
            &self.api_base,
            &context.auth_token,
            SLACK_SET_TITLE_METHOD,
            payload,
        )
        .await
        {
            Ok(_) => ChannelDeliveryResult::Ok,
            Err(e) => classify_slack_failure(e),
        }
    }
}

async fn post_to_slack_with_retry_base(
    base_url: &str,
    bot_token: &str,
    channel: &str,
    thread_ts: &str,
    text: &str,
    correlation: Option<&SlackCorrelation>,
) -> Result<(), SlackApiError> {
    let max_attempts = 3;
    let mut backoff = std::time::Duration::from_secs(1);

    for attempt in 1..=max_attempts {
        match post_slack_message(base_url, bot_token, channel, thread_ts, text, correlation).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                if e.is_permanent() || attempt == max_attempts {
                    return Err(e);
                }

                let wait = retry_wait(&e, backoff);

                warn!(
                    attempt,
                    max_attempts,
                    wait_secs = wait.as_secs(),
                    error = %e,
                    "Slack post failed, retrying"
                );
                tokio::time::sleep(wait).await;
                backoff *= 2;
            }
        }
    }

    unreachable!()
}

mod markdown_split;
use markdown_split::{
    SLACK_MARKDOWN_BLOCK_LIMIT, SLACK_MAX_BLOCKS_PER_MESSAGE, SLACK_MAX_OUTBOUND_REPLY_CHARS,
    split_markdown_for_blocks, truncate_chars,
};

/// Cap on the `text` notification fallback.
///
/// `text` is not rendered when `blocks` are present — it is what Slack shows in
/// push notifications and the channel list — so it only needs enough to be
/// recognisable, and Slack rejects the whole post if it runs long.
const SLACK_TEXT_FALLBACK_LIMIT: usize = 3_000;

/// Correlation stamped onto a posted message via `chat.postMessage`'s
/// `metadata`, giving a durable key from a Slack message back to the run that
/// produced it — no tag-string heuristics required.
#[derive(Debug, Clone)]
pub(crate) struct SlackCorrelation {
    pub session_id: String,
    pub input_message_id: String,
}

impl SlackCorrelation {
    fn to_metadata(&self) -> serde_json::Value {
        serde_json::json!({
            "event_type": "everruns_agent_reply",
            "event_payload": {
                "session_id": self.session_id,
                "input_message_id": self.input_message_id,
            }
        })
    }
}

/// Build the `chat.postMessage` payloads for one reply.
///
/// Normally one payload. Replies are bounded to one full Slack message's worth
/// of source text before splitting, which keeps synchronous payload allocation
/// finite while still allowing fence continuations to spill into another API
/// call when their added markers cross the 50-block boundary.
pub(crate) fn build_post_payloads(
    channel: &str,
    thread_ts: &str,
    text: &str,
    correlation: Option<&SlackCorrelation>,
) -> Vec<serde_json::Value> {
    let text = truncate_chars(text, SLACK_MAX_OUTBOUND_REPLY_CHARS);
    let chunks = split_markdown_for_blocks(text, SLACK_MARKDOWN_BLOCK_LIMIT);

    chunks
        .chunks(SLACK_MAX_BLOCKS_PER_MESSAGE)
        .map(|group| {
            let blocks: Vec<serde_json::Value> = group
                .iter()
                .map(|chunk| serde_json::json!({ "type": "markdown", "text": chunk }))
                .collect();

            let mut payload = serde_json::json!({
                "channel": channel,
                // Notification fallback only; `blocks` is what renders.
                "text": truncate_chars(text, SLACK_TEXT_FALLBACK_LIMIT),
                "blocks": blocks,
            });

            if !thread_ts.is_empty() {
                payload["thread_ts"] = serde_json::Value::String(thread_ts.to_string());
            }
            if let Some(correlation) = correlation {
                payload["metadata"] = correlation.to_metadata();
            }
            payload
        })
        .collect()
}

/// Post a message to Slack using the Bot API.
pub(crate) async fn post_to_slack(
    bot_token: &str,
    channel: &str,
    thread_ts: &str,
    text: &str,
) -> anyhow::Result<()> {
    post_to_slack_base(SLACK_API_BASE, bot_token, channel, thread_ts, text).await?;
    Ok(())
}

/// Post a message to Slack using the Bot API (with configurable base URL for testing).
pub(crate) async fn post_to_slack_base(
    base_url: &str,
    bot_token: &str,
    channel: &str,
    thread_ts: &str,
    text: &str,
) -> Result<(), SlackApiError> {
    post_slack_message(base_url, bot_token, channel, thread_ts, text, None).await
}

/// Post one reply, as one or more `chat.postMessage` calls.
///
/// Every call renders through `markdown` blocks, so agent output reaches Slack
/// as the Markdown it actually is rather than being reinterpreted as the much
/// smaller `mrkdwn` dialect.
pub(crate) async fn post_slack_message(
    base_url: &str,
    bot_token: &str,
    channel: &str,
    thread_ts: &str,
    text: &str,
    correlation: Option<&SlackCorrelation>,
) -> Result<(), SlackApiError> {
    let client = reqwest::Client::new();
    let payloads = build_post_payloads(channel, thread_ts, text, correlation);
    let total = payloads.len();

    for (index, payload) in payloads.into_iter().enumerate() {
        let response = client
            .post(format!("{}/chat.postMessage", base_url))
            .header("Authorization", format!("Bearer {}", bot_token))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| SlackApiError::Transient(e.to_string()))?;

        let status = response.status();
        // Read the header before the body is consumed: a 429 carries its advice
        // here, not in the JSON.
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

            // A rate limit is routine backpressure, not a fault to shout about.
            if matches!(failure, SlackApiError::RateLimited { .. }) {
                debug!(
                    channel = channel,
                    retry_after_secs = ?retry_after.map(|d| d.as_secs()),
                    status = %status,
                    part = index + 1,
                    parts = total,
                    "Slack rate limited the post"
                );
            } else {
                error!(
                    channel = channel,
                    error = error,
                    status = %status,
                    part = index + 1,
                    parts = total,
                    "Failed to post message to Slack"
                );
            }
            return Err(failure);
        }
    }

    info!(channel = channel, parts = total, "Posted response to Slack");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    // ==========================================
    // WireMock integration tests — post_to_slack
    // ==========================================

    mod wiremock_tests {
        use super::*;
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        #[tokio::test]
        async fn test_post_to_slack_success() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .and(header("Authorization", "Bearer xoxb-test-token"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": true,
                    "channel": "C123",
                    "ts": "1234567890.123456"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result = post_to_slack_base(
                &mock_server.uri(),
                "xoxb-test-token",
                "C123",
                "1234567890.000000",
                "Hello from agent!",
            )
            .await;

            assert!(result.is_ok());
        }

        #[tokio::test]
        async fn test_post_to_slack_success_no_thread_ts() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": true,
                    "channel": "C123",
                    "ts": "1234567890.123456"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            // Empty thread_ts should still succeed (message not in thread)
            let result =
                post_to_slack_base(&mock_server.uri(), "xoxb-test-token", "C123", "", "Hello!")
                    .await;

            assert!(result.is_ok());
        }

        #[tokio::test]
        async fn test_post_to_slack_channel_not_found() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "channel_not_found"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result = post_to_slack_base(
                &mock_server.uri(),
                "xoxb-test-token",
                "C_INVALID",
                "",
                "Hello!",
            )
            .await;

            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("channel_not_found")
            );
        }

        #[tokio::test]
        async fn test_post_to_slack_invalid_auth() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "invalid_auth"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result =
                post_to_slack_base(&mock_server.uri(), "xoxb-bad-token", "C123", "", "Hello!")
                    .await;

            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("invalid_auth"));
        }

        #[tokio::test]
        async fn test_post_to_slack_rate_limited() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(429).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "ratelimited"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result =
                post_to_slack_base(&mock_server.uri(), "xoxb-test-token", "C123", "", "Hello!")
                    .await;

            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("rate limited"));
        }

        #[tokio::test]
        async fn test_post_to_slack_token_revoked() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "token_revoked"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result =
                post_to_slack_base(&mock_server.uri(), "xoxb-revoked", "C123", "", "Hello!").await;

            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("token_revoked"));
        }

        #[tokio::test]
        async fn test_post_to_slack_unknown_error() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "some_unexpected_error"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result =
                post_to_slack_base(&mock_server.uri(), "xoxb-test-token", "C123", "", "Hello!")
                    .await;

            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("some_unexpected_error")
            );
        }

        #[tokio::test]
        async fn test_post_to_slack_malformed_json() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result =
                post_to_slack_base(&mock_server.uri(), "xoxb-test-token", "C123", "", "Hello!")
                    .await;

            assert!(result.is_err());
        }

        #[tokio::test]
        async fn test_post_to_slack_server_error() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result =
                post_to_slack_base(&mock_server.uri(), "xoxb-test-token", "C123", "", "Hello!")
                    .await;

            // 500 with non-JSON body → reqwest JSON parse error
            assert!(result.is_err());
        }

        #[tokio::test]
        async fn test_post_to_slack_retry_non_retryable_errors_fail_immediately() {
            let mock_server = MockServer::start().await;

            // Non-retryable error should only be called once (no retry)
            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "channel_not_found"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result = post_to_slack_with_retry_base(
                &mock_server.uri(),
                "xoxb-test-token",
                "C_GONE",
                "",
                "Hello!",
                None,
            )
            .await;

            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("channel_not_found")
            );
            // .expect(1) verifies exactly one call was made (no retries)
        }

        #[tokio::test]
        async fn test_post_to_slack_retry_success_on_first_attempt() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": true,
                    "channel": "C123",
                    "ts": "1234567890.123456"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result = post_to_slack_with_retry_base(
                &mock_server.uri(),
                "xoxb-test-token",
                "C123",
                "1234.0000",
                "Hello!",
                None,
            )
            .await;

            assert!(result.is_ok());
        }

        #[tokio::test]
        async fn test_post_to_slack_retry_not_authed() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "not_authed"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result = post_to_slack_with_retry_base(
                &mock_server.uri(),
                "xoxb-test-token",
                "C123",
                "",
                "Hello!",
                None,
            )
            .await;

            assert!(result.is_err());
            // Should not retry — exactly 1 call
        }

        #[tokio::test]
        async fn test_post_to_slack_retry_account_inactive() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "account_inactive"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result = post_to_slack_with_retry_base(
                &mock_server.uri(),
                "xoxb-test-token",
                "C123",
                "",
                "Hello!",
                None,
            )
            .await;

            assert!(result.is_err());
        }

        #[tokio::test]
        async fn test_post_to_slack_retry_no_text() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "no_text"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result = post_to_slack_with_retry_base(
                &mock_server.uri(),
                "xoxb-test-token",
                "C123",
                "",
                "",
                None,
            )
            .await;

            assert!(result.is_err());
        }

        #[tokio::test]
        async fn test_post_to_slack_ok_false_no_error_field() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result =
                post_to_slack_base(&mock_server.uri(), "xoxb-test-token", "C123", "", "Hello!")
                    .await;

            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("unknown"));
        }
    }

    // Regression tests for Slack recovery app ownership.
    //
    // Recovery must use the server-owned `sessions.app_id` FK. Mutable
    // `slack:app:*` routing tags alone must not opt a session into recovery.

    /// EVE-973: one app serves both Slack surfaces, and which one an event
    /// belongs to is read from the event rather than from config.
    mod surface_tests {
        use super::*;

        #[test]
        fn surface_off_is_channel_everywhere() {
            // Including DMs — that is exactly today's behaviour, and enabling the
            // feature must be the only thing that changes it.
            assert_eq!(
                classify_surface(false, Some("im"), "D123"),
                SlackSurface::Channel
            );
            assert_eq!(
                classify_surface(false, Some("channel"), "C123"),
                SlackSurface::Channel
            );
        }

        #[test]
        fn live_events_classify_on_channel_type() {
            assert_eq!(
                classify_surface(true, Some("im"), "D123"),
                SlackSurface::Pane
            );
            for kind in ["channel", "group", "mpim"] {
                assert_eq!(
                    classify_surface(true, Some(kind), "C123"),
                    SlackSurface::Channel,
                    "{kind} is not the pane"
                );
            }
        }

        /// Recovery has no event to read, only the stored channel id.
        #[test]
        fn recovery_classifies_on_channel_id() {
            assert_eq!(classify_surface(true, None, "D0A1B2C3"), SlackSurface::Pane);
            assert_eq!(
                classify_surface(true, None, "C0A1B2C3"),
                SlackSurface::Channel
            );
            assert_eq!(classify_surface(true, None, ""), SlackSurface::Channel);
        }

        /// An `app_mention` delivered with an explicit channel type is channel-shaped
        /// even if the id looks like a DM: the event's own word wins.
        #[test]
        fn channel_type_wins_over_channel_id() {
            assert_eq!(
                classify_surface(true, Some("channel"), "D123"),
                SlackSurface::Channel
            );
        }
    }

    /// EVE-972: every outbound message leaves through `ChannelDeliveryAdapter`,
    /// and transient-vs-permanent is decided in exactly one place.
    mod adapter_routing_tests {
        use super::*;
        use everruns_contracts::typed_id::SessionId;
        use std::collections::HashMap;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        async fn deliver_against(body: serde_json::Value) -> ChannelDeliveryResult {
            let mock_server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .mount(&mock_server)
                .await;

            let adapter = SlackDeliveryAdapter::with_api_base(mock_server.uri());
            let message = OutboundChannelMessage {
                session_id: SessionId::from_uuid(uuid::Uuid::nil()),
                text: "hello".to_string(),
                thread_ref: String::new(),
                correlation_id: None,
            };
            let ctx = ChannelDeliveryContext {
                auth_token: "xoxb-test-token".to_string(),
                channel_id: "C123".to_string(),
                thread_ref: String::new(),
                extra: HashMap::new(),
            };
            adapter.deliver(&message, &ctx).await
        }

        /// `account_inactive` and `no_text` were already treated as unretryable by
        /// the retry loop, but the adapter reported them as transient — the two
        /// lists had drifted apart. One list now answers both questions.
        #[tokio::test]
        async fn permanent_errors_are_reported_permanent() {
            for error in ["channel_not_found", "account_inactive", "no_text"] {
                let result =
                    deliver_against(serde_json::json!({ "ok": false, "error": error })).await;
                assert!(
                    matches!(result, ChannelDeliveryResult::PermanentError(_)),
                    "{error} must be permanent, got {result:?}"
                );
            }
        }

        #[tokio::test]
        async fn unknown_errors_stay_transient() {
            let result =
                deliver_against(serde_json::json!({ "ok": false, "error": "ratelimited" })).await;
            assert!(
                matches!(result, ChannelDeliveryResult::TransientError(_)),
                "a rate limit must stay retryable, got {result:?}"
            );
        }

        #[test]
        fn classification_lists_every_unretryable_error() {
            for error in [
                "channel_not_found",
                "not_authed",
                "invalid_auth",
                "token_revoked",
                "account_inactive",
                "no_text",
            ] {
                assert!(
                    SlackApiError::from_code(error, None).is_permanent(),
                    "{error} must be permanent"
                );
            }
            assert!(!SlackApiError::from_code("internal_error", None).is_permanent());

            // A rate limit is its own variant, never permanent.
            assert!(matches!(
                SlackApiError::from_code("ratelimited", None),
                SlackApiError::RateLimited { .. }
            ));
        }
    }

    /// EVE-968: a 429 carries Slack's own advice on how long to wait. Ignoring it
    /// is how a burst drops replies that would otherwise have gone through.
    mod rate_limit_tests {
        use super::*;
        use std::time::Duration;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        /// Post once against a mock returning `response`, and hand back the error.
        async fn post_against(response: ResponseTemplate) -> SlackApiError {
            let mock_server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(response)
                .mount(&mock_server)
                .await;

            post_to_slack_base(&mock_server.uri(), "xoxb-test-token", "C123", "", "hi")
                .await
                .expect_err("rate limited")
        }

        fn rate_limited_body() -> serde_json::Value {
            serde_json::json!({ "ok": false, "error": "ratelimited" })
        }

        #[tokio::test]
        async fn retry_after_header_is_propagated() {
            let error = post_against(
                ResponseTemplate::new(429)
                    .insert_header("Retry-After", "30")
                    .set_body_json(rate_limited_body()),
            )
            .await;

            assert!(
                matches!(
                    error,
                    SlackApiError::RateLimited {
                        retry_after: Some(d)
                    } if d == Duration::from_secs(30)
                ),
                "expected 30s of advice, got {error:?}"
            );
        }

        #[tokio::test]
        async fn rate_limit_without_header_is_still_rate_limited() {
            let error =
                post_against(ResponseTemplate::new(200).set_body_json(rate_limited_body())).await;

            assert!(
                matches!(error, SlackApiError::RateLimited { retry_after: None }),
                "expected a rate limit with no advice, got {error:?}"
            );
        }

        #[tokio::test]
        async fn unparseable_retry_after_falls_back_to_no_advice() {
            // Slack documents whole seconds; an HTTP-date or junk value must not
            // panic or be mistaken for a duration.
            let error = post_against(
                ResponseTemplate::new(429)
                    .insert_header("Retry-After", "Wed, 21 Oct 2026 07:28:00 GMT")
                    .set_body_json(rate_limited_body()),
            )
            .await;

            assert!(
                matches!(error, SlackApiError::RateLimited { retry_after: None }),
                "expected no usable advice, got {error:?}"
            );
        }

        /// End to end through the retry loop, on virtual time: the first attempt
        /// is rate limited with 30s of advice, the second succeeds, and the loop
        /// must have waited the 30s rather than its 1s backoff.
        #[tokio::test(start_paused = true)]
        async fn retry_loop_waits_the_advised_window() {
            let mock_server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(
                    ResponseTemplate::new(429)
                        .insert_header("Retry-After", "30")
                        .set_body_json(rate_limited_body()),
                )
                .up_to_n_times(1)
                .mount(&mock_server)
                .await;
            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({ "ok": true, "ts": "1.2" })),
                )
                .mount(&mock_server)
                .await;

            let started = tokio::time::Instant::now();
            let result =
                post_to_slack_with_retry_base(&mock_server.uri(), "xoxb-t", "C123", "", "hi", None)
                    .await;
            let waited = started.elapsed();

            assert!(result.is_ok(), "second attempt should succeed: {result:?}");
            assert!(
                waited >= Duration::from_secs(30),
                "loop waited {waited:?}, expected the advised 30s"
            );
        }
    }

    // ==========================================
    // Markdown block rendering (EVE-971)
    // ==========================================

    mod markdown_block_tests {
        use super::*;

        fn blocks_of(payload: &serde_json::Value) -> Vec<String> {
            payload["blocks"]
                .as_array()
                .expect("blocks array")
                .iter()
                .map(|b| {
                    assert_eq!(
                        b["type"], "markdown",
                        "every block must be a markdown block"
                    );
                    b["text"].as_str().unwrap().to_string()
                })
                .collect()
        }

        /// The bug: agent Markdown went out in `text`, which Slack reads as the
        /// much smaller `mrkdwn` dialect. A heading, a table and a fenced code
        /// block must now reach Slack verbatim inside a `markdown` block.
        #[test]
        fn reply_is_posted_as_a_markdown_block_verbatim() {
            let reply =
                "# Heading\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n```rust\nfn main() {}\n```";

            let payloads = build_post_payloads("C123", "1700.1", reply, None);

            assert_eq!(payloads.len(), 1);
            let blocks = blocks_of(&payloads[0]);
            assert_eq!(blocks.len(), 1);
            assert_eq!(blocks[0], reply, "Markdown must not be rewritten");
            // The fallback stays populated for notifications.
            assert_eq!(payloads[0]["text"], reply);
            assert_eq!(payloads[0]["thread_ts"], "1700.1");
        }

        /// `thread_ts` is omitted rather than sent empty for a top-level post.
        #[test]
        fn top_level_post_omits_thread_ts() {
            let payloads = build_post_payloads("C123", "", "hi", None);
            assert!(payloads[0].get("thread_ts").is_none());
        }

        /// Past the block limit the reply is split across blocks, not cut off.
        #[test]
        fn long_reply_splits_across_blocks_without_losing_text() {
            let line = "x".repeat(200);
            let reply = std::iter::repeat_n(line.as_str(), 200)
                .collect::<Vec<_>>()
                .join("\n"); // ~40k characters

            let payloads = build_post_payloads("C123", "", &reply, None);

            assert_eq!(payloads.len(), 1, "40k fits in one message");
            let blocks = blocks_of(&payloads[0]);
            assert!(
                blocks.len() > 1,
                "must actually split, got {}",
                blocks.len()
            );
            for block in &blocks {
                assert!(
                    block.chars().count() <= SLACK_MARKDOWN_BLOCK_LIMIT,
                    "block of {} exceeds Slack's limit",
                    block.chars().count()
                );
            }
            // Nothing was dropped: rejoining reproduces the reply.
            let rejoined: String = blocks.join("");
            assert_eq!(rejoined, reply, "split must be lossless");
        }

        /// A fence interrupted by a split is closed and reopened, so both halves
        /// still render as code rather than the second half leaking as prose.
        #[test]
        fn split_inside_a_fence_closes_and_reopens_it() {
            let body = std::iter::repeat_n("let x = 1;", 2000)
                .collect::<Vec<_>>()
                .join("\n");
            let reply = format!("intro\n\n```rust\n{}\n```", body);

            let blocks = blocks_of(&build_post_payloads("C123", "", &reply, None)[0]);
            assert!(blocks.len() > 1, "fixture must be long enough to split");

            // Every block has balanced fences, so none renders half-open.
            for (i, block) in blocks.iter().enumerate() {
                let fences = block
                    .lines()
                    .filter(|l| l.trim_start().starts_with("```"))
                    .count();
                assert_eq!(
                    fences % 2,
                    0,
                    "block {i} leaves a fence open: {fences} markers"
                );
            }
            // The reopened fence keeps the language hint.
            assert!(
                blocks[1].starts_with("```rust"),
                "continuation must reopen the fence with its info string, got: {:?}",
                &blocks[1][..20.min(blocks[1].len())]
            );
        }

        /// The notification fallback is capped; Slack rejects an oversized `text`.
        #[test]
        fn notification_fallback_is_capped() {
            let reply = "y".repeat(SLACK_TEXT_FALLBACK_LIMIT * 3);
            let payloads = build_post_payloads("C123", "", &reply, None);
            let fallback = payloads[0]["text"].as_str().unwrap();
            assert_eq!(fallback.chars().count(), SLACK_TEXT_FALLBACK_LIMIT);
        }

        /// Past 50 blocks the reply spills into a second message rather than
        /// losing the tail to Slack's per-message block cap.
        #[test]
        fn reply_past_the_block_cap_spills_into_another_message() {
            // Comfortably more than 50 full blocks.
            let reply = "z".repeat(SLACK_MARKDOWN_BLOCK_LIMIT * 52);
            let payloads = build_post_payloads("C123", "1700.1", &reply, None);

            assert_eq!(payloads.len(), 2);
            assert_eq!(blocks_of(&payloads[0]).len(), SLACK_MAX_BLOCKS_PER_MESSAGE);
            assert!(!blocks_of(&payloads[1]).is_empty());
            // Every part still targets the same thread.
            for payload in &payloads {
                assert_eq!(payload["thread_ts"], "1700.1");
            }
        }

        /// Correlation gives a durable Slack-message-to-run key.
        #[test]
        fn correlation_is_stamped_as_message_metadata() {
            let correlation = SlackCorrelation {
                session_id: "session_01abc".to_string(),
                input_message_id: "msg_01xyz".to_string(),
            };
            let payloads = build_post_payloads("C123", "", "hi", Some(&correlation));

            let metadata = &payloads[0]["metadata"];
            assert_eq!(metadata["event_type"], "everruns_agent_reply");
            assert_eq!(metadata["event_payload"]["session_id"], "session_01abc");
            assert_eq!(metadata["event_payload"]["input_message_id"], "msg_01xyz");
        }

        /// Without correlation the key is absent, not empty or invented.
        #[test]
        fn absent_correlation_omits_metadata() {
            let payloads = build_post_payloads("C123", "", "hi", None);
            assert!(payloads[0].get("metadata").is_none());
        }

        /// Multi-byte text must not panic the fallback truncation.
        #[test]
        fn fallback_truncation_respects_char_boundaries() {
            let reply = "\u{1f680}".repeat(SLACK_TEXT_FALLBACK_LIMIT * 2);
            let payloads = build_post_payloads("C123", "", &reply, None);
            let fallback = payloads[0]["text"].as_str().unwrap();
            assert_eq!(fallback.chars().count(), SLACK_TEXT_FALLBACK_LIMIT);
        }

        /// A single unbroken line past the limit has no line boundary to split
        /// on; it must still be chunked rather than emitted oversized.
        #[test]
        fn a_single_oversized_line_is_hard_split() {
            let reply = "q".repeat(SLACK_MARKDOWN_BLOCK_LIMIT * 3);
            let blocks = blocks_of(&build_post_payloads("C123", "", &reply, None)[0]);
            assert!(blocks.len() >= 3);
            for block in &blocks {
                assert!(block.chars().count() <= SLACK_MARKDOWN_BLOCK_LIMIT);
            }
            assert_eq!(blocks.join(""), reply);
        }

        /// Regression for a zero-progress hard split: an opening fence whose
        /// continuation prefix filled the effective block size used to make
        /// the following oversized line loop and allocate forever.
        #[test]
        fn boundary_sized_fence_info_cannot_stall_hard_split() {
            let effective = SLACK_MARKDOWN_BLOCK_LIMIT - 4;
            let reply = format!(
                "```{}\n{}",
                "i".repeat(effective - 4),
                "X".repeat(effective + 1)
            );

            let blocks = split_markdown_for_blocks(&reply, SLACK_MARKDOWN_BLOCK_LIMIT);

            assert!(blocks.len() >= 2);
            assert!(
                blocks
                    .iter()
                    .all(|block| block.chars().count() <= SLACK_MARKDOWN_BLOCK_LIMIT)
            );
        }

        /// Exercise varied limits, fence-info lengths, Unicode, and oversized
        /// lines. Returning proves termination; every output block must retain
        /// the splitter's hard size invariant.
        #[test]
        fn split_markdown_always_makes_progress_and_respects_limits() {
            for limit in 9..80 {
                for info_len in [0, 1, limit / 2, limit, limit * 2] {
                    let reply = format!(
                        "```{}\n{}\n~~~{}\n{}",
                        "lang".repeat(info_len),
                        "🚀".repeat(limit * 3),
                        "x".repeat(info_len),
                        "tail".repeat(limit * 2),
                    );
                    let blocks = split_markdown_for_blocks(&reply, limit);
                    assert!(!blocks.is_empty());
                    assert!(
                        blocks.iter().all(|block| block.chars().count() <= limit),
                        "limit {limit}, info length {info_len}, blocks: {blocks:?}"
                    );
                }
            }
        }

        #[test]
        fn outbound_reply_source_has_a_total_cap() {
            let reply = "z".repeat(SLACK_MAX_OUTBOUND_REPLY_CHARS + 10_000);
            let payloads = build_post_payloads("C123", "", &reply, None);
            let delivered_chars: usize = payloads
                .iter()
                .flat_map(blocks_of)
                .map(|block| block.chars().count())
                .sum();

            assert_eq!(delivered_chars, SLACK_MAX_OUTBOUND_REPLY_CHARS);
        }

        /// End to end through the real post path: the wire body Slack receives
        /// carries the blocks, not just the payload builder's return value.
        #[tokio::test]
        async fn posted_request_body_carries_markdown_blocks_and_metadata() {
            use wiremock::{
                Mock, MockServer, ResponseTemplate,
                matchers::{method, path},
            };

            let mock_server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({"ok": true, "ts": "1.2"})),
                )
                .mount(&mock_server)
                .await;

            let correlation = SlackCorrelation {
                session_id: "session_01abc".to_string(),
                input_message_id: "msg_01xyz".to_string(),
            };
            post_slack_message(
                &mock_server.uri(),
                "xoxb-test-token",
                "C123",
                "1700.1",
                "## Title\n\n```py\nx = 1\n```",
                Some(&correlation),
            )
            .await
            .expect("post should succeed");

            let requests = mock_server.received_requests().await.unwrap();
            assert_eq!(requests.len(), 1);
            let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();

            assert_eq!(body["blocks"][0]["type"], "markdown");
            assert_eq!(body["blocks"][0]["text"], "## Title\n\n```py\nx = 1\n```");
            assert_eq!(
                body["metadata"]["event_payload"]["session_id"],
                "session_01abc"
            );
            assert_eq!(body["thread_ts"], "1700.1");
        }

        /// The ack path has no correlation to stamp, but still renders as a block.
        #[tokio::test]
        async fn ack_path_posts_blocks_without_metadata() {
            use wiremock::{
                Mock, MockServer, ResponseTemplate,
                matchers::{method, path},
            };

            let mock_server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})),
                )
                .mount(&mock_server)
                .await;

            post_to_slack_base(&mock_server.uri(), "xoxb-test-token", "C123", "", "On it.")
                .await
                .expect("ack should succeed");

            let requests = mock_server.received_requests().await.unwrap();
            let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
            assert_eq!(body["blocks"][0]["text"], "On it.");
            assert!(body.get("metadata").is_none());
        }

        /// Short replies stay a single block — no gratuitous splitting.
        #[test]
        fn short_reply_is_one_block() {
            let blocks = blocks_of(&build_post_payloads("C123", "", "ok", None)[0]);
            assert_eq!(blocks, vec!["ok".to_string()]);
        }
    }
}
