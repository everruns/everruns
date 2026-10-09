//! What the agent said to the conversation.
//!
//! Every surface that shows or returns an agent's answer (web chat, A2A, AG-UI,
//! MCP, FCP, the CLI, subagent results, evals) must agree on which assistant
//! output counts. Each used to decide on its own, and they disagreed: some
//! dropped commentary, some returned the first message, some the last. This
//! module is the one definition.
//!
//! Today an agent talks by writing assistant text, so "said" means a
//! non-commentary agent message with text. Explicit communication will add a
//! second source, messages sent through `send_message`; the readers below are
//! where that lands.

use crate::execution_phase::ExecutionPhase;

use super::message::{ContentPart, RuntimeMessage, RuntimeMessageRole};

/// Joins a message's non-empty text parts with newlines, dropping tool calls,
/// tool results, images, reasoning and provider-opaque parts.
pub fn spoken_text(parts: &[ContentPart]) -> String {
    parts
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text(text) if !text.text.is_empty() => Some(text.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether an assistant message is working commentary rather than something
/// said to the person. Commentary stays in the transcript and never counts as
/// an answer.
pub fn is_commentary(phase: Option<ExecutionPhase>) -> bool {
    matches!(phase, Some(ExecutionPhase::Commentary))
}

/// The text an agent message says to the conversation, or `None` when it is
/// not an agent message, is commentary, or carries no text.
pub fn said_text(message: &RuntimeMessage) -> Option<String> {
    if message.role != RuntimeMessageRole::Agent {
        return None;
    }
    said_text_with_phase(message.phase, &message.content)
}

/// [`said_text`] for callers that hold the parts and phase separately, such as
/// an `output.message.completed` payload already known to be from the agent.
pub fn said_text_with_phase(
    phase: Option<ExecutionPhase>,
    parts: &[ContentPart],
) -> Option<String> {
    if is_commentary(phase) {
        return None;
    }
    let text = spoken_text(parts);
    (!text.trim().is_empty()).then_some(text)
}

/// [`said_text`] over the JSON of an `output.message.completed` payload, for
/// readers that hold raw event data. Lenient on purpose: it needs only
/// `message.content` (and `message.phase` when present), so partial fixtures
/// and older stored events read the same way typed messages do.
pub fn said_text_in_event(data: &serde_json::Value) -> Option<String> {
    let message = data.get("message")?;
    let phase = message
        .get("phase")
        .and_then(|phase| serde_json::from_value::<ExecutionPhase>(phase.clone()).ok());
    if is_commentary(phase) {
        return None;
    }
    let text = message
        .get("content")?
        .as_array()?
        .iter()
        .filter(|part| part.get("type").and_then(|t| t.as_str()) == Some("text"))
        .filter_map(|part| part.get("text").and_then(|t| t.as_str()))
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    (!text.trim().is_empty()).then_some(text)
}

/// One agent message as a reply candidate: what it said, and whether it also
/// carried tool calls (text next to a tool call is usually a preamble such as
/// "let me check", not the answer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaidMessage {
    /// Text said to the conversation.
    pub text: String,
    /// Whether the same message also requested tool calls.
    pub with_tool_calls: bool,
}

impl SaidMessage {
    /// The candidate for an agent message, or `None` when it said nothing.
    pub fn from_message(message: &RuntimeMessage) -> Option<Self> {
        Some(Self {
            text: said_text(message)?,
            with_tool_calls: message.has_tool_calls(),
        })
    }
}

/// The reply of a turn or run: the last thing the agent said that was not a
/// preamble to a tool call, falling back to the last thing it said at all
/// (a turn cut short mid-tool still reports its last words).
pub fn final_reply<I>(said: I) -> Option<String>
where
    I: IntoIterator<Item = SaidMessage>,
{
    let mut last_any = None;
    let mut last_plain = None;
    for message in said {
        if !message.with_tool_calls {
            last_plain = Some(message.text.clone());
        }
        last_any = Some(message.text);
    }
    last_plain.or(last_any)
}

/// What an agent said over one turn, folded into the turn's reply with
/// [`final_reply`]. Hosts push as the turn runs and take the reply at the end.
#[derive(Debug, Clone, Default)]
pub struct TurnReply {
    said: Vec<SaidMessage>,
}

impl TurnReply {
    /// Record one reasoning step's text. Blank text says nothing.
    pub fn push_text(&mut self, text: &str, with_tool_calls: bool) {
        if !text.trim().is_empty() {
            self.said.push(SaidMessage {
                text: text.to_owned(),
                with_tool_calls,
            });
        }
    }

    /// The turn's reply so far, empty when nothing was said. Resets the turn.
    pub fn take(&mut self) -> String {
        final_reply(std::mem::take(&mut self.said)).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests;
