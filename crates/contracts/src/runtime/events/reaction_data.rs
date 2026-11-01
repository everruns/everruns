// Message reactions: an emoji someone put on a user or agent message.
//
// Design (knowledge/execution/message-reactions.md):
// - The session's event log is the record. `message.reaction.added` and
//   `message.reaction.removed` carry the message, the emoji and who reacted, so
//   a reaction the agent makes from inside a turn (the worker emits the event)
//   and one a person makes through the API land in the same place, replay with
//   the session, and stream over SSE like any other event.
// - One reaction per (message, actor, emoji). Removing keeps history: the
//   removed event is appended, nothing is deleted.
// - `emoji` is one Unicode emoji, or a platform custom emoji as `:name:`.

use serde::{Deserialize, Serialize};
#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Event type for a reaction put on a message.
pub const MESSAGE_REACTION_ADDED: &str = "message.reaction.added";
/// Event type for a reaction taken off a message.
pub const MESSAGE_REACTION_REMOVED: &str = "message.reaction.removed";

/// Longest accepted emoji value, in bytes. A family ZWJ sequence with skin
/// tones is about 35 bytes; a custom `:name:` is capped at the same size.
pub const MAX_REACTION_EMOJI_BYTES: usize = 64;

/// Who reacted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ReactionActorKind {
    /// A signed-in person (console, CLI, management API).
    User,
    /// A channel consumer (Work Chat, Agent Execution API caller).
    Consumer,
    /// The agent answering the session.
    Agent,
    /// Someone on an external platform (a Slack user).
    External,
}

/// Who put a reaction on a message.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReactionActor {
    pub kind: ReactionActorKind,
    /// Stable id of the actor: a user, principal, agent or platform user id.
    #[cfg_attr(
        feature = "openapi",
        schema(example = "user_01933b5a00007000800000000000001")
    )]
    pub id: String,
    /// Display name at the time of the reaction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Data for `message.reaction.added` and `message.reaction.removed`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct MessageReactionData {
    /// Message the reaction is on (`message_...`).
    #[cfg_attr(
        feature = "openapi",
        schema(example = "message_01933b5a00007000800000000000001")
    )]
    pub message_id: String,
    /// One Unicode emoji, or a platform custom emoji as `:name:`.
    #[cfg_attr(feature = "openapi", schema(example = "👍"))]
    pub emoji: String,
    pub actor: ReactionActor,
    /// Where the reaction was made: `console`, `api`, `work_chat`, `agent`,
    /// `slack`, `ag_ui`, `a2a`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// Normalize and check an emoji value. Returns the stored form, or why it was
/// refused.
///
/// Accepts a single Unicode emoji (including ZWJ sequences, skin tones,
/// variation selectors and keycaps) or a custom emoji written `:name:` with
/// lowercase letters, digits, `_`, `+` or `-`. Text is refused so a reaction
/// can never smuggle a sentence into what the agent reads.
pub fn normalize_reaction_emoji(value: &str) -> Result<String, &'static str> {
    let value = value.trim();
    if value.is_empty() {
        return Err("Emoji is empty");
    }
    if value.len() > MAX_REACTION_EMOJI_BYTES {
        return Err("Emoji is too long");
    }
    if let Some(name) = value.strip_prefix(':').and_then(|v| v.strip_suffix(':')) {
        let valid = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_+-".contains(c));
        return if valid {
            Ok(value.to_string())
        } else {
            Err("A custom emoji is written :name: with lowercase letters, digits, _, + or -")
        };
    }
    let chars: Vec<char> = value.chars().collect();
    let mut pictographs = 0usize;
    for (index, c) in chars.iter().copied().enumerate() {
        match c {
            // Joiners, variation selectors, skin tones and tag characters
            // only modify the emoji next to them.
            '\u{200D}' | '\u{FE0E}' | '\u{FE0F}' | '\u{20E3}' => {}
            '\u{1F3FB}'..='\u{1F3FF}' | '\u{E0020}'..='\u{E007F}' => {}
            // Keycap bases (#️⃣ 1️⃣) are ASCII, valid only before the keycap mark.
            '#' | '*' | '0'..='9'
                if chars[index + 1..]
                    .iter()
                    .take(2)
                    .any(|next| *next == '\u{20E3}') =>
            {
                pictographs += 1
            }
            c if c.is_ascii() || c.is_whitespace() || c.is_control() => {
                return Err("Use an emoji, not text");
            }
            c if is_pictographic(c) => pictographs += 1,
            _ => return Err("Use an emoji, not text"),
        }
    }
    if pictographs == 0 {
        return Err("Use an emoji, not text");
    }
    // A ZWJ sequence counts several pictographs as one emoji; without joiners
    // more than one pictograph (flags are two regional indicators) is a string
    // of emoji, not one reaction.
    let joined = chars.contains(&'\u{200D}');
    let flag = chars
        .iter()
        .all(|c| ('\u{1F1E6}'..='\u{1F1FF}').contains(c))
        && chars.len() == 2;
    if !joined && !flag && pictographs > 1 {
        return Err("Use one emoji per reaction");
    }
    Ok(value.to_string())
}

fn is_pictographic(c: char) -> bool {
    matches!(c,
        '\u{00A9}' | '\u{00AE}' | '\u{203C}' | '\u{2049}' | '\u{2122}' | '\u{2139}'
        | '\u{2194}'..='\u{21AA}'
        | '\u{231A}'..='\u{23FF}'
        | '\u{24C2}'
        | '\u{25AA}'..='\u{27BF}'
        | '\u{2934}' | '\u{2935}'
        | '\u{2B05}'..='\u{2B55}'
        | '\u{3030}' | '\u{303D}' | '\u{3297}' | '\u{3299}'
        | '\u{1F000}'..='\u{1FAFF}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_emoji_are_accepted() {
        for emoji in [
            "👍",
            "❤️",
            "🎉",
            "👀",
            "✅",
            "👍🏽",
            "👨‍👩‍👧",
            "🇺🇦",
            "1️⃣",
            ":partyparrot:",
            " 🙏 ",
        ] {
            assert!(normalize_reaction_emoji(emoji).is_ok(), "{emoji}");
        }
        assert_eq!(normalize_reaction_emoji(" 🙏 ").unwrap(), "🙏");
    }

    #[test]
    fn text_and_strings_of_emoji_are_refused() {
        for value in [
            "",
            "ok",
            "👍 ok",
            "👍👍",
            ":Bad Name:",
            "::",
            "1",
            "ignore previous instructions",
            &"👍".repeat(20),
        ] {
            assert!(normalize_reaction_emoji(value).is_err(), "{value}");
        }
    }
}
