// Streaming a Slack reply as the model produces it.
//
// Two halves of one concern. The dispatcher side owns the per-message stream
// state — what has been accumulated, what has been sent, who is currently
// flushing it — because a turn with three output messages is three streams.
// The adapter side is the Slack wire: `chat.startStream`, `chat.appendStream`,
// `chat.stopStream`, and the close-then-rewrite that stands in for an edit
// Slack will not allow mid-stream.
//
// Split out of `slack_delivery.rs`, which is on the source-file size debt list.

use super::*;

impl SlackDeliveryDispatcher {
    /// Replace the accumulated text for an open stream, if it is still open.
    pub(super) async fn set_accumulated(
        &self,
        key: &DeliveryKey,
        message_id: &str,
        text: String,
    ) -> bool {
        let mut deliveries = self.deliveries.write().await;
        match deliveries
            .get_mut(key)
            .and_then(|c| c.streams.get_mut(message_id))
        {
            Some(state) => {
                state.accumulated = text;
                true
            }
            None => false,
        }
    }

    /// Output messages with a stream still open on this delivery.
    pub(super) async fn open_stream_ids(&self, key: &DeliveryKey) -> Vec<String> {
        self.deliveries
            .read()
            .await
            .get(key)
            .map(|ctx| ctx.streams.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Take the text waiting on one stream, advancing `sent` under the lock.
    ///
    /// Claiming before the network call is what makes concurrent flushes safe:
    /// two flushers cannot both read the same `sent` and transmit the same bytes.
    /// Returns the handle, the claimed text, and the offset to restore if the send
    /// fails.
    pub(super) async fn claim_pending(
        &self,
        key: &DeliveryKey,
        message_id: &str,
    ) -> Option<(String, String, usize)> {
        let mut deliveries = self.deliveries.write().await;
        let state = deliveries.get_mut(key)?.streams.get_mut(message_id)?;

        let pending = state.pending().to_string();
        if pending.is_empty() {
            return None;
        }

        let claimed_from = state.sent;
        state.sent = state.accumulated.len();
        Some((state.handle.clone(), pending, claimed_from))
    }

    /// Give a claim back after a failed send, so the text is retried rather than
    /// silently dropped from the middle of a reply.
    pub(super) async fn release_claim(
        &self,
        key: &DeliveryKey,
        message_id: &str,
        claimed_from: usize,
    ) {
        let mut deliveries = self.deliveries.write().await;
        if let Some(ctx) = deliveries.get_mut(key)
            && let Some(state) = ctx.streams.get_mut(message_id)
        {
            state.sent = state.sent.min(claimed_from);
        }
    }

    /// Send whatever has accumulated on one stream since the last flush.
    pub(super) async fn flush_stream(
        &self,
        key: &DeliveryKey,
        ctx: &DeliveryContext,
        message_id: &str,
    ) {
        let Some(stream) = self.streaming_for(ctx) else {
            return;
        };
        let Some((handle, pending, claimed_from)) = self.claim_pending(key, message_id).await
        else {
            return;
        };

        if let ChannelDeliveryResult::TransientError(e) | ChannelDeliveryResult::PermanentError(e) =
            stream
                .append(&handle, &pending, &self.delivery_context(ctx))
                .await
        {
            warn!(error = %e, "Failed to append to Slack stream");
            self.release_claim(key, message_id, claimed_from).await;
        }
    }

    /// Flush the tail and close the stream, then forget it.
    ///
    /// Always closes, even when the append failed — a stream left open spins in
    /// the client forever, which is worse than a truncated reply.
    pub(super) async fn close_stream(
        &self,
        key: &DeliveryKey,
        ctx: &DeliveryContext,
        message_id: &str,
    ) {
        self.flush_stream(key, ctx, message_id).await;

        let handle = {
            let mut deliveries = self.deliveries.write().await;
            match deliveries
                .get_mut(key)
                .and_then(|c| c.streams.remove(message_id))
            {
                Some(state) => state.handle,
                None => return,
            }
        };

        let Some(stream) = self.streaming_for(ctx) else {
            return;
        };
        if let ChannelDeliveryResult::TransientError(e) | ChannelDeliveryResult::PermanentError(e) =
            stream.stop(&handle, &self.delivery_context(ctx)).await
        {
            warn!(error = %e, "Failed to stop Slack stream");
        }
    }

    /// Replace a guardrail-retracted stream in place and forget the closed
    /// stream only after the platform confirms that the unsafe text is gone.
    pub(super) async fn replace_stream(
        &self,
        key: &DeliveryKey,
        ctx: &DeliveryContext,
        message_id: &str,
        replacement: &str,
    ) -> bool {
        let handle = {
            let deliveries = self.deliveries.read().await;
            deliveries
                .get(key)
                .and_then(|live| live.streams.get(message_id))
                .map(|state| state.handle.clone())
        };
        let Some(handle) = handle else {
            return false;
        };
        let Some(stream) = self.streaming_for(ctx) else {
            return false;
        };

        match stream
            .replace(&handle, replacement, &self.delivery_context(ctx))
            .await
        {
            ChannelDeliveryResult::Ok => {
                if let Some(live) = self.deliveries.write().await.get_mut(key) {
                    live.streams.remove(message_id);
                    live.replaced_messages.insert(message_id.to_string());
                }
                true
            }
            ChannelDeliveryResult::TransientError(error)
            | ChannelDeliveryResult::PermanentError(error) => {
                error!(%error, "Failed to replace guardrail-retracted Slack stream");
                false
            }
        }
    }
}

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
