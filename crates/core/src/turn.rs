//! Provider-neutral turn completion semantics.
//!
//! Turn execution state and transition logic live in `everruns-engine`. Core
//! retains only the stable stop-reason value shared by events, APIs, hosts,
//! providers, and durable storage.

use serde::{Deserialize, Serialize};

/// Stable reason a turn stopped, independent of provider-specific strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStopReason {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Error,
    Cancelled,
}

impl TurnStopReason {
    /// Normalize a provider finish reason at the runtime boundary.
    pub fn from_provider_finish_reason(reason: Option<&str>) -> Self {
        match reason.map(str::to_ascii_lowercase).as_deref() {
            Some("length" | "max_tokens" | "max_output_tokens") => Self::MaxTokens,
            Some("refusal" | "content_filter" | "safety") => Self::Refusal,
            Some("error") => Self::Error,
            Some("cancelled" | "canceled") => Self::Cancelled,
            _ => Self::EndTurn,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_reason_wire_values_are_stable() {
        for (value, expected) in [
            (TurnStopReason::EndTurn, "\"end_turn\""),
            (TurnStopReason::MaxTokens, "\"max_tokens\""),
            (TurnStopReason::MaxTurnRequests, "\"max_turn_requests\""),
            (TurnStopReason::Refusal, "\"refusal\""),
            (TurnStopReason::Error, "\"error\""),
            (TurnStopReason::Cancelled, "\"cancelled\""),
        ] {
            assert_eq!(serde_json::to_string(&value).unwrap(), expected);
        }
    }

    #[test]
    fn provider_finish_reasons_are_normalized() {
        assert_eq!(
            TurnStopReason::from_provider_finish_reason(Some("content_filter")),
            TurnStopReason::Refusal
        );
        assert_eq!(
            TurnStopReason::from_provider_finish_reason(Some("cancelled")),
            TurnStopReason::Cancelled
        );
        assert_eq!(
            TurnStopReason::from_provider_finish_reason(None),
            TurnStopReason::EndTurn
        );
    }
}
