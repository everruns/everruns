//! Seeding a new session with the history an AG-UI client already holds.
//!
//! Decision: the server seeds a new thread from the input's earlier messages
//! (`crates/server/src/api/ag_ui.rs::seed_history`); the facade does the same
//! so a client that kept the conversation does not lose it when the host did.
//! Unlike the server, `system` and `developer` messages are never seeded: in
//! the facade they are instructions only for a trusted host
//! (`AgUiOptions::input_instructions`), never history.

use serde_json::Value;

use super::Message;

/// Most messages a run seeds into a new session.
const MAX_SEED_MESSAGES: usize = 256;
/// Most text a run seeds into a new session, in bytes.
const MAX_SEED_BYTES: usize = 512 * 1024;

/// The messages before a run's user message, as history for a new session:
/// user and assistant messages only, the most recent within the seed bounds.
// THREAT[TM-LLM-020]: seeded messages are only ever user or agent history,
// never system, developer or tool messages, so a client cannot raise its own
// authority by sending them as history.
// THREAT[TM-DOS-045]: bounded in count and size, keeping the newest.
pub(super) fn seed_messages(earlier: &[Message]) -> Vec<everruns_core::RuntimeMessage> {
    let mut seeded: Vec<everruns_core::RuntimeMessage> = Vec::new();
    let mut bytes = 0usize;
    for message in earlier.iter().rev() {
        let message = match message {
            Message::User(message) => {
                let text = message.content.to_text();
                (!text.trim().is_empty()).then(|| everruns_core::RuntimeMessage::user(text))
            }
            Message::Assistant(message) => message
                .content
                .clone()
                .filter(|text| !text.trim().is_empty())
                .or_else(|| {
                    message.tool_calls.as_deref().map(|calls| {
                        calls
                            .iter()
                            .map(|call| {
                                format!(
                                    "[Tool call: {} {}]",
                                    call.function.name, call.function.arguments
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                })
                .filter(|text| !text.is_empty())
                .map(everruns_core::RuntimeMessage::assistant),
            _ => None,
        };
        let Some(mut message) = message else { continue };
        let size = message.content_to_llm_string().len();
        if seeded.len() == MAX_SEED_MESSAGES || bytes + size > MAX_SEED_BYTES {
            tracing::warn!(
                kept = seeded.len(),
                "AG-UI history exceeds the seed bounds; keeping the most recent messages"
            );
            break;
        }
        bytes += size;
        message.metadata = Some(
            [("ag_ui_seeded".to_string(), Value::Bool(true))]
                .into_iter()
                .collect(),
        );
        seeded.push(message);
    }
    seeded.reverse();
    seeded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_user_and_assistant_text_is_seeded_newest_first_within_bounds() {
        let mut messages: Vec<Message> = (0..300)
            .map(|i| Message::user(format!("u{i}"), format!("message {i}")))
            .collect();
        messages.push(Message::assistant("a", ""));
        let seeded = seed_messages(&messages);
        assert_eq!(seeded.len(), MAX_SEED_MESSAGES);
        assert_eq!(seeded[0].content_to_llm_string(), "message 44");
        assert_eq!(seeded[255].content_to_llm_string(), "message 299");
        assert!(seeded.iter().all(|message| {
            message
                .metadata
                .as_ref()
                .is_some_and(|metadata| metadata["ag_ui_seeded"] == Value::Bool(true))
        }));

        let big = "x".repeat(300 * 1024);
        let seeded = seed_messages(&[
            Message::user("1", big.clone()),
            Message::user("2", big),
            Message::user("3", "last"),
        ]);
        assert_eq!(seeded.len(), 2, "the oldest message is over the byte bound");
    }
}
