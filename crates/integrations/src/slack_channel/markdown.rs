//! Everruns mention policy and compatibility names. Channelkit owns Markdown
//! rendering, grouping, fallback, and provider bounds; output is never truncated.

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
    // Compatibility for consumers of the former helper. Channelkit owns the
    // bounded fence-aware renderer; malformed fences retain literal text.
    channelkit::split_markdown(text, limit)
        .or_else(|_| channelkit::split_text(text, limit))
        .unwrap_or_default()
}

/// Rewrite broadcast mentions to plain text, so agent output cannot page a
/// whole channel.
///
/// `<!here>`, `<!channel>` and `<!everyone>` (with or without a `|label`)
/// become `@here`, `@channel` and `@everyone`, which Slack shows as text in
/// both `markdown` blocks and `mrkdwn`. User-group broadcasts are neutralized
/// too. Direct user/channel references and date tokens retain their meaning.
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
        if let Some(group) = tail.strip_prefix("subteam^")
            && let Some(end) = group.find('>')
        {
            let (id, label) = group[..end]
                .split_once('|')
                .map_or((&group[..end], None), |(id, label)| (id, Some(label)));
            out.push('@');
            // Labels are untrusted too: never create a nested mention while
            // replacing the outer group token.
            out.extend(
                label
                    .unwrap_or(id)
                    .trim_start_matches('@')
                    .chars()
                    .filter(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ' ')),
            );
            rest = &group[end + 1..];
            continue;
        }
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

pub const SLACK_TEXT_FALLBACK_LIMIT: usize = 3000;

/// Provider validation rejects oversized output before any write; it is never truncated.
pub fn reply_payloads(
    channel: &str,
    thread_ts: &str,
    text: &str,
) -> Result<Vec<Value>, super::web_api::SlackApiError> {
    let mut payloads = channelkit_slack::markdown_payloads(&neutralize_broadcasts(text))
        .map_err(super::web_api::provider_error)?;
    for payload in &mut payloads {
        payload["channel"] = json!(channel);
        if !thread_ts.is_empty() {
            payload["thread_ts"] = json!(thread_ts);
        }
    }
    Ok(payloads)
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
        let text = "<@U123> <#C123> <!date^1^{date}|x> <!hereafter> a <! b";
        assert_eq!(neutralize_broadcasts(text), text);
    }

    #[test]
    fn unterminated_broadcast_is_left_alone() {
        assert_eq!(neutralize_broadcasts("<!here|oops"), "<!here|oops");
    }

    #[test]
    fn reply_payloads_carry_markdown_and_a_neutral_fallback() {
        let payloads = reply_payloads("C1", "1.2", "## Done <!here>").unwrap();
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0]["blocks"][0]["type"], "markdown");
        assert_eq!(payloads[0]["blocks"][0]["text"], "## Done @here");
        assert_eq!(payloads[0]["text"], "## Done @here");
        assert_eq!(payloads[0]["thread_ts"], "1.2");
        assert!(
            reply_payloads("C1", "", "x").unwrap()[0]
                .get("thread_ts")
                .is_none()
        );
    }
}

#[cfg(test)]
mod group_policy_tests {
    use super::*;
    #[test]
    fn group_notifications_and_nested_tokens_are_neutralized() {
        assert_eq!(
            neutralize_broadcasts("<!subteam^S1> <!subteam^S2|@eng-team>"),
            "@S1 @eng-team"
        );
        let text = neutralize_broadcasts("<!subteam^S1|<!here>>");
        assert!(!text.contains("<!"));
        assert_eq!(
            neutralize_broadcasts("<!subteam^S1|unfinished"),
            "<!subteam^S1|unfinished"
        );
    }
}
