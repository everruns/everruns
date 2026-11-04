//! Refusals are terminal model decisions, never transient transport failures.
use crate::{LlmStreamError, LlmStreamEvent};
use serde_json::Value;

// Deltas are buffered privately until the authoritative full reason arrives.
// THREAT[TM-DOS-038]: bound provider-controlled refusal content even when
// the provider never sends a terminal frame.
const MAX_REASON_BYTES: usize = 16 * 1024;

#[derive(Default)]
pub(super) struct RefusalState {
    reason: String,
    observed: bool,
    emitted: bool,
}

pub(super) fn error(reason: &str) -> LlmStreamError {
    let end = reason.floor_char_boundary(MAX_REASON_BYTES.min(reason.len()));
    LlmStreamError::provider(Some("refusal"), None, &reason[..end])
}

pub(super) fn resource_reason(resource: &Value) -> Option<&str> {
    resource["output"].as_array()?.iter().find_map(item_reason)
}

fn item_reason(item: &Value) -> Option<&str> {
    if item["type"] != "message" {
        return None;
    }
    item["content"].as_array()?.iter().find_map(part_reason)
}

fn part_reason(part: &Value) -> Option<&str> {
    (part["type"] == "refusal")
        .then(|| part["refusal"].as_str())
        .flatten()
}

impl RefusalState {
    pub(super) fn observe(&mut self, frame: &Value) -> Option<LlmStreamEvent> {
        if self.emitted {
            return Some(LlmStreamEvent::TextDelta(String::new()));
        }
        let reason = match frame["type"].as_str()? {
            "response.refusal.delta" => {
                self.observed = true;
                if let Some(delta) = frame["delta"].as_str() {
                    let remaining = MAX_REASON_BYTES.saturating_sub(self.reason.len());
                    self.reason
                        .push_str(&delta[..delta.floor_char_boundary(remaining.min(delta.len()))]);
                    if delta.len() >= remaining {
                        return self.finish();
                    }
                }
                return Some(LlmStreamEvent::TextDelta(String::new()));
            }
            "error" if self.observed => return self.finish(),
            "response.refusal.done" => frame["refusal"].as_str(),
            "response.content_part.done" => part_reason(&frame["part"]),
            "response.output_item.done" => item_reason(&frame["item"]),
            "response.completed" | "response.incomplete" | "response.done" | "response.failed" => {
                resource_reason(&frame["response"]).or_else(|| resource_reason(frame))
            }
            _ => None,
        };
        if let Some(reason) = reason {
            self.observed = true;
            self.emitted = true;
            return Some(LlmStreamEvent::Error(error(reason)));
        }
        if matches!(
            frame["type"].as_str(),
            Some(
                "response.completed" | "response.incomplete" | "response.done" | "response.failed"
            )
        ) {
            return self.finish();
        }
        None
    }

    /// A truncated refusal still must not become a retryable network error or
    /// a successful empty completion. Preserve the bounded fragment received.
    pub(super) fn finish(&mut self) -> Option<LlmStreamEvent> {
        if self.emitted {
            return Some(LlmStreamEvent::TextDelta(String::new()));
        }
        if !self.observed {
            return None;
        }
        self.emitted = true;
        Some(LlmStreamEvent::Error(error(&self.reason)))
    }
}
