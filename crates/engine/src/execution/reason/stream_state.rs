use crate::driver_registry::{LlmCompletionMetadata, LlmStreamError, LlmStreamEvent};
use crate::llm_retry::{RetryMetadata, is_transient_stream_error};
use crate::output_guardrail::{ArmedGuardrail, TrippedGuardrail, evaluate_guardrails};
use everruns_provider::reasoning::ReasoningContentPart;

/// Whether replaying the current provider attempt can duplicate externally
/// visible output or tool side effects.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum StreamReplayState {
    /// Only replay-safe reasoning or metadata has been observed.
    #[default]
    Replayable,
    /// Final answer text or tool calls have crossed the replay boundary.
    Committed,
}

impl StreamReplayState {
    pub(super) fn observe(&mut self, event: &LlmStreamEvent) {
        if matches!(self, Self::Committed) {
            return;
        }

        if match event {
            LlmStreamEvent::TextDelta(delta) => !delta.is_empty(),
            LlmStreamEvent::ToolCalls(calls) => !calls.is_empty(),
            LlmStreamEvent::ReasoningDelta { .. }
            | LlmStreamEvent::ReasoningItem(_)
            | LlmStreamEvent::MessagePhase(_)
            | LlmStreamEvent::Done(_)
            | LlmStreamEvent::Error(_) => false,
        } {
            *self = Self::Committed;
        }
    }

    pub(super) fn should_retry(
        self,
        error: &LlmStreamError,
        retry_attempts: u32,
        max_retries: u32,
    ) -> bool {
        matches!(self, Self::Replayable)
            && retry_attempts < max_retries
            && is_transient_stream_error(error)
    }
}

/// Terminal state of a single provider stream attempt.
///
/// Keeping this as one enum prevents completion, partial success, and
/// guardrail blocking from being represented by contradictory independent
/// flags.
pub(super) enum StreamTermination {
    Exhausted,
    /// Boxed like `LlmStreamEvent::Done`: the metadata dwarfs every other
    /// variant, so carrying it inline would size the whole enum after it.
    Completed(Box<LlmCompletionMetadata>),
    PartialSuccess,
    GuardrailBlocked(TrippedGuardrail),
}

impl StreamTermination {
    pub(super) fn into_parts(self) -> (Option<LlmCompletionMetadata>, Option<TrippedGuardrail>) {
        match self {
            Self::Completed(metadata) => (Some(*metadata), None),
            Self::GuardrailBlocked(guardrail) => (None, Some(guardrail)),
            Self::Exhausted | Self::PartialSuccess => (None, None),
        }
    }
}

pub(super) fn merge_retry_metadata(
    existing: Option<RetryMetadata>,
    additional: &RetryMetadata,
) -> Option<RetryMetadata> {
    if !additional.had_retries() {
        return existing;
    }

    let mut merged = existing.unwrap_or_default();
    merged.attempts += additional.attempts;
    merged.total_retry_wait += additional.total_retry_wait;
    if additional.last_rate_limit_info.is_some() {
        merged.last_rate_limit_info = additional.last_rate_limit_info.clone();
    }
    Some(merged)
}

/// Returns true when a stream event carries assistant output progress.
pub(super) fn advances_stall_deadline(event: &LlmStreamEvent) -> bool {
    match event {
        LlmStreamEvent::TextDelta(delta) | LlmStreamEvent::ReasoningDelta { delta, .. } => {
            !delta.is_empty()
        }
        LlmStreamEvent::ReasoningItem(item) => {
            item.has_replay_state() || item.display_text().is_some()
        }
        LlmStreamEvent::ToolCalls(calls) => !calls.is_empty(),
        LlmStreamEvent::MessagePhase(_) | LlmStreamEvent::Done(_) | LlmStreamEvent::Error(_) => {
            false
        }
    }
}

pub(super) fn append_guarded_thinking_delta(
    armed_guardrails: &mut [ArmedGuardrail],
    thinking: &mut String,
    pending_thinking_delta: &mut String,
    delta: &str,
) -> Option<TrippedGuardrail> {
    thinking.push_str(delta);

    if let Some(tripped) = evaluate_guardrails(armed_guardrails, thinking, delta) {
        pending_thinking_delta.clear();
        Some(tripped)
    } else {
        pending_thinking_delta.push_str(delta);
        None
    }
}

/// Inspect readable completed reasoning without feeding a provider's streamed
/// copy through stateful guardrails twice.
pub(super) fn inspect_guarded_reasoning_item(
    armed_guardrails: &mut [ArmedGuardrail],
    inspected_reasoning: &mut String,
    item: &ReasoningContentPart,
) -> Option<TrippedGuardrail> {
    let display_text = item.display_text()?;

    // Anthropic emits the completed signed block after streaming the same text
    // as deltas. Gemini signed thoughts arrive only as completed items.
    if inspected_reasoning.ends_with(&display_text) {
        return None;
    }

    inspected_reasoning.push_str(&display_text);
    evaluate_guardrails(armed_guardrails, inspected_reasoning, &display_text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasoning_only_attempt_remains_replayable() {
        let mut state = StreamReplayState::default();
        state.observe(&LlmStreamEvent::ReasoningDelta {
            delta: "analysis".to_string(),
            summary: false,
        });
        state.observe(&LlmStreamEvent::ReasoningItem(
            everruns_provider::reasoning::ReasoningContentPart::opaque("openai")
                .with_item_id("item")
                .with_encrypted("opaque")
                .with_tokens(1),
        ));

        let stall = LlmStreamError::new("provider stream stall: no tokens for 120s");
        assert!(state.should_retry(&stall, 0, 2));
    }

    #[test]
    fn final_output_commits_attempt_against_replay() {
        let stall = LlmStreamError::new("provider stream stall: no tokens for 120s");

        let mut text = StreamReplayState::default();
        text.observe(&LlmStreamEvent::TextDelta("answer".to_string()));
        assert_eq!(text, StreamReplayState::Committed);
        assert!(!text.should_retry(&stall, 0, 2));

        let mut tools = StreamReplayState::default();
        tools.observe(&LlmStreamEvent::ToolCalls(vec![
            crate::tool_types::ToolCall {
                id: "call".to_string(),
                name: "tool".to_string(),
                arguments: serde_json::json!({}),
            },
        ]));
        assert_eq!(tools, StreamReplayState::Committed);
        assert!(!tools.should_retry(&stall, 0, 2));
    }

    #[test]
    fn retry_budget_is_part_of_replay_decision() {
        let stall = LlmStreamError::new("provider stream stall: no tokens for 120s");
        assert!(StreamReplayState::Replayable.should_retry(&stall, 0, 2));
        assert!(!StreamReplayState::Replayable.should_retry(&stall, 2, 2));
    }
}
