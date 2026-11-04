// Reaction notes: how a person's reactions reach the agent.
//
// Design (knowledge/execution/message-reactions.md):
// - Reactions are not typed messages, so the agent never gets one turn per
//   emoji. Reactions made while the session is idle wait; the next input
//   message (whoever wrote it) carries them under the reserved
//   `everruns_reactions` metadata key, and the model reads one rendered note
//   before that message's text.
// - When a reaction should reach the agent now (the agent is mid-turn, or the
//   agent's Reactions capability says "start a turn"), the platform writes a
//   user-role message with origin kind `reaction` whose text is the same note.
// - The note is rendered from stored metadata, so it reads the same on every
//   replay and the prompt cache holds.
// - `through_sequence` is the last reaction event the note covers. The next
//   note starts after it, so a reaction is told at most once.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Message metadata key carrying the reactions told with that message.
/// Reserved: client-supplied metadata loses this key.
pub const MESSAGE_REACTIONS_METADATA_KEY: &str = "everruns_reactions";
/// Label on the first line of every note, so the model never takes a reaction
/// for something the person typed.
pub const REACTION_NOTE_LABEL: &str = "[Reactions, not typed messages]";
/// Longest excerpt of the reacted message quoted in a note, in characters.
pub const REACTION_EXCERPT_CHARS: usize = 60;

/// One reaction change told to the agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReactionNoteItem {
    /// Message the reaction is on.
    pub message_id: String,
    pub emoji: String,
    /// Display name of whoever reacted.
    pub actor_name: String,
    /// The reaction was taken off (after the agent had been told of it).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub removed: bool,
    /// The reacted message was the agent's own reply.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub on_agent_message: bool,
    /// Start of the reacted message's text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
}

/// The reactions told with one message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReactionNote {
    /// Sequence of the last reaction event this note covers.
    pub through_sequence: i64,
    pub items: Vec<ReactionNoteItem>,
}

impl ReactionNote {
    /// Read the note from message metadata.
    pub fn from_message_metadata(metadata: Option<&HashMap<String, Value>>) -> Option<Self> {
        let value = metadata?.get(MESSAGE_REACTIONS_METADATA_KEY)?;
        serde_json::from_value(value.clone()).ok()
    }

    /// Write the note into message metadata.
    pub fn write(&self, metadata: &mut HashMap<String, Value>) {
        if let Ok(value) = serde_json::to_value(self) {
            metadata.insert(MESSAGE_REACTIONS_METADATA_KEY.to_string(), value);
        }
    }

    /// The text the model reads: the label, then one line per reacted
    /// message, in the order the messages were first reacted to.
    pub fn render(&self) -> String {
        let mut order: Vec<&str> = Vec::new();
        let mut by_message: HashMap<&str, Vec<&ReactionNoteItem>> = HashMap::new();
        for item in &self.items {
            if !by_message.contains_key(item.message_id.as_str()) {
                order.push(&item.message_id);
            }
            by_message.entry(&item.message_id).or_default().push(item);
        }
        let mut lines = vec![REACTION_NOTE_LABEL.to_string()];
        for message_id in order {
            let items = &by_message[message_id];
            let parts: Vec<String> = items
                .iter()
                .map(|item| {
                    let verb = if item.removed { "took back" } else { "reacted" };
                    format!("{} {verb} {}", item.actor_name, item.emoji)
                })
                .collect();
            let first = items[0];
            let target = if first.on_agent_message {
                "your reply"
            } else {
                "the message"
            };
            let excerpt = first
                .excerpt
                .as_deref()
                .map(|text| format!(" \"{text}\""))
                .unwrap_or_default();
            lines.push(format!(
                "{} on {target}{excerpt} ({message_id})",
                join_and(&parts)
            ));
        }
        lines.join("\n")
    }
}

/// Start of a message's text for a note: first non-empty line, cut to
/// [`REACTION_EXCERPT_CHARS`] with an ellipsis.
pub fn reaction_excerpt(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    let mut excerpt: String = line.chars().take(REACTION_EXCERPT_CHARS).collect();
    if line.chars().count() > REACTION_EXCERPT_CHARS {
        excerpt = excerpt.trim_end().to_string();
        excerpt.push('…');
    }
    // Quotes would end the quoted excerpt early in the note.
    Some(excerpt.replace('"', "'"))
}

fn join_and(parts: &[String]) -> String {
    match parts {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(actor: &str, emoji: &str, message: &str) -> ReactionNoteItem {
        ReactionNoteItem {
            message_id: message.into(),
            emoji: emoji.into(),
            actor_name: actor.into(),
            removed: false,
            on_agent_message: true,
            excerpt: Some("Deployed 0.50.1 to production.".into()),
        }
    }

    #[test]
    fn a_note_groups_reactions_by_message() {
        let mut removed = item("Bob", "❤️", "message_2");
        removed.removed = true;
        removed.on_agent_message = false;
        removed.excerpt = None;
        let note = ReactionNote {
            through_sequence: 42,
            items: vec![
                item("Mykhailo", "👍", "message_1"),
                removed,
                item("Alice", "🎉", "message_1"),
            ],
        };
        assert_eq!(
            note.render(),
            "[Reactions, not typed messages]\n\
             Mykhailo reacted 👍 and Alice reacted 🎉 on your reply \"Deployed 0.50.1 to production.\" (message_1)\n\
             Bob took back ❤️ on the message (message_2)"
        );
    }

    #[test]
    fn the_note_round_trips_through_metadata() {
        let note = ReactionNote {
            through_sequence: 7,
            items: vec![item("A", "👍", "message_1")],
        };
        let mut metadata = HashMap::new();
        note.write(&mut metadata);
        assert_eq!(
            ReactionNote::from_message_metadata(Some(&metadata)),
            Some(note)
        );
        assert_eq!(ReactionNote::from_message_metadata(None), None);
    }

    #[test]
    fn excerpts_are_one_short_line_without_quotes() {
        assert_eq!(
            reaction_excerpt("\n  Say \"hi\"\nmore"),
            Some("Say 'hi'".into())
        );
        let long = reaction_excerpt(&"x".repeat(100)).unwrap();
        assert_eq!(long.chars().count(), REACTION_EXCERPT_CHARS + 1);
        assert_eq!(reaction_excerpt("  \n"), None);
    }
}
