//! Thread backfill: paging replies, injecting context, and posting back.

use everruns_core::channel::ThreadContext;
use everruns_platform::SlackReplyMode;

use crate::storage::StorageBackend;

use super::*;

/// Messages Slack returns per `conversations.replies` page. 100 is Slack's
/// documented default and its recommended maximum for this method.
pub(crate) const THREAD_BACKFILL_PAGE_SIZE: u32 = 100;

/// Most thread messages injected into a new session.
///
/// A thread is backfilled in full up to this many messages; beyond it the
/// oldest are dropped so a very long thread cannot exhaust the agent's context
/// window. The agent is told when this happens rather than being handed a
/// window it would read as the whole thread.
pub(crate) const THREAD_BACKFILL_MAX_MESSAGES: usize = 500;

/// Cursor pages followed before giving up, bounding the work one inbound Slack
/// message can cause. At [`THREAD_BACKFILL_PAGE_SIZE`] this reaches 2000
/// messages — far past any thread Slack's own UI stays usable in — so it is a
/// runaway-cursor guard, not the truncation mechanism.
pub(crate) const THREAD_BACKFILL_MAX_PAGES: usize = 20;

/// Thread history fetched for backfill, plus what had to be left out.
#[derive(Debug, Default)]
pub(crate) struct ThreadBackfill {
    /// Messages in chronological order, newest-biased when capped.
    pub(crate) messages: Vec<SlackReplyMessage>,
    /// Older messages dropped to stay within [`THREAD_BACKFILL_MAX_MESSAGES`].
    pub(crate) omitted_older: usize,
    /// False when the page cap stopped us before Slack ran out of cursors, in
    /// which case the *newest* messages are missing too.
    pub(crate) exhausted: bool,
}

impl ThreadBackfill {
    pub(crate) fn is_truncated(&self) -> bool {
        self.omitted_older > 0 || !self.exhausted
    }
}

/// Fetch thread replies from Slack's conversations.replies API.
///
/// Returns messages in chronological order. Gracefully returns an empty
/// backfill on API errors (missing scope, invalid token, etc.) so the agent can
/// proceed without history rather than failing the entire message flow.
pub(crate) async fn fetch_thread_replies(
    bot_token: &str,
    channel: &str,
    thread_ts: &str,
) -> ThreadBackfill {
    fetch_thread_replies_base(SLACK_API_BASE, bot_token, channel, thread_ts).await
}

/// Fetch thread replies, following `response_metadata.next_cursor` until the
/// thread is exhausted (with a configurable base URL for testing).
///
/// A partial page is not an end-of-thread signal — Slack documents the cursor
/// as the only one — so paging stops on an absent or empty cursor.
pub(crate) async fn fetch_thread_replies_base(
    base_url: &str,
    bot_token: &str,
    channel: &str,
    thread_ts: &str,
) -> ThreadBackfill {
    let client = reqwest::Client::new();
    let mut backfill = ThreadBackfill::default();
    let mut cursor: Option<String> = None;

    for _ in 0..THREAD_BACKFILL_MAX_PAGES {
        let mut url = format!(
            "{}/conversations.replies?channel={}&ts={}&limit={}",
            base_url.trim_end_matches('/'),
            urlencoding::encode(channel),
            urlencoding::encode(thread_ts),
            THREAD_BACKFILL_PAGE_SIZE
        );
        if let Some(ref c) = cursor {
            url.push_str(&format!("&cursor={}", urlencoding::encode(c)));
        }

        let result = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", bot_token))
            .send()
            .await;

        let response = match result {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to fetch thread replies (network)");
                // Keep whatever earlier pages produced: partial history the
                // agent is told about beats silently dropping all of it.
                backfill.exhausted = backfill.messages.is_empty();
                return backfill;
            }
        };

        let body: serde_json::Value = match response.json().await {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to parse conversations.replies response");
                backfill.exhausted = backfill.messages.is_empty();
                return backfill;
            }
        };

        if !body.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            let error = body
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("unknown");
            tracing::warn!(
                error = error,
                channel = channel,
                thread_ts = thread_ts,
                "Slack conversations.replies API error (thread context unavailable)"
            );
            backfill.exhausted = backfill.messages.is_empty();
            return backfill;
        }

        match serde_json::from_value::<Vec<SlackReplyMessage>>(
            body.get("messages").cloned().unwrap_or_default(),
        ) {
            Ok(msgs) => backfill.messages.extend(msgs),
            Err(e) => {
                tracing::warn!(error = %e, "Failed to parse thread reply messages");
                backfill.exhausted = backfill.messages.is_empty();
                return backfill;
            }
        }

        // Cap as we go so a runaway thread cannot balloon memory either.
        if backfill.messages.len() > THREAD_BACKFILL_MAX_MESSAGES {
            let excess = backfill.messages.len() - THREAD_BACKFILL_MAX_MESSAGES;
            backfill.messages.drain(0..excess);
            backfill.omitted_older += excess;
        }

        cursor = body
            .get("response_metadata")
            .and_then(|m| m.get("next_cursor"))
            .and_then(|c| c.as_str())
            .filter(|c| !c.is_empty())
            .map(str::to_string);

        if cursor.is_none() {
            backfill.exhausted = true;
            return backfill;
        }
    }

    tracing::warn!(
        channel = channel,
        thread_ts = thread_ts,
        max_pages = THREAD_BACKFILL_MAX_PAGES,
        "Stopped paging thread replies at the page cap; newest messages may be missing"
    );
    backfill
}

/// Inject thread history as context messages into a newly created session.
///
/// Fetches prior messages from Slack's conversations.replies API and stores
/// them as input.message events (without triggering agent workflows). This
/// gives the agent full conversational context when first mentioned mid-thread.
///
/// Messages from users get user-role with ExternalActor attribution.
/// Bot-authored replies are skipped to avoid role confusion from third-party
/// bots in shared channels.
/// The triggering message (matching `exclude_ts`) is skipped since it will be
/// created normally by the caller.
pub(crate) async fn inject_thread_context(
    state: &SlackState,
    bot_token: &str,
    channel: &str,
    thread_ts: &str,
    session_id: everruns_provider::typed_id::SessionId,
    exclude_ts: Option<&str>,
) -> anyhow::Result<()> {
    let backfill = fetch_thread_replies(bot_token, channel, thread_ts).await;

    if backfill.messages.is_empty() {
        return Ok(());
    }

    // Say up front that this is a window, not the thread. Injected before the
    // history so the agent reads the qualifier before the messages it governs.
    if backfill.is_truncated() {
        let notice = truncation_notice(&backfill);
        tracing::info!(
            session_id = %session_id,
            thread_ts = thread_ts,
            omitted_older = backfill.omitted_older,
            exhausted = backfill.exhausted,
            "Thread backfill truncated; telling the agent"
        );
        let message = everruns_core::Message {
            id: everruns_provider::typed_id::MessageId::new(),
            role: everruns_core::MessageRole::System,
            content: vec![everruns_core::ContentPart::text(&notice)],
            phase: None,
            phase_source: None,
            controls: None,
            metadata: None,
            external_actor: None,
            created_at: chrono::Utc::now(),
        };
        state
            .event_service
            .emit(everruns_core::events::EventRequest::new(
                session_id,
                everruns_core::events::EventContext::empty(),
                everruns_core::events::InputMessageData::new(message),
            ))
            .await?;
    }

    let mut injected = 0u32;
    for reply in &backfill.messages {
        if should_skip_thread_reply(reply, exclude_ts) {
            continue;
        }

        // Safe because should_skip_thread_reply filters empty content.
        let text = reply.text.as_deref().unwrap_or("");
        let user_id = reply.user.clone().unwrap_or_default();
        let display_name = if !user_id.is_empty() {
            resolve_slack_user_name(&state.user_name_cache, bot_token, &user_id).await
        } else {
            None
        };
        let external_actor = if !user_id.is_empty() {
            Some(everruns_core::ExternalActor {
                actor_id: user_id,
                actor_name: display_name,
                source: "slack".to_string(),
                metadata: None,
            })
        } else {
            None
        };

        let message = everruns_core::Message {
            id: everruns_provider::typed_id::MessageId::new(),
            role: everruns_core::MessageRole::User,
            content: vec![everruns_core::ContentPart::text(text)],
            phase: None,
            phase_source: None,
            controls: None,
            metadata: reply.ts.as_ref().map(|ts| {
                [(
                    "slack_ts".to_string(),
                    serde_json::Value::String(ts.clone()),
                )]
                .into_iter()
                .collect()
            }),
            external_actor,
            created_at: chrono::Utc::now(),
        };

        // Emit as input.message event directly (no workflow trigger)
        state
            .event_service
            .emit(everruns_core::events::EventRequest::new(
                session_id,
                everruns_core::events::EventContext::empty(),
                everruns_core::events::InputMessageData::new(message),
            ))
            .await?;

        injected += 1;
    }

    if injected > 0 {
        tracing::info!(
            session_id = %session_id,
            thread_ts = thread_ts,
            injected_count = injected,
            "Injected thread context into new session"
        );
    }

    Ok(())
}

/// Wording for the truncation notice injected ahead of a capped backfill.
///
/// Names the cap so the agent can tell "this thread is short" from "you are
/// seeing the tail of a long one", and stays vague only where we genuinely do
/// not know how much is missing.
pub(crate) fn truncation_notice(backfill: &ThreadBackfill) -> String {
    let shown = backfill.messages.len();
    if backfill.exhausted {
        format!(
            "[Thread history truncated: showing the most recent {} messages of this Slack thread; \
             {} earlier messages were omitted.]",
            shown, backfill.omitted_older
        )
    } else {
        format!(
            "[Thread history truncated: showing {} messages from this Slack thread. It was too \
             long to read in full, so both earlier and more recent messages may be missing.]",
            shown
        )
    }
}

/// Read the session's persisted `ThreadContext`.
///
/// Goes through `StorageBackend` rather than a `SessionStorageStore` handle
/// because the webhook holds the backend and must work against both the
/// Postgres and in-memory backends. The key and the JSON shape come from
/// `everruns_core::channel`, which prompt assembly reads through the trait, so
/// the two paths cannot drift (EVE-977).
pub(crate) async fn load_thread_context(
    state: &SlackState,
    session_id: everruns_provider::typed_id::SessionId,
) -> Option<ThreadContext> {
    match state
        .db
        .get_session_key_value(
            session_id.uuid(),
            everruns_core::channel::THREAD_CONTEXT_KV_KEY,
        )
        .await
    {
        Ok(Some(row)) => everruns_core::channel::decode_thread_context(&row.value),
        Ok(None) => None,
        Err(error) => {
            tracing::warn!(%session_id, %error, "Failed to read persisted thread context");
            None
        }
    }
}

/// Persist the session's `ThreadContext`, replacing any previous record.
pub(crate) async fn save_thread_context(
    state: &SlackState,
    session_id: everruns_provider::typed_id::SessionId,
    context: &ThreadContext,
) -> anyhow::Result<()> {
    let value = everruns_core::channel::encode_thread_context(context)?;
    state
        .db
        .upsert_session_key_value(crate::storage::models::UpsertSessionKeyValue {
            session_id,
            key: everruns_core::channel::THREAD_CONTEXT_KV_KEY.to_string(),
            value,
        })
        .await?;
    Ok(())
}

pub(crate) fn should_skip_thread_reply(
    reply: &SlackReplyMessage,
    exclude_ts: Option<&str>,
) -> bool {
    // Skip the triggering message (it will be created by the normal flow).
    if let (Some(reply_ts), Some(skip_ts)) = (reply.ts.as_deref(), exclude_ts)
        && reply_ts == skip_ts
    {
        return true;
    }

    // Skip messages with no text content.
    if reply.text.as_deref().unwrap_or("").is_empty() {
        return true;
    }

    // Skip subtypes we can't meaningfully represent (channel_join, etc.).
    if let Some(ref subtype) = reply.subtype
        && subtype != "bot_message"
        && subtype != "thread_broadcast"
    {
        return true;
    }

    // Ignore bot-authored history entries. We cannot safely treat arbitrary
    // Slack bot output as trusted assistant context.
    if reply.bot_id.is_some() {
        return true;
    }

    false
}

/// Wait for the agent turn to complete and stream responses to Slack.
///
/// Posts each `output.message.completed` text to Slack as it arrives, giving
/// users real-time progress during multi-step turns (Reason→Act cycles).
/// Filtering by `input_message_id` ensures we only see events from our turn.
/// Stops polling when `turn.completed` or `turn.failed` fires.
pub(crate) async fn wait_and_post_response(
    db: &StorageBackend,
    session_id: uuid::Uuid,
    input_message_id: everruns_provider::typed_id::MessageId,
    bot_token: &str,
    channel: &str,
    thread_ts: &str,
    reply_mode: SlackReplyMode,
) -> anyhow::Result<()> {
    use everruns_provider::typed_id::{EventId, SessionId};

    let session_id_typed = SessionId::from_uuid(session_id);
    let input_msg_str = input_message_id.to_string();

    // Poll for events (max 120 seconds)
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(120);
    let mut since_id: Option<EventId> = None;
    let empty: Vec<String> = vec![];

    loop {
        if tokio::time::Instant::now() > deadline {
            tracing::warn!(session_id = %session_id, "Timed out waiting for agent response");
            break;
        }

        let events = db
            .list_events(session_id_typed, None, since_id, &empty, &empty, None, None)
            .await?;

        for event_row in &events {
            since_id = Some(event_row.id);

            // Only consider events triggered by our input message
            let event_input_msg = event_row
                .context
                .get("input_message_id")
                .and_then(|v| v.as_str());
            let is_our_turn = event_input_msg == Some(&input_msg_str);

            // Post each assistant message to Slack as it arrives. This gives
            // users progress visibility during multi-step agent turns (e.g.
            // "Let me search for that..." before tool execution).
            if is_our_turn
                && let Some(text) = crate::slack_delivery::extract_delivery_text(
                    &event_row.event_type,
                    reply_mode,
                    &event_row.data,
                )
            {
                post_to_slack(bot_token, channel, thread_ts, &text).await?;
            }

            // Stop polling once the turn ends
            if event_row.event_type == "turn.completed" && is_our_turn {
                return Ok(());
            }

            if event_row.event_type == "turn.failed" && is_our_turn {
                tracing::warn!(session_id = %session_id, "Turn failed");
                return Ok(());
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    Ok(())
}
