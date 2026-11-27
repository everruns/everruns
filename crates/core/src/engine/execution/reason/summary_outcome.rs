//! A summary becomes installable only after a known, complete terminal outcome.
use super::context_plan::{SUMMARY_MAX_OUTPUT, SUMMARY_RETRY_MAX_OUTPUT};
use crate::driver_registry::{LlmCallConfig, LlmResponse};

/// The output cap ran out before the summary finished: `length` and the
/// provider spellings of it, or an empty answer after hidden reasoning.
pub(super) fn summary_ran_out_of_output(response: &LlmResponse) -> bool {
    let outcome = response.metadata.outcome();
    let cut = outcome == everruns_contracts::CompletionOutcome::OutputLimit;
    let reasoned_away = response.text.trim().is_empty()
        && response.metadata.reasoning_tokens.is_some_and(|n| n > 0);
    let reasoning_only = reasoned_away
        && (outcome == everruns_contracts::CompletionOutcome::Complete
            || (outcome == everruns_contracts::CompletionOutcome::Unknown
                && response.metadata.finish_reason.is_none()
                && response.metadata.provider_finish_reason.is_none()));
    (cut || reasoning_only)
        && response.tool_calls.as_ref().is_none_or(Vec::is_empty)
        && response.metadata.tool_calls_dropped == 0
        && response.metadata.tool_calls_truncated_executed == 0
}

/// Output caps for one continuity summary: the first attempt and the single
/// retry after it ran out of output. The retry is always larger whenever the
/// window allows more than one token, and the source budget reserves it.
pub(super) fn summary_output_caps(window: usize) -> (usize, usize) {
    let first = (window / 4).clamp(1, SUMMARY_MAX_OUTPUT);
    let retry = (window / 2).clamp(1, SUMMARY_RETRY_MAX_OUTPUT).max(first);
    (first, retry)
}

/// Config for the single retry after a summary ran out of output, or `None`
/// when the window leaves no more room than the first attempt had.
///
/// `reserved` is the retry cap the source budget held back. It is a floor:
/// without it, a source page sized to the budget left the retry exactly the
/// first attempt's room, so the retry was skipped and the turn failed at the
/// first cap (Ecosystem Watch, 200k+ history, 2026-11-24).
pub(super) fn summary_retry_config(
    first: &LlmCallConfig,
    response: &LlmResponse,
    window: usize,
    input_estimate: usize,
    reserved: usize,
) -> Option<LlmCallConfig> {
    if !summary_ran_out_of_output(response) {
        return None;
    }
    let first_cap = first.max_tokens.unwrap_or(0) as usize;
    let room = window
        .saturating_sub(input_estimate)
        .min(SUMMARY_RETRY_MAX_OUTPUT)
        .max(reserved.min(window));
    if room <= first_cap {
        return None;
    }
    let mut retry = first.clone();
    retry.max_tokens = Some(room as u32);
    // Only lower reasoning the model demonstrably used; a request field a
    // non-reasoning model never needed is not added.
    if response.metadata.reasoning_tokens.is_some_and(|n| n > 0) || response.text.trim().is_empty()
    {
        retry.reasoning_effort = Some(everruns_contracts::model::ReasoningEffort::Low);
    }
    Some(retry)
}

/// Why `valid_summary` rejected a response, in words for the turn error.
pub(super) fn summary_rejection(response: &LlmResponse) -> &'static str {
    if response
        .tool_calls
        .as_ref()
        .is_some_and(|calls| !calls.is_empty())
        || response.metadata.tool_calls_dropped > 0
    {
        "a tool request"
    } else if response.text.trim().is_empty() {
        "empty"
    } else if summary_ran_out_of_output(response) {
        "cut off"
    } else {
        "unfinished"
    }
}

/// Only a completed, nonempty, tool-free response may replace source history.
/// The text is private so checkpoint installers cannot accept unvalidated output.
pub(super) struct ValidatedSummary<'a>(&'a str);

impl<'a> ValidatedSummary<'a> {
    pub(super) fn new(response: &'a LlmResponse) -> Option<Self> {
        (!response.text.trim().is_empty()
            && response.tool_calls.as_ref().is_none_or(Vec::is_empty)
            && response.metadata.tool_calls_dropped == 0
            && response.metadata.tool_calls_truncated_executed == 0
            && response.metadata.outcome() == everruns_contracts::CompletionOutcome::Complete)
            .then_some(Self(response.text.as_str()))
    }
    pub(super) fn text(&self) -> &'a str {
        self.0
    }
}

#[cfg(test)]
pub(super) fn valid_summary(response: &LlmResponse) -> bool {
    ValidatedSummary::new(response).is_some()
}

#[cfg(test)]
#[path = "summary_outcome_tests.rs"]
mod tests;
