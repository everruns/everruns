//! Thread backfill: paging a thread's replies for a new conversation.

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

/// Fetch thread replies, following `response_metadata.next_cursor` until the
/// thread is exhausted (with a configurable base URL for testing).
///
/// A partial page is not an end-of-thread signal — Slack documents the cursor
/// as the only one — so paging stops on an absent or empty cursor.
#[cfg(test)]
pub(crate) async fn fetch_thread_replies_base(
    base_url: &str,
    bot_token: &str,
    channel: &str,
    thread_ts: &str,
) -> ThreadBackfill {
    let client = match everruns_integrations::slack_channel::bridge::direct_factory() {
        Ok(factory) => {
            factory
                .for_credentials("slack-history-compat", None, bot_token, base_url, None)
                .await
        }
        Err(error) => Err(error),
    };
    let Ok(client) = client else {
        return ThreadBackfill {
            exhausted: true,
            ..Default::default()
        };
    };
    let target = channelkit::ConversationKey::new(
        channelkit::Provider::Slack,
        "slack-history-compat",
        channel,
        Some(thread_ts.into()),
    );
    let Ok(target) = target else {
        return ThreadBackfill {
            exhausted: true,
            ..Default::default()
        };
    };
    fetch_thread_replies_client(&client, &target).await
}

pub(crate) async fn fetch_thread_replies_client(
    client: &channelkit_slack::Slack,
    target: &channelkit::ConversationKey,
) -> ThreadBackfill {
    let mut backfill = ThreadBackfill::default();
    let mut cursor: Option<String> = None;
    for _ in 0..THREAD_BACKFILL_MAX_PAGES {
        let page = client
            .history_with_context(
                target,
                cursor.as_deref(),
                THREAD_BACKFILL_PAGE_SIZE as usize,
            )
            .await;
        let Ok(page) = page else {
            backfill.exhausted = backfill.messages.is_empty();
            return backfill;
        };
        backfill
            .messages
            .extend(page.messages.into_iter().map(|entry| SlackReplyMessage {
                user: Some(entry.message.author.id),
                text: Some(entry.message.text),
                ts: Some(entry.message.reference.id),
                bot_id: entry.message.author.is_bot.then(|| "bot".into()),
                subtype: entry.context.subtype,
            }));
        // Cap as we go so a runaway thread cannot balloon memory either.
        if backfill.messages.len() > THREAD_BACKFILL_MAX_MESSAGES {
            let excess = backfill.messages.len() - THREAD_BACKFILL_MAX_MESSAGES;
            backfill.messages.drain(0..excess);
            backfill.omitted_older += excess;
        }

        cursor = page.next_cursor;

        if cursor.is_none() {
            backfill.exhausted = true;
            return backfill;
        }
    }

    tracing::warn!(
        channel = target.channel,
        thread_ts = ?target.thread,
        max_pages = THREAD_BACKFILL_MAX_PAGES,
        "Stopped paging thread replies at the page cap; newest messages may be missing"
    );
    backfill
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
