//! Output-truncation gate: a generation that lost tool calls (cut off at the
//! output limit, or arguments that did not parse) never runs a partial call,
//! and the configured policy decides what the turn does next (see
//! `crate::output_truncation`).
//!
//! State lives in the transcript, not in `TurnState`: a retried generation's
//! assistant message carries [`METADATA_KEY`]. That gives the two things the
//! gate needs without new host plumbing:
//! - the consecutive count, read back from the trailing assistant messages of
//!   the current turn, so it survives durable hand-offs and resets on the next
//!   user input or the first clean generation;
//! - the note to the model, rendered after every stamped exchange on each
//!   request (like the `<facts>` blocks), so it is identical every time and
//!   the cached prefix stays stable. A note rather than a synthetic tool error
//!   because most drivers drop a cut-off call before it has an id to answer.

use std::collections::HashMap;

use serde_json::{Value, json};

use super::generation_outcome::GenerationOutcome;
use crate::engine::CapabilityRef;
use crate::engine::error::{AgentLoopError, LlmErrorKind};
use crate::engine::message::{RuntimeMessage, RuntimeMessageRole};
use crate::output_truncation::{OutputTruncationAction, OutputTruncationConfig};

/// Assistant-message metadata marking a generation the gate retried.
pub(super) const METADATA_KEY: &str = "output_truncation";

/// The gate for one generation: the policy in effect and how many
/// generations right before this one in the turn already lost calls.
#[derive(Debug, Clone, Copy)]
pub(super) struct Gate {
    config: OutputTruncationConfig,
    prior: u32,
}

/// What the gate decided for a finished generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Decision {
    pub action: OutputTruncationAction,
    lost: u32,
    finish_reason: String,
    consecutive: u32,
    policy: &'static str,
}

impl Gate {
    pub(super) fn new(capabilities: &[CapabilityRef], history: &[RuntimeMessage]) -> Self {
        Self {
            config: OutputTruncationConfig::resolve(capabilities),
            prior: consecutive_truncations(history),
        }
    }

    pub(super) fn decide(&self, outcome: &GenerationOutcome) -> Decision {
        Decision {
            action: self.config.decide(
                &outcome.finish_reason,
                outcome.tool_calls_dropped,
                self.prior,
            ),
            lost: outcome.tool_calls_dropped,
            finish_reason: outcome.finish_reason.clone(),
            consecutive: self.prior.saturating_add(1),
            policy: self.config.policy.as_str(),
        }
    }
}

impl Decision {
    /// `llm.generation`'s `truncation_gate` value; `None` when it did not act.
    pub(super) fn label(&self) -> Option<&'static str> {
        match self.action {
            OutputTruncationAction::Proceed => None,
            OutputTruncationAction::Retry => Some("retried"),
            OutputTruncationAction::Fail => Some("failed"),
        }
    }

    pub(super) fn retries(&self) -> bool {
        self.action == OutputTruncationAction::Retry
    }

    /// Mark a retried generation's assistant message (see module docs).
    pub(super) fn stamp(&self, metadata: &mut HashMap<String, Value>) {
        if self.retries() {
            metadata.insert(
                METADATA_KEY.to_string(),
                json!({"tool_calls_lost": self.lost, "finish_reason": self.finish_reason}),
            );
        }
    }

    /// One `everruns::llm_telemetry` line when the gate acted.
    pub(super) fn log(&self, provider: &str, model: &str) {
        if let Some(action) = self.label() {
            everruns_contracts::llm_telemetry::warn_truncation_gate(
                provider,
                model,
                action,
                self.policy,
                self.lost,
                self.consecutive,
                &self.finish_reason,
            );
        }
    }

    /// The turn-ending error for [`OutputTruncationAction::Fail`]. Not
    /// transient, so no host retries it.
    pub(super) fn failure(&self) -> AgentLoopError {
        let cause = if self.finish_reason == "length" {
            "was cut off at its output limit"
        } else {
            "had tool-call arguments that were not valid JSON"
        };
        let attempts = if self.consecutive > 1 {
            format!(" in {} consecutive responses", self.consecutive)
        } else {
            String::new()
        };
        AgentLoopError::llm_kind(
            LlmErrorKind::MalformedResponse,
            format!(
                "The model's response {cause}{attempts}, so {} tool call(s) were not run \
                 (output_truncation policy: {}). Raise the model's max output tokens or ask \
                 for smaller steps.",
                self.lost, self.policy
            ),
        )
    }
}

fn stamp_of(message: &RuntimeMessage) -> Option<&Value> {
    (message.role == RuntimeMessageRole::Agent)
        .then_some(message.metadata.as_ref()?.get(METADATA_KEY))
        .flatten()
}

/// Generations at the end of the transcript, back to the current turn's input,
/// that the gate retried. Tool results and system messages between them do not
/// break the run; a user message or an unstamped answer does.
pub(super) fn consecutive_truncations(history: &[RuntimeMessage]) -> u32 {
    let mut count = 0u32;
    for message in history.iter().rev() {
        match message.role {
            RuntimeMessageRole::ToolResult | RuntimeMessageRole::System => {}
            RuntimeMessageRole::User => break,
            RuntimeMessageRole::Agent if stamp_of(message).is_some() => {
                count = count.saturating_add(1);
            }
            RuntimeMessageRole::Agent => break,
        }
    }
    count
}

fn note(stamp: &Value) -> String {
    let lost = stamp
        .get("tool_calls_lost")
        .and_then(Value::as_u64)
        .unwrap_or(1);
    if stamp.get("finish_reason").and_then(Value::as_str) == Some("length") {
        format!(
            "[Runtime notice] Your previous response hit the output token limit before it \
             finished, so {lost} tool call(s) in it were cut off and NOT run. Retry with less \
             output per response: for example split large file contents or arguments across \
             several smaller tool calls."
        )
    } else {
        format!(
            "[Runtime notice] {lost} tool call(s) in your previous response had arguments that \
             were not valid JSON, so they were NOT run. Retry them with valid JSON arguments."
        )
    }
}

/// Insert the note after each retried exchange: after its assistant message
/// and the tool results that answer it, before whatever comes next.
pub(super) fn insert_notes(messages: Vec<RuntimeMessage>) -> Vec<RuntimeMessage> {
    if !messages.iter().any(|message| stamp_of(message).is_some()) {
        return messages;
    }
    let mut placed = Vec::with_capacity(messages.len() + 1);
    let mut pending: Option<RuntimeMessage> = None;
    for message in messages {
        if message.role != RuntimeMessageRole::ToolResult
            && let Some(note) = pending.take()
        {
            placed.push(note);
        }
        if let Some(stamp) = stamp_of(&message) {
            let mut notice = RuntimeMessage::user(note(stamp));
            // Rendered from stored history: keep its timestamp stable.
            notice.created_at = message.created_at;
            pending = Some(notice);
        }
        placed.push(message);
    }
    placed.extend(pending);
    placed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamped(finish: &str, lost: u32) -> RuntimeMessage {
        let mut message = RuntimeMessage::assistant("partial");
        let mut metadata = HashMap::new();
        Decision {
            action: OutputTruncationAction::Retry,
            lost,
            finish_reason: finish.into(),
            consecutive: 1,
            policy: "continue",
        }
        .stamp(&mut metadata);
        message.metadata = Some(metadata);
        message
    }

    fn tool_result() -> RuntimeMessage {
        let mut message = RuntimeMessage::user("result");
        message.role = RuntimeMessageRole::ToolResult;
        message
    }

    fn texts(messages: &[RuntimeMessage]) -> Vec<String> {
        messages
            .iter()
            .map(|m| {
                let text = m.text().unwrap_or_default();
                if text.starts_with("[Runtime notice]") {
                    "NOTE".into()
                } else {
                    text.to_string()
                }
            })
            .collect()
    }

    #[test]
    fn consecutive_count_runs_back_to_the_turn_input() {
        let user = RuntimeMessage::user("go");
        let clean = RuntimeMessage::assistant("done");
        assert_eq!(consecutive_truncations(&[]), 0);
        assert_eq!(consecutive_truncations(std::slice::from_ref(&user)), 0);
        assert_eq!(
            consecutive_truncations(&[
                user.clone(),
                stamped("length", 1),
                tool_result(),
                stamped("length", 2)
            ]),
            2
        );
        // A clean answer or a new input ends the run.
        assert_eq!(
            consecutive_truncations(&[stamped("length", 1), clean, stamped("length", 1)]),
            1
        );
        assert_eq!(
            consecutive_truncations(&[stamped("length", 1), user, stamped("length", 1)]),
            1
        );
    }

    #[test]
    fn notes_follow_each_retried_exchange_and_its_results() {
        let messages = vec![
            RuntimeMessage::user("go"),
            stamped("length", 1),
            tool_result(),
            tool_result(),
            RuntimeMessage::assistant("ok"),
            stamped("tool_calls", 1),
        ];
        let placed = insert_notes(messages);
        assert_eq!(
            texts(&placed),
            [
                "go", "partial", "result", "result", "NOTE", "ok", "partial", "NOTE"
            ]
        );
        assert_eq!(placed[4].role, RuntimeMessageRole::User);
        assert!(
            placed[4]
                .text()
                .unwrap()
                .contains("hit the output token limit")
        );
        assert!(placed[7].text().unwrap().contains("not valid JSON"));
        // Stable across requests: same text, same timestamp as its answer.
        let again = insert_notes(placed[..4].to_vec());
        assert_eq!(again[4].text(), placed[4].text());
        assert_eq!(again[4].created_at, placed[1].created_at);
    }

    #[test]
    fn untouched_transcript_is_returned_as_is() {
        let messages = vec![RuntimeMessage::user("go"), RuntimeMessage::assistant("ok")];
        assert_eq!(texts(&insert_notes(messages)), ["go", "ok"]);
    }

    #[test]
    fn failure_names_the_cause_and_is_not_transient() {
        let decision = Decision {
            action: OutputTruncationAction::Fail,
            lost: 2,
            finish_reason: "length".into(),
            consecutive: 3,
            policy: "continue",
        };
        let error = decision.failure();
        let text = error.to_string();
        assert!(
            text.contains("cut off at its output limit in 3 consecutive"),
            "{text}"
        );
        assert!(text.contains("2 tool call(s) were not run"), "{text}");
        assert!(!error.is_transient_llm_error());
        assert_eq!(decision.label(), Some("failed"));
    }
}
