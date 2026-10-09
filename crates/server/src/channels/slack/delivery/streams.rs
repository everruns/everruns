// Streaming a Slack reply as the model produces it: the Slack wire for
// `chat.startStream`, `chat.appendStream`, `chat.stopStream`, and the
// close-then-rewrite that stands in for an edit Slack will not allow
// mid-stream. Per-message stream state (what was produced, what was sent) is
// the shared channel runtime's `TurnDelivery`.

use super::*;

/// Slack caps `markdown_text` at 12,000 characters per append.
const SLACK_APPEND_MAX_CHARS: usize = 12_000;

#[async_trait]
impl ChannelStreamDelivery for SlackDeliveryAdapter {
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
            &self.api_base,
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
            "markdown_text": truncate_chars(text, SLACK_APPEND_MAX_CHARS),
        });

        match slack_api_call(
            &self.api_base,
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
            "text": text,
            "blocks": [{ "type": "markdown", "text": text }],
        });
        match slack_api_call(&self.api_base, &context.auth_token, "chat.update", payload).await {
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
            &self.api_base,
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
