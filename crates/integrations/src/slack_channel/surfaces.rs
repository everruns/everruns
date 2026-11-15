// Streaming a Slack reply as the model produces it: the Slack wire for
// `chat.startStream`, `chat.appendStream`, `chat.stopStream`, and the
// close-then-rewrite that stands in for an edit Slack will not allow
// mid-stream. Per-message stream state (what was produced, what was sent) is
// the shared channel runtime's `TurnDelivery`.

use super::{
    markdown::{markdown_block, neutralize_broadcasts},
    web_api::{SlackApiError, slack_api_call},
};
use async_trait::async_trait;
use everruns_contracts::runtime::channel::{
    ChannelAgentSurface, ChannelProgressSurface, ChannelStreamDelivery,
    DeliveryContext as ChannelDeliveryContext, DeliveryResult as ChannelDeliveryResult,
};
/// Shared Slack wire adapter, used by both application and hosted channels.
pub struct SlackSurfaces<'a> {
    /// Trusted Web API base.
    pub api_base: &'a str,
}
fn truncate_chars(text: &str, max: usize) -> &str {
    text.char_indices()
        .nth(max)
        .map_or(text, |(i, _)| &text[..i])
}
/// Recipient metadata for Slack streams.
pub const SLACK_RECIPIENT_USER_ID: &str = "slack_recipient_user_id";
/// Recipient team metadata for Slack streams.
pub const SLACK_RECIPIENT_TEAM_ID: &str = "slack_recipient_team_id";

/// Slack caps `markdown_text` at 12,000 characters per append.
const SLACK_APPEND_MAX_CHARS: usize = 12_000;

#[async_trait]
impl ChannelStreamDelivery for SlackSurfaces<'_> {
    async fn start(&self, context: &ChannelDeliveryContext) -> Result<String, String> {
        let mut payload = serde_json::json!({ "channel": context.channel_id });

        if !context.thread_ref.is_empty() {
            payload["thread_ts"] = serde_json::Value::String(context.thread_ref.clone());
        }
        for (extra_key, field) in [
            (SLACK_RECIPIENT_USER_ID, "recipient_user_id"),
            (SLACK_RECIPIENT_TEAM_ID, "recipient_team_id"),
        ] {
            if let Some(value) = context.extra.get(extra_key) {
                payload[field] = serde_json::Value::String(value.clone());
            }
        }

        let body = slack_api_call(
            self.api_base,
            &context.auth_token,
            "chat.startStream",
            payload,
        )
        .await
        .map_err(|e| e.to_string())?;

        body.get("ts")
            .and_then(|ts| ts.as_str())
            .map(str::to_string)
            .ok_or_else(|| "chat.startStream returned no ts".to_string())
    }

    async fn append(
        &self,
        handle: &str,
        text: &str,
        context: &ChannelDeliveryContext,
    ) -> ChannelDeliveryResult {
        let payload = serde_json::json!({
            "channel": context.channel_id,
            "ts": handle,
            // The core holds back an unclosed `<...` tail, so a broadcast
            // token arrives whole and is caught here (TM-SLACK-015).
            "markdown_text": neutralize_broadcasts(truncate_chars(text, SLACK_APPEND_MAX_CHARS)),
        });

        match slack_api_call(
            self.api_base,
            &context.auth_token,
            "chat.appendStream",
            payload,
        )
        .await
        {
            Ok(_) => ChannelDeliveryResult::Ok,
            Err(e) => classify_slack_failure(e),
        }
    }

    async fn replace(
        &self,
        handle: &str,
        text: &str,
        context: &ChannelDeliveryContext,
    ) -> ChannelDeliveryResult {
        // Slack does not permit an in-progress stream to be edited. Close it
        // first, then rewrite the same message so retracted text cannot remain.
        let stop_result = self.stop(handle, context).await;
        if !matches!(stop_result, ChannelDeliveryResult::Ok) {
            return stop_result;
        }
        let payload = serde_json::json!({
            "channel": context.channel_id,
            "ts": handle,
            "text": neutralize_broadcasts(text),
            "blocks": [markdown_block(text)],
        });
        match slack_api_call(self.api_base, &context.auth_token, "chat.update", payload).await {
            Ok(_) => ChannelDeliveryResult::Ok,
            Err(error) => classify_slack_failure(error),
        }
    }

    async fn stop(&self, handle: &str, context: &ChannelDeliveryContext) -> ChannelDeliveryResult {
        let payload = serde_json::json!({
            "channel": context.channel_id,
            "ts": handle,
        });

        match slack_api_call(
            self.api_base,
            &context.auth_token,
            "chat.stopStream",
            payload,
        )
        .await
        {
            Ok(_) => ChannelDeliveryResult::Ok,
            Err(e) => classify_slack_failure(e),
        }
    }
}

pub fn classify_slack_failure(error: SlackApiError) -> ChannelDeliveryResult {
    match error {
        SlackApiError::Permanent(message) => ChannelDeliveryResult::PermanentError(message),
        other => ChannelDeliveryResult::TransientError(other.to_string()),
    }
}

const SLACK_SET_STATUS_METHOD: &str = "agents.sessions.setStatus";
const SLACK_SET_TITLE_METHOD: &str = "agents.sessions.rename";

/// Slack rejects a title longer than this.
const SLACK_TITLE_MAX_CHARS: usize = 250;

#[async_trait]
impl ChannelAgentSurface for SlackSurfaces<'_> {
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
            self.api_base,
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
            self.api_base,
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

#[async_trait]
impl ChannelProgressSurface for SlackSurfaces<'_> {
    async fn post(&self, text: &str, context: &ChannelDeliveryContext) -> Result<String, String> {
        let mut payload = serde_json::json!({"channel":context.channel_id,"text":neutralize_broadcasts(text),"blocks":[markdown_block(text)]});
        if !context.thread_ref.is_empty() {
            payload["thread_ts"] = context.thread_ref.clone().into();
        }
        let body = slack_api_call(
            self.api_base,
            &context.auth_token,
            "chat.postMessage",
            payload,
        )
        .await
        .map_err(|e| e.to_string())?;
        body["ts"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| "Slack returned no timestamp".into())
    }
    async fn update(
        &self,
        handle: &str,
        text: &str,
        context: &ChannelDeliveryContext,
    ) -> ChannelDeliveryResult {
        match slack_api_call(self.api_base,&context.auth_token,"chat.update",serde_json::json!({"channel":context.channel_id,"ts":handle,"text":neutralize_broadcasts(text),"blocks":[markdown_block(text)]})).await {Ok(_)=>ChannelDeliveryResult::Ok,Err(e)=>classify_slack_failure(e)}
    }
}
