//! The control protocol: one JSON request line per connection, then one or
//! more JSON response lines.
//!
//! The shapes are serde types so a host in another language can speak the
//! protocol too; the Rust [`ControlClient`](super::ControlClient) is a thin
//! reader and writer over them.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{SendDisposition, SessionEvent, SessionEventKind, Turn};

/// The single request line a connection opens with.
#[derive(Clone, Serialize, Deserialize)]
pub struct ControlRequest {
    /// The bearer token from the server's token file.
    pub token: String,
    /// What to do.
    pub command: ControlCommand,
}

// THREAT[TM-AUTH-040]: never print the bearer token through `{:?}`.
impl std::fmt::Debug for ControlRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlRequest")
            .field("token", &"<redacted>")
            .field("command", &self.command)
            .finish()
    }
}

/// One control operation.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ControlCommand {
    /// List the registered sessions.
    List,
    /// Send a user message: starts a turn on an idle session, steers a busy one.
    Send {
        /// Session id, or a unique label.
        session: String,
        /// Message text.
        text: String,
        /// Keep the connection open until the accepting turn ends and return it.
        #[serde(default)]
        wait: bool,
    },
    /// Cancel every unfinished tracked turn of the session.
    Cancel {
        /// Session id, or a unique label.
        session: String,
    },
    /// Stream the session's output until the client disconnects.
    Attach {
        /// Session id, or a unique label.
        session: String,
        /// Replay durable events after this sequence first. `None` streams
        /// live events only; `Some(0)` replays the whole log.
        #[serde(default)]
        after: Option<i32>,
    },
}

/// One response line.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ControlResponse {
    /// Answer to [`ControlCommand::List`].
    Sessions {
        /// The registered sessions.
        sessions: Vec<ControlSessionInfo>,
    },
    /// The message was accepted.
    Sent(ControlSent),
    /// The accepting turn ended (only after `send` with `wait`).
    Turn(ControlTurn),
    /// The cancellation request reached the session's unfinished turns.
    Cancelled {
        /// Identities of the cancelled turns, as `list` reports them.
        turns: Vec<String>,
    },
    /// The attach stream is open; events follow.
    Attached {
        /// The resolved session id.
        session: String,
    },
    /// One projected session event.
    Event(ControlEvent),
    /// The server ended an attach stream.
    Closed {
        /// Why: `revoked`, `shutdown` or `session_closed`.
        reason: String,
    },
    /// The request failed. The connection closes after this line.
    Error {
        /// Stable code: `unauthorized`, `bad_request`, `not_found`,
        /// `ambiguous`, `no_active_turn`, `session_closed`, `busy` or `failed`.
        code: String,
        /// Human-readable detail.
        message: String,
    },
}

/// A registered session as `list` reports it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlSessionInfo {
    /// The session id.
    pub id: String,
    /// The label the host registered it under.
    pub label: String,
    /// Identities of the tracked turns that have not finished, oldest first.
    /// An identity is fixed when the message is sent: the turn id when
    /// [`Session::send`](crate::Session::send) already returned one, and the
    /// accepted message's id otherwise (for example on the default Engine's
    /// durable admission). It does not change once the turn is assigned.
    /// Empty when the session is idle.
    pub active_turns: Vec<String>,
}

/// Receipt for an accepted message.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ControlSent {
    /// The accepted user message.
    pub message_id: String,
    /// The accepting turn, when already assigned.
    pub turn_id: Option<String>,
    /// `started`, `steered` or `pending`.
    pub disposition: String,
}

impl ControlSent {
    pub(super) fn new(
        message_id: String,
        turn_id: Option<String>,
        disposition: SendDisposition,
    ) -> Self {
        let disposition = match disposition {
            SendDisposition::Started => "started",
            SendDisposition::Steered => "steered",
            _ => "pending",
        };
        Self {
            message_id,
            turn_id,
            disposition: disposition.to_owned(),
        }
    }
}

/// A finished turn.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ControlTurn {
    /// The turn id.
    pub turn_id: String,
    /// Final text response.
    pub response: String,
    /// `end_turn`, `cancelled`, `error`, ...
    pub stop_reason: String,
    /// Whether the turn completed without an unrecoverable failure.
    pub success: bool,
    /// Failure detail when `success` is false.
    pub error: Option<String>,
}

impl From<Turn> for ControlTurn {
    fn from(turn: Turn) -> Self {
        let stop_reason = serde_json::to_value(turn.stop_reason)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_default();
        Self {
            turn_id: turn.turn_id,
            response: turn.response,
            stop_reason,
            success: turn.success,
            error: turn.error,
        }
    }
}

/// A session event projected for a terminal.
///
/// Carries only what an attached renderer needs: the conversation's text, tool
/// names and turn boundaries. Tool arguments and results are not forwarded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ControlEvent {
    /// Durable sequence; `None` for ephemeral deltas. Resume an attach from
    /// the last sequence seen.
    pub sequence: Option<i32>,
    /// Canonical event type, for example `turn.started`.
    pub event_type: String,
    /// The rendered meaning.
    #[serde(flatten)]
    pub kind: ControlEventKind,
}

/// What a [`ControlEvent`] says.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ControlEventKind {
    /// A user message entered the conversation, from any sender.
    UserMessage {
        /// Its text.
        text: String,
    },
    /// A chunk of streaming assistant text.
    TextDelta {
        /// The new text.
        text: String,
    },
    /// An assistant message completed.
    AssistantMessage {
        /// Its full text.
        text: String,
        /// Whether this stream already delivered it as deltas.
        streamed: bool,
    },
    /// Streamed output was replaced, for example by a guardrail.
    OutputReplaced {
        /// The replacement to show.
        text: String,
    },
    /// The agent sent a message with `send_message`.
    MessageSent {
        /// Its text.
        text: String,
    },
    /// A tool call started.
    ToolStarted {
        /// The tool.
        name: String,
    },
    /// A tool call finished.
    ToolCompleted {
        /// The tool.
        name: String,
        /// Whether it succeeded.
        success: bool,
    },
    /// A turn began.
    TurnStarted,
    /// A turn finished successfully.
    TurnCompleted,
    /// A turn failed.
    TurnFailed {
        /// Failure detail.
        error: String,
    },
    /// A turn was cancelled.
    TurnCancelled,
    /// The stream fell behind and dropped events. Re-attach with the last
    /// durable sequence to recover.
    Lagged {
        /// Events dropped.
        missed: u64,
    },
    /// A durable event with nothing to render; kept so the client's cursor
    /// advances.
    #[serde(other)]
    Other,
}

impl ControlEvent {
    /// Text to print for this event, if any. Deltas carry no newline; the
    /// completed message adds one.
    pub fn render(&self) -> Option<String> {
        Some(match &self.kind {
            ControlEventKind::UserMessage { text } => format!("> {text}\n"),
            ControlEventKind::TextDelta { text } => text.clone(),
            ControlEventKind::AssistantMessage { streamed: true, .. } => "\n".to_owned(),
            ControlEventKind::AssistantMessage { text, .. } => format!("{text}\n"),
            ControlEventKind::OutputReplaced { text } => format!("\n[replaced] {text}\n"),
            ControlEventKind::MessageSent { text } => format!("{text}\n"),
            ControlEventKind::ToolStarted { name } => format!("[tool {name}]\n"),
            ControlEventKind::ToolCompleted {
                name,
                success: false,
            } => format!("[tool {name} failed]\n"),
            ControlEventKind::TurnFailed { error } => format!("[turn failed: {error}]\n"),
            ControlEventKind::TurnCancelled => "[turn cancelled]\n".to_owned(),
            ControlEventKind::Lagged { missed } => format!("[missed {missed} events]\n"),
            _ => return None,
        })
    }
}

/// Per-stream projection state: whether the current output already streamed
/// as deltas, so its completion is not printed twice.
#[derive(Default)]
pub(super) struct Projector {
    streamed: bool,
}

impl Projector {
    pub(super) fn project(&mut self, event: &SessionEvent) -> Option<ControlEvent> {
        let sequence = event.sequence();
        let kind = match &event.kind {
            SessionEventKind::InputMessage { .. } => ControlEventKind::UserMessage {
                text: message_text(event.canonical_json()),
            },
            SessionEventKind::OutputStarted { .. } => {
                self.streamed = false;
                ControlEventKind::Other
            }
            SessionEventKind::TextDelta { delta } => {
                self.streamed = true;
                ControlEventKind::TextDelta {
                    text: delta.clone(),
                }
            }
            SessionEventKind::OutputCompleted { .. } => {
                let streamed = std::mem::take(&mut self.streamed);
                ControlEventKind::AssistantMessage {
                    text: message_text(event.canonical_json()),
                    streamed,
                }
            }
            SessionEventKind::OutputReplaced { replacement, .. } => {
                ControlEventKind::OutputReplaced {
                    text: replacement.clone(),
                }
            }
            SessionEventKind::MessageSent { text, .. } => {
                ControlEventKind::MessageSent { text: text.clone() }
            }
            SessionEventKind::ToolStarted { tool_name, .. } => ControlEventKind::ToolStarted {
                name: tool_name.clone(),
            },
            SessionEventKind::ToolCompleted {
                tool_name, success, ..
            } => ControlEventKind::ToolCompleted {
                name: tool_name.clone(),
                success: *success,
            },
            SessionEventKind::TurnStarted => ControlEventKind::TurnStarted,
            SessionEventKind::TurnCompleted => ControlEventKind::TurnCompleted,
            SessionEventKind::TurnFailed { error } => ControlEventKind::TurnFailed {
                error: error.clone(),
            },
            SessionEventKind::TurnCancelled => ControlEventKind::TurnCancelled,
            _ => ControlEventKind::Other,
        };
        // Ephemeral events with nothing to render (reasoning deltas, progress)
        // would only cost bandwidth.
        if kind == ControlEventKind::Other && sequence.is_none() {
            return None;
        }
        Some(ControlEvent {
            sequence,
            event_type: event.event_type().to_owned(),
            kind,
        })
    }
}

/// The text parts of `data.message.content` in a canonical message event.
/// Reasoning and non-text parts are skipped.
fn message_text(canonical: &Value) -> String {
    let parts = canonical
        .pointer("/data/message/content")
        .and_then(Value::as_array);
    let mut text = String::new();
    for part in parts.into_iter().flatten() {
        if part.get("type").and_then(Value::as_str) == Some("text")
            && let Some(chunk) = part.get("text").and_then(Value::as_str)
        {
            text.push_str(chunk);
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_never_shows_the_token() {
        let request = ControlRequest {
            token: "secret-token-value".to_owned(),
            command: ControlCommand::List,
        };
        let printed = format!("{request:?}");
        assert!(!printed.contains("secret-token-value"), "{printed}");
        assert!(printed.contains("redacted"), "{printed}");
    }

    #[test]
    fn unknown_event_kinds_from_a_newer_server_read_as_other() {
        let line = r#"{"sequence":7,"event_type":"x.new","kind":"something_new"}"#;
        let event: ControlEvent = serde_json::from_str(line).unwrap();
        assert_eq!(event.kind, ControlEventKind::Other);
        assert_eq!(event.sequence, Some(7));
        assert_eq!(event.render(), None);
    }

    #[test]
    fn a_streamed_message_is_not_rendered_twice() {
        let event = |kind| ControlEvent {
            sequence: None,
            event_type: String::new(),
            kind,
        };
        let streamed = event(ControlEventKind::AssistantMessage {
            text: "done".to_owned(),
            streamed: true,
        });
        let replayed = event(ControlEventKind::AssistantMessage {
            text: "done".to_owned(),
            streamed: false,
        });
        assert_eq!(streamed.render().as_deref(), Some("\n"));
        assert_eq!(replayed.render().as_deref(), Some("done\n"));
    }
}
