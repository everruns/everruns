//! What the agent said to the conversation.
//!
//! Every surface that shows or returns an agent's answer (web chat, A2A, AG-UI,
//! MCP, FCP, the CLI, subagent results, evals) must agree on which assistant
//! output counts. Each used to decide on its own, and they disagreed: some
//! dropped commentary, some returned the first message, some the last. This
//! module is the one definition.
//!
//! An agent talks in one of two ways, set per agent by [`Communication`]:
//!
//! - `direct`: its assistant text is what it says. A non-commentary agent
//!   message with text is a reply.
//! - `explicit`: its assistant text is private working notes, marked as
//!   commentary when the message is recorded. It talks by calling
//!   [`SEND_MESSAGE_TOOL_NAME`], and every sent message is recorded as a
//!   `conversation.message` event.
//!
//! The readers below hold both, so a surface never needs to know which mode an
//! agent uses: commentary never counts, and sent messages always do..

use serde::{Deserialize, Serialize};

use crate::execution_phase::ExecutionPhase;

use super::message::{ContentPart, RuntimeMessage, RuntimeMessageRole};

mod tools;

pub use tools::{
    ConversationSender, ConversationSenderExt, MAX_MESSAGE_CHARS, NO_REPLY_TOOL_NAME, NoReplyTool,
    SEND_MESSAGE_TOOL_NAME, SendMessageTool, apply_explicit_communication, sent_message,
};

/// How an agent talks to the people in its conversations. The agent owns this
/// setting; every surface (web chat, API, Slack, A2A, AG-UI, MCP) honors it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum Communication {
    /// Assistant text is the reply, shown as the agent writes it.
    #[default]
    Direct,
    /// Assistant text is private working notes. The agent talks only through
    /// `send_message` (and `no_reply` when it decides not to answer).
    Explicit,
}

impl Communication {
    /// Whether this is the default, for `skip_serializing_if`.
    pub fn is_direct(&self) -> bool {
        matches!(self, Self::Direct)
    }

    /// Whether the agent talks through tools.
    pub fn is_explicit(&self) -> bool {
        matches!(self, Self::Explicit)
    }

    /// Wire and storage spelling.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Explicit => "explicit",
        }
    }

    /// Parse a stored or wire value. Absent or unknown reads as `direct`.
    pub fn from_wire(value: Option<&str>) -> Self {
        value.and_then(Self::from_str_opt).unwrap_or_default()
    }

    /// Parse the wire and storage spelling.
    pub fn from_str_opt(value: &str) -> Option<Self> {
        match value {
            "direct" => Some(Self::Direct),
            "explicit" => Some(Self::Explicit),
            _ => None,
        }
    }
}

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

    /// Record messages the agent sent with `send_message`. A sent message is
    /// never a preamble: the agent chose to send it.
    pub fn extend_sent(&mut self, sent: impl IntoIterator<Item = String>) {
        self.said.extend(sent.into_iter().map(|text| SaidMessage {
            text,
            with_tool_calls: false,
        }));
    }
}

/// What one event said to the conversation: the text of a non-commentary
/// `output.message.completed`, or of a `conversation.message`. `None` for every
/// other event. The one entry point for readers that walk an event stream.
pub fn said_in_event(event_type: &str, data: &serde_json::Value) -> Option<String> {
    match event_type {
        crate::runtime::events::OUTPUT_MESSAGE_COMPLETED => said_text_in_event(data),
        crate::runtime::events::CONVERSATION_MESSAGE => data
            .get("text")
            .and_then(|text| text.as_str())
            .filter(|text| !text.trim().is_empty())
            .map(str::to_owned),
        _ => None,
    }
}

/// [`said_in_event`] as a reply candidate. A sent message is never a preamble:
/// the agent chose to send it.
pub fn said_message_in_event(event_type: &str, data: &serde_json::Value) -> Option<SaidMessage> {
    let text = said_in_event(event_type, data)?;
    let with_tool_calls = event_type == crate::runtime::events::OUTPUT_MESSAGE_COMPLETED
        && data
            .get("message")
            .and_then(|message| message.get("content"))
            .and_then(|content| content.as_array())
            .is_some_and(|parts| {
                parts
                    .iter()
                    .any(|part| part.get("type").and_then(|t| t.as_str()) == Some("tool_call"))
            });
    Some(SaidMessage {
        text,
        with_tool_calls,
    })
}

/// The name Slack's reply tool had before explicit communication replaced it.
/// Stored transcripts still carry it, and its calls read as sent messages.
const LEGACY_SEND_MESSAGE_TOOL_NAME: &str = "channel_post_message";

/// Everything said in a stored transcript, in order: agent text that is not
/// commentary, plus every `send_message` call whose result reports delivery.
/// For readers that hold messages rather than events (subagent results,
/// handoffs, coordination digests).
pub fn said_in_transcript(messages: &[RuntimeMessage]) -> Vec<SaidMessage> {
    said_per_message(messages).into_iter().flatten().collect()
}

/// [`said_in_transcript`] grouped by message: entry `i` holds what message `i`
/// said (empty for user and tool messages, commentary, and calls that were not
/// delivered). For readers that keep one row per agent message.
pub fn said_per_message(messages: &[RuntimeMessage]) -> Vec<Vec<SaidMessage>> {
    let delivered: std::collections::HashSet<&str> = messages
        .iter()
        .filter(|message| message.role == RuntimeMessageRole::ToolResult)
        .flat_map(|message| &message.content)
        .filter_map(|part| match part {
            ContentPart::ToolResult(result)
                if result.error.is_none()
                    && result.result.as_ref().is_some_and(|value| {
                        // `delivered` is the legacy Slack reply tool's receipt.
                        ["sent", "delivered"]
                            .iter()
                            .any(|key| value.get(key).and_then(|v| v.as_bool()) == Some(true))
                    }) =>
            {
                Some(result.tool_call_id.as_str())
            }
            _ => None,
        })
        .collect();
    messages
        .iter()
        .map(|message| {
            let mut said: Vec<SaidMessage> =
                SaidMessage::from_message(message).into_iter().collect();
            if message.role != RuntimeMessageRole::Agent {
                return said;
            }
            for part in &message.content {
                if let ContentPart::ToolCall(call) = part
                    && (call.name == SEND_MESSAGE_TOOL_NAME
                        || call.name == LEGACY_SEND_MESSAGE_TOOL_NAME)
                    && delivered.contains(call.id.as_str())
                    && let Some(text) = call.arguments.get("text").and_then(|t| t.as_str())
                    && !text.trim().is_empty()
                {
                    said.push(SaidMessage {
                        text: text.to_owned(),
                        with_tool_calls: false,
                    });
                }
            }
            said
        })
        .collect()
}

/// What each message in a transcript said, as one text per message (`None`
/// when it said nothing). User messages read as their text; agent messages as
/// [`said_per_message`] joined by blank lines; tool messages never speak.
pub fn transcript_lines(messages: &[RuntimeMessage]) -> Vec<Option<String>> {
    said_per_message(messages)
        .into_iter()
        .zip(messages)
        .map(|(said, message)| {
            let text = match message.role {
                RuntimeMessageRole::User => spoken_text(&message.content),
                RuntimeMessageRole::Agent => said
                    .into_iter()
                    .map(|said| said.text)
                    .collect::<Vec<_>>()
                    .join("\n\n"),
                _ => String::new(),
            };
            (!text.trim().is_empty()).then_some(text)
        })
        .collect()
}

#[cfg(test)]
mod tests;
