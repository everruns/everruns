//! Turning agent Markdown into what Slack renders.
//!
//! Agents write standard Markdown on every channel. Slack's `text` field is
//! `mrkdwn`, a different and much smaller dialect, so every post carries the
//! Markdown in `markdown` blocks instead and keeps `text` only as the
//! notification fallback. Both Slack drivers (the server's and the framework's
//! [`super::Slack`]) build their payloads here, so they render the same way.
//!
//! Decisions:
//! - Slack's `markdown` block takes 12k characters and a message takes 50 of
//!   them, so a long reply is cut on line boundaries. Cutting inside a fenced
//!   code block would leave both halves rendering as prose, so the fence is
//!   closed and reopened around the seam: the TM-DOS-040 bounds are about that.
//! - Broadcast tokens (`<!here>`, `<!channel>`, `<!everyone>`) are rewritten to
//!   plain `@here` text, which Slack does not treat as a mention. Output is
//!   model-generated and may be steered by untrusted input; paging a whole
//!   channel is never the agent's call (TM-SLACK-015).
//! - Markdown is otherwise passed through unmodified. Slack's renderer keeps
//!   improving, and any rewrite here would be a lossy guess against it.
//! - Slack rejects a whole `markdown` block over one bad token (an unknown
//!   `<!subteam^...>` gives `invalid_blocks`). [`plain_text_retry`] rebuilds such
//!   a post as plain `text` so the reply still arrives, unformatted.

use std::borrow::Cow;

use serde_json::{Value, json};

/// Characters Slack accepts in one `markdown` block.
pub const SLACK_MARKDOWN_BLOCK_LIMIT: usize = 12_000;

/// Blocks Slack accepts in one `chat.postMessage` call.
pub const SLACK_MAX_BLOCKS_PER_MESSAGE: usize = 50;

/// Maximum assistant output considered for one Slack delivery.
///
/// This bounds synchronous payload construction before the first Slack API
/// request. The cap still allows a full message's worth of Markdown blocks.
pub const SLACK_MAX_OUTBOUND_REPLY_CHARS: usize =
    SLACK_MARKDOWN_BLOCK_LIMIT * SLACK_MAX_BLOCKS_PER_MESSAGE;

/// Truncate on a char boundary, so multi-byte text cannot panic the slice.
pub fn truncate_chars(text: &str, limit: usize) -> &str {
    match text.char_indices().nth(limit) {
        Some((idx, _)) => &text[..idx],
        None => text,
    }
}

/// Split Markdown into pieces that each fit one Slack `markdown` block.
///
/// Splits on line boundaries, and never leaves a fenced code block open: when a
/// boundary lands inside a fence the fence is closed at the end of the piece and
/// reopened — with its original info string — at the start of the next, so a
/// split code block still renders as code on both sides.
pub fn split_markdown_for_blocks(text: &str, limit: usize) -> Vec<String> {
    // THREAT[TM-DOS-040]: Fence continuations must be bounded and every hard
    // split iteration must either consume input or reset to a consumable state.
    debug_assert!(limit > 8, "limit must leave room for fence markers");
    if text.chars().count() <= limit {
        return vec![text.to_string()];
    }

    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;
    // The fence currently open, as (marker, info string) — e.g. ("```", "rust").
    let mut open_fence: Option<(String, String)> = None;
    // Reopened at the top of the next chunk when a split interrupts a fence.
    let mut reopen: Option<String> = None;

    // Reserve room for the closing fence we may have to append.
    let effective = limit.saturating_sub(4);

    let flush = |current: &mut String,
                 current_len: &mut usize,
                 open_fence: &Option<(String, String)>,
                 chunks: &mut Vec<String>,
                 reopen: &mut Option<String>| {
        if current.is_empty() {
            return;
        }
        let mut chunk = std::mem::take(current);
        if let Some((marker, info)) = open_fence {
            // Close the fence here and reopen it in the next chunk.
            if !chunk.ends_with('\n') {
                chunk.push('\n');
            }
            chunk.push_str(marker);
            *reopen = Some(format!("{}{}", marker, info));
        } else {
            *reopen = None;
        }
        chunks.push(chunk);
        *current_len = 0;
    };

    for line in text.split_inclusive('\n') {
        let line_len = line.chars().count();

        // A line that does not fit the current chunk has to go through the
        // hard-split path. This includes a short line following a long fence
        // reopen prefix, not only lines that exceed an empty chunk.
        if line_len > effective.saturating_sub(current_len) {
            flush(
                &mut current,
                &mut current_len,
                &open_fence,
                &mut chunks,
                &mut reopen,
            );
            if let Some(ref head) = reopen.take() {
                current.push_str(head);
                current.push('\n');
                current_len = head.chars().count() + 1;
            }
            let mut rest = line;
            while rest.chars().count() > effective.saturating_sub(current_len) {
                let room = effective.saturating_sub(current_len);
                // Fence reopen prefixes are bounded below, so this is
                // unreachable. Keep the guard at the consumption point: no
                // malformed Markdown may turn this into a zero-progress loop.
                if room == 0 {
                    current.clear();
                    current_len = 0;
                    open_fence = None;
                    reopen = None;
                    continue;
                }
                let head = truncate_chars(rest, room);
                current.push_str(head);
                current_len += head.chars().count();
                rest = &rest[head.len()..];
                flush(
                    &mut current,
                    &mut current_len,
                    &open_fence,
                    &mut chunks,
                    &mut reopen,
                );
                if let Some(ref h) = reopen.take() {
                    current.push_str(h);
                    current.push('\n');
                    current_len = h.chars().count() + 1;
                }
            }
            current.push_str(rest);
            current_len += rest.chars().count();
            continue;
        }

        if current_len + line_len > effective {
            flush(
                &mut current,
                &mut current_len,
                &open_fence,
                &mut chunks,
                &mut reopen,
            );
            if let Some(head) = reopen.take() {
                current.push_str(&head);
                current.push('\n');
                current_len = head.chars().count() + 1;
            }
        }

        // Track fence state after placement, so the marker line itself lands in
        // the chunk that opens or closes it.
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed
            .strip_prefix("```")
            .or_else(|| trimmed.strip_prefix("~~~"))
        {
            let marker = &trimmed[..3];
            match open_fence {
                // A closing fence carries no info string.
                Some((ref open_marker, _)) if open_marker == marker => open_fence = None,
                Some(_) => {}
                None => {
                    // A continuation must leave room for its newline and at
                    // least one input character. Fence info is presentation
                    // metadata, so bounding only the repeated copy preserves
                    // the original opening line while guaranteeing progress.
                    let max_info_len = effective.saturating_sub(marker.chars().count() + 2);
                    let info = truncate_chars(rest.trim_end(), max_info_len).to_string();
                    open_fence = Some((marker.to_string(), info));
                }
            }
        }

        current.push_str(line);
        current_len += line_len;
    }

    flush(
        &mut current,
        &mut current_len,
        &open_fence,
        &mut chunks,
        &mut reopen,
    );

    chunks
}

/// Rewrite broadcast mentions to plain text, so agent output cannot page a
/// whole channel.
///
/// `<!here>`, `<!channel>` and `<!everyone>` (with or without a `|label`)
/// become `@here`, `@channel` and `@everyone`, which Slack shows as text in
/// both `markdown` blocks and `mrkdwn`. User, channel and user-group mentions
/// are left alone: they notify only the people they name.
pub fn neutralize_broadcasts(text: &str) -> Cow<'_, str> {
    // THREAT[TM-SLACK-015]: model output may carry an injected broadcast.
    if !text.contains("<!") {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<!") {
        out.push_str(&rest[..start]);
        let tail = &rest[start + 2..];
        let replaced = ["here", "channel", "everyone"].iter().find_map(|name| {
            let after = tail.strip_prefix(name)?;
            let end = match after.as_bytes().first() {
                Some(b'>') => 1,
                Some(b'|') => after.find('>')? + 1,
                _ => return None,
            };
            Some((*name, name.len() + end))
        });
        match replaced {
            Some((name, consumed)) => {
                out.push('@');
                out.push_str(name);
                rest = &tail[consumed..];
            }
            None => {
                out.push_str("<!");
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// One `markdown` block carrying agent text, broadcasts neutralized.
///
/// For calls that post a single bounded text (an update, a file comment);
/// a full reply goes through [`markdown_block_groups`].
pub fn markdown_block(text: &str) -> Value {
    json!({ "type": "markdown", "text": neutralize_broadcasts(text) })
}

/// A reply as groups of `markdown` blocks, one group per `chat.postMessage`.
///
/// Normally one group. The source is capped at one full Slack message of
/// text before splitting, which keeps payload construction finite, while fence
/// continuations may still spill into a second call.
pub fn markdown_block_groups(text: &str) -> Vec<Vec<Value>> {
    let text = neutralize_broadcasts(truncate_chars(text, SLACK_MAX_OUTBOUND_REPLY_CHARS));
    let chunks = split_markdown_for_blocks(&text, SLACK_MARKDOWN_BLOCK_LIMIT);
    chunks
        .chunks(SLACK_MAX_BLOCKS_PER_MESSAGE)
        .map(|group| {
            group
                .iter()
                .map(|chunk| json!({ "type": "markdown", "text": chunk }))
                .collect()
        })
        .collect()
}

/// Cap on the `text` notification fallback of a Markdown post.
///
/// `text` is not rendered when `blocks` are present: it is what Slack shows in
/// push notifications and the channel list, so it only needs enough to be
/// recognisable, and Slack rejects the whole post if it runs long.
pub const SLACK_TEXT_FALLBACK_LIMIT: usize = 3_000;

/// The `chat.postMessage` payloads for one agent reply: normally one.
///
/// Each carries its Markdown in `markdown` blocks and a short `text` fallback
/// for notifications. Callers add `metadata` or other fields they own.
pub fn reply_payloads(channel: &str, thread_ts: &str, text: &str) -> Vec<Value> {
    let fallback = neutralize_broadcasts(truncate_chars(text, SLACK_TEXT_FALLBACK_LIMIT));
    markdown_block_groups(text)
        .into_iter()
        .map(|blocks| {
            let mut payload = json!({
                "channel": channel,
                // Notification fallback only; `blocks` is what renders.
                "text": fallback,
                "blocks": blocks,
            });
            if !thread_ts.is_empty() {
                payload["thread_ts"] = Value::String(thread_ts.to_string());
            }
            payload
        })
        .collect()
}

/// Longest `text` Slack keeps on a message; it truncates beyond this.
const SLACK_TEXT_MAX_CHARS: usize = 40_000;

/// Rebuild a rejected Markdown post as plain `text`.
///
/// Returns `Some` only for a `chat.postMessage` or `chat.update` payload whose
/// blocks are all `markdown` blocks, so hand-built Block Kit (approval cards,
/// forms) is never silently flattened. The text is the blocks' Markdown joined
/// back together; it renders as `mrkdwn`, so formatting degrades, but the
/// person gets the reply instead of nothing.
pub fn plain_text_retry(method: &str, payload: &Value) -> Option<Value> {
    if !matches!(method, "chat.postMessage" | "chat.update") {
        return None;
    }
    let blocks = payload.get("blocks")?.as_array()?;
    if blocks.is_empty() {
        return None;
    }
    let mut text = String::new();
    for block in blocks {
        if block.get("type")?.as_str()? != "markdown" {
            return None;
        }
        text.push_str(block.get("text")?.as_str()?);
    }
    let mut retry = payload.clone();
    let object = retry.as_object_mut()?;
    object.remove("blocks");
    object.insert(
        "text".to_string(),
        Value::String(truncate_chars(&text, SLACK_TEXT_MAX_CHARS).to_string()),
    );
    Some(retry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broadcasts_become_plain_text() {
        assert_eq!(
            neutralize_broadcasts("hey <!here> and <!channel|channel>, <!everyone>!"),
            "hey @here and @channel, @everyone!"
        );
    }

    #[test]
    fn other_tokens_are_left_alone() {
        let text = "<@U123> <#C123> <!subteam^S1> <!date^1^{date}|x> <!hereafter> a <! b";
        assert_eq!(neutralize_broadcasts(text), text);
    }

    #[test]
    fn unterminated_broadcast_is_left_alone() {
        assert_eq!(neutralize_broadcasts("<!here|oops"), "<!here|oops");
    }

    #[test]
    fn block_groups_neutralize_broadcasts() {
        let groups = markdown_block_groups("**Hi** <!channel>");
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0][0]["text"], "**Hi** @channel");
        assert_eq!(markdown_block("<!here>")["text"], "@here");
    }

    #[test]
    fn reply_payloads_carry_markdown_and_a_neutral_fallback() {
        let payloads = reply_payloads("C1", "1.2", "## Done <!here>");
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0]["blocks"][0]["type"], "markdown");
        assert_eq!(payloads[0]["blocks"][0]["text"], "## Done @here");
        assert_eq!(payloads[0]["text"], "## Done @here");
        assert_eq!(payloads[0]["thread_ts"], "1.2");
        assert!(reply_payloads("C1", "", "x")[0].get("thread_ts").is_none());
    }

    #[test]
    fn plain_text_retry_joins_markdown_blocks() {
        let payload = json!({
            "channel": "C1",
            "thread_ts": "1.2",
            "text": "fallback",
            "blocks": [
                { "type": "markdown", "text": "part one\n" },
                { "type": "markdown", "text": "part two" },
            ],
        });
        let retry = plain_text_retry("chat.postMessage", &payload).expect("retry");
        assert_eq!(retry["text"], "part one\npart two");
        assert!(retry.get("blocks").is_none());
        assert_eq!(retry["thread_ts"], "1.2");
    }

    #[test]
    fn plain_text_retry_skips_block_kit_and_other_methods() {
        let card =
            json!({ "blocks": [{ "type": "section", "text": { "type": "mrkdwn", "text": "x" } }] });
        assert!(plain_text_retry("chat.postMessage", &card).is_none());
        let md = json!({ "blocks": [{ "type": "markdown", "text": "x" }] });
        assert!(plain_text_retry("reactions.add", &md).is_none());
        assert!(plain_text_retry("chat.postMessage", &json!({ "text": "x" })).is_none());
    }
}
