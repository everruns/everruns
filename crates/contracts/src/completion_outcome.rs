//! Provider-neutral completion semantics. Unknown never means complete.

/// A terminal model outcome, separate from the provider's retained raw reason.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum CompletionOutcome {
    Complete,
    ToolCalls,
    OutputLimit,
    Refused,
    Cancelled,
    Failed,
    #[default]
    Unknown,
}

impl CompletionOutcome {
    /// Normalize once at the protocol boundary; unrecognized values stay unknown.
    pub fn from_finish_reason(reason: Option<&str>) -> Self {
        match reason.map(str::to_ascii_lowercase).as_deref() {
            Some("stop" | "stop_sequence" | "end_turn" | "completed") => Self::Complete,
            Some("tool_calls" | "tool_use" | "function_call" | "requires_action") => {
                Self::ToolCalls
            }
            Some("length" | "max_tokens" | "max_output_tokens") => Self::OutputLimit,
            Some("refusal" | "content_filter" | "safety") => Self::Refused,
            Some("cancelled" | "canceled") => Self::Cancelled,
            Some("error" | "failed") => Self::Failed,
            _ => Self::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_spellings_and_unknown_endings_stay_distinct() {
        for reason in ["length", "max_tokens", "MAX_TOKENS", "MAX_OUTPUT_TOKENS"] {
            assert_eq!(
                CompletionOutcome::from_finish_reason(Some(reason)),
                CompletionOutcome::OutputLimit
            );
        }
        for reason in [None, Some("future-provider-reason"), Some("")] {
            assert_eq!(
                CompletionOutcome::from_finish_reason(reason),
                CompletionOutcome::Unknown
            );
        }
        assert_eq!(
            CompletionOutcome::from_finish_reason(Some("END_TURN")),
            CompletionOutcome::Complete
        );
        assert_eq!(
            CompletionOutcome::from_finish_reason(Some("SAFETY")),
            CompletionOutcome::Refused
        );
    }

    #[test]
    fn a_normalized_stop_cannot_promote_an_unknown_provider_ending() {
        for raw in ["pause_turn", "future-provider-ending", "requires_action"] {
            let metadata = crate::LlmCompletionMetadata {
                finish_reason: Some("stop".into()),
                provider_finish_reason: Some(raw.into()),
                ..Default::default()
            };
            assert_ne!(metadata.outcome(), CompletionOutcome::Complete, "{raw}");
        }
        let mut metadata = crate::LlmCompletionMetadata {
            finish_reason: Some("stop".into()),
            provider_finish_reason: Some("stop_sequence".into()),
            ..Default::default()
        };
        assert_eq!(metadata.outcome(), CompletionOutcome::Complete);
        metadata.finish_reason = Some("incomplete".into());
        assert_eq!(metadata.outcome(), CompletionOutcome::Unknown);
    }
}
