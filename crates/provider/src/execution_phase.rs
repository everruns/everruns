//! Execution phase for assistant messages in multi-step tool-calling flows.
//!
//! This is a provider-wire concept: phases are parsed off the provider stream
//! and, for providers with native support, sent back on the request. It lives in
//! the provider abstraction so both the driver types (`LlmMessage`,
//! `LlmStreamEvent`) and core's `Message` can share it.

use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Execution phase for assistant messages in multi-step tool-calling flows.
///
/// Providers that natively support phases (OpenAI GPT-5.x) send the phase value
/// directly in the API request. For providers without native support (Anthropic,
/// Gemini), the phase is still tracked internally and derived from state in the
/// ReasonAtom, but is not sent to the provider API.
///
/// Serialized as lowercase strings for backward compatibility with existing
/// persisted messages: `"commentary"` and `"final_answer"`.
///
/// Legacy values `"in_progress"` and `"completed"` are accepted during
/// deserialization for backward compatibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
// The serde impls below are hand-written, so utoipa cannot infer the wire
// casing from a `serde(rename_all)` attribute and would otherwise publish the
// Rust variant names. Keep this in lockstep with `as_provider_str`.
#[cfg_attr(feature = "openapi", schema(rename_all = "snake_case"))]
pub enum ExecutionPhase {
    /// Intermediate update — preamble or commentary before/between tool calls.
    /// The model is still working and may issue more tool calls.
    Commentary,
    /// Final completed response — no more tool calls expected.
    FinalAnswer,
}

impl ExecutionPhase {
    /// Derive phase from whether the response contains tool calls.
    pub fn from_has_tool_calls(has_tool_calls: bool) -> Self {
        if has_tool_calls {
            Self::Commentary
        } else {
            Self::FinalAnswer
        }
    }

    /// Parse a provider wire value into an ExecutionPhase.
    /// Returns `None` for unrecognized values.
    pub fn from_provider_str(s: &str) -> Option<Self> {
        match s {
            "commentary" | "in_progress" => Some(Self::Commentary),
            "final_answer" | "completed" => Some(Self::FinalAnswer),
            _ => None,
        }
    }

    /// Wire value used by providers that support native phases (OpenAI).
    pub fn as_provider_str(&self) -> &'static str {
        match self {
            Self::Commentary => "commentary",
            Self::FinalAnswer => "final_answer",
        }
    }

    /// Monotonic refinement for the streamed phase *hint* on
    /// `output.message.started` / `output.message.delta`.
    ///
    /// The hint may advance `None -> Commentary | FinalAnswer` exactly once and
    /// then never changes: it never flip-flops between variants and never
    /// reverts to `None`. Once a message has been classified mid-stream that
    /// classification stays put; the authoritative value remains the completed
    /// `Message.phase`. Returns the (possibly unchanged) refined hint.
    pub fn refine_streamed_hint(current: Option<Self>, incoming: Self) -> Option<Self> {
        // First classification wins; a later hint cannot overwrite it.
        Some(current.unwrap_or(incoming))
    }
}

impl std::fmt::Display for ExecutionPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_provider_str())
    }
}

impl Serialize for ExecutionPhase {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_provider_str())
    }
}

impl<'de> Deserialize<'de> for ExecutionPhase {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "commentary" | "in_progress" => Ok(Self::Commentary),
            "final_answer" | "completed" => Ok(Self::FinalAnswer),
            other => Err(serde::de::Error::unknown_variant(
                other,
                &["commentary", "final_answer", "in_progress", "completed"],
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execution_phase_from_provider_str() {
        assert_eq!(
            ExecutionPhase::from_provider_str("commentary"),
            Some(ExecutionPhase::Commentary)
        );
        assert_eq!(
            ExecutionPhase::from_provider_str("final_answer"),
            Some(ExecutionPhase::FinalAnswer)
        );
        assert_eq!(
            ExecutionPhase::from_provider_str("in_progress"),
            Some(ExecutionPhase::Commentary)
        );
        assert_eq!(
            ExecutionPhase::from_provider_str("completed"),
            Some(ExecutionPhase::FinalAnswer)
        );
        assert_eq!(ExecutionPhase::from_provider_str("unknown"), None);
    }
}
