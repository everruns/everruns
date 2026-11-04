// Reactions on Slack messages, both ways (knowledge/execution/message-reactions.md).
//
// Decisions:
// - A Slack message is keyed `{channel}:{ts}`, which is what a Slack
//   reaction event names (`item.channel`, `item.ts`).
// - Everruns stores a reaction as one Unicode emoji or a custom `:name:`;
//   Slack names every reaction. The table maps the common ones. A Slack name
//   the table lacks arrives as `:name:`, so custom workspace emoji keep their
//   name; a Unicode emoji the table lacks is not mirrored to Slack, since
//   guessing a name could put a different emoji on the message.
// - Skin tones (`thumbsup::skin-tone-2`) are dropped: a reaction is about
//   what it says, and one 👍 per person is what the agent should hear.

use super::*;

/// Unicode emoji and the Slack name for it. The first entry for an emoji is
/// the name posted; later entries are aliases Slack may report.
const SLACK_REACTION_NAMES: &[(&str, &str)] = &[
    ("👍", "thumbsup"),
    ("👍", "+1"),
    ("👎", "thumbsdown"),
    ("👎", "-1"),
    ("❤️", "heart"),
    ("😂", "joy"),
    ("😄", "smile"),
    ("😀", "grinning"),
    ("🙂", "slightly_smiling_face"),
    ("😊", "blush"),
    ("😉", "wink"),
    ("😍", "heart_eyes"),
    ("😅", "sweat_smile"),
    ("😬", "grimacing"),
    ("😢", "cry"),
    ("😮", "open_mouth"),
    ("🤔", "thinking_face"),
    ("🤯", "exploding_head"),
    ("🥳", "partying_face"),
    ("🤷", "shrug"),
    ("🎉", "tada"),
    ("🙏", "pray"),
    ("👀", "eyes"),
    ("👏", "clap"),
    ("🙌", "raised_hands"),
    ("👋", "wave"),
    ("👌", "ok_hand"),
    ("💪", "muscle"),
    ("🤝", "handshake"),
    ("✅", "white_check_mark"),
    ("✔️", "heavy_check_mark"),
    ("❌", "x"),
    ("➕", "heavy_plus_sign"),
    ("🔥", "fire"),
    ("🚀", "rocket"),
    ("💯", "100"),
    ("⭐", "star"),
    ("✨", "sparkles"),
    ("💡", "bulb"),
    ("⚠️", "warning"),
    ("🐛", "bug"),
    ("🎯", "dart"),
    ("📌", "pushpin"),
    ("⏳", "hourglass_flowing_sand"),
    ("❗", "exclamation"),
    ("❓", "question"),
    ("💚", "green_heart"),
    ("💙", "blue_heart"),
    ("🟢", "large_green_circle"),
    ("🔴", "red_circle"),
];

/// Presentation selectors do not change which emoji it is.
fn bare(emoji: &str) -> String {
    emoji.chars().filter(|c| *c != '\u{FE0F}').collect()
}

/// The Slack name to post for a stored reaction, or `None` when Slack has no
/// name this side knows.
pub(crate) fn slack_reaction_name(emoji: &str) -> Option<String> {
    if let Some(name) = emoji.strip_prefix(':').and_then(|e| e.strip_suffix(':')) {
        return Some(name.to_string());
    }
    let emoji = bare(emoji);
    SLACK_REACTION_NAMES
        .iter()
        .find(|(unicode, _)| bare(unicode) == emoji)
        .map(|(_, name)| (*name).to_string())
}

/// The stored form of a reaction Slack reported by name.
pub(crate) fn emoji_for_slack_reaction(name: &str) -> String {
    let name = name.split("::").next().unwrap_or(name);
    SLACK_REACTION_NAMES
        .iter()
        .find(|(_, slack)| *slack == name)
        .map(|(unicode, _)| (*unicode).to_string())
        .unwrap_or_else(|| format!(":{name}:"))
}

#[async_trait]
impl ChannelReactionSurface for SlackDeliveryAdapter {
    fn stream_message_key(&self, handle: &str, context: &ChannelDeliveryContext) -> String {
        format!("{}:{handle}", context.channel_id)
    }

    async fn post_keyed(
        &self,
        message: &OutboundChannelMessage,
        context: &ChannelDeliveryContext,
    ) -> Result<Option<String>, ChannelDeliveryResult> {
        match self.post_reply(message, context).await {
            Ok(ts) => Ok(ts.map(|ts| format!("{}:{ts}", context.channel_id))),
            Err(error) => Err(classify_slack_failure(error)),
        }
    }

    async fn react(
        &self,
        message_key: &str,
        emoji: &str,
        add: bool,
        context: &ChannelDeliveryContext,
    ) -> ChannelDeliveryResult {
        let Some((channel, ts)) = message_key.split_once(':') else {
            return ChannelDeliveryResult::PermanentError(format!(
                "not a Slack message key: {message_key}"
            ));
        };
        let Some(name) = slack_reaction_name(emoji) else {
            debug!(%emoji, "No Slack name for the agent's reaction; not mirrored");
            return ChannelDeliveryResult::Ok;
        };
        let method = if add {
            "reactions.add"
        } else {
            "reactions.remove"
        };
        let payload = serde_json::json!({ "channel": channel, "timestamp": ts, "name": name });
        match slack_api_call(&self.api_base, &context.auth_token, method, payload).await {
            Ok(_) => ChannelDeliveryResult::Ok,
            // Already there, or already gone: the agent's intent holds.
            Err(error) if matches!(error.code(), Some("already_reacted" | "no_reaction")) => {
                ChannelDeliveryResult::Ok
            }
            Err(error) => classify_slack_failure(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn names_map_both_ways() {
        assert_eq!(slack_reaction_name("👍").as_deref(), Some("thumbsup"));
        assert_eq!(slack_reaction_name("❤").as_deref(), Some("heart"));
        assert_eq!(
            slack_reaction_name(":party_parrot:").as_deref(),
            Some("party_parrot")
        );
        assert_eq!(slack_reaction_name("🦀"), None);

        assert_eq!(emoji_for_slack_reaction("+1"), "👍");
        assert_eq!(emoji_for_slack_reaction("thumbsup::skin-tone-3"), "👍");
        assert_eq!(emoji_for_slack_reaction("party_parrot"), ":party_parrot:");
    }

    fn context(server: &MockServer) -> (SlackDeliveryAdapter, ChannelDeliveryContext) {
        let adapter = SlackDeliveryAdapter::with_api_base(server.uri());
        let context = ChannelDeliveryContext {
            auth_token: "xoxb-test".into(),
            channel_id: "C1".into(),
            thread_ref: "1700.1".into(),
            extra: HashMap::new(),
        };
        (adapter, context)
    }

    #[tokio::test]
    async fn the_agents_reaction_lands_on_the_slack_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/reactions.add"))
            .and(body_partial_json(
                serde_json::json!({"channel": "C1", "timestamp": "1700.2", "name": "tada"}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/reactions.remove"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"ok": false, "error": "no_reaction"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let (adapter, context) = context(&server);

        let added = adapter.react("C1:1700.2", "🎉", true, &context).await;
        assert!(matches!(added, ChannelDeliveryResult::Ok));
        let removed = adapter.react("C1:1700.2", "🎉", false, &context).await;
        assert!(
            matches!(removed, ChannelDeliveryResult::Ok),
            "already gone is fine"
        );
        let unknown = adapter.react("C1:1700.2", "🦀", true, &context).await;
        assert!(
            matches!(unknown, ChannelDeliveryResult::Ok),
            "skipped, not sent"
        );
    }

    #[tokio::test]
    async fn a_posted_reply_is_keyed_by_its_first_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat.postMessage"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"ok": true, "ts": "1700.9"})),
            )
            .mount(&server)
            .await;
        let (adapter, context) = context(&server);
        let message = OutboundChannelMessage {
            session_id: everruns_contracts::typed_id::SessionId::new(),
            text: "Done.".into(),
            thread_ref: "1700.1".into(),
            correlation_id: Some("msg_1".into()),
        };
        let key = adapter.post_keyed(&message, &context).await.ok().flatten();
        assert_eq!(key.as_deref(), Some("C1:1700.9"));
        assert_eq!(adapter.stream_message_key("1700.5", &context), "C1:1700.5");
    }
}
