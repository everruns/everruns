//! How one generation ended, recorded on `llm.generation` and the
//! `ReasonAtom: LLM call completed` log line.
//!
//! Why: a response cut off at the output limit looked like any other in the
//! logs, and the tool calls a driver discarded because of it were not counted
//! anywhere. These fields make the edge cases measurable (see
//! `everruns_contracts::llm_telemetry`); nothing here changes behaviour.
//!
//! Split out of `reason.rs` (file-size ratchet), together with the retry and
//! compaction-cost folding that build the same event.

use crate::engine::driver_registry::LlmCompletionMetadata;
use crate::engine::events::{LlmCompactionInfo, LlmGenerationData, LlmRetryInfo, TokenUsage};

/// The ending of one generation, as the driver reported it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct GenerationOutcome {
    /// Normalized finish reason; inferred from the output when the provider
    /// sent none (`tool_calls` with calls, otherwise `stop`).
    pub finish_reason: String,
    /// The provider's own stop reason, verbatim.
    pub provider_finish_reason: Option<String>,
    pub output_tokens: Option<u32>,
    pub tool_calls_dropped: u32,
    pub tool_calls_truncated_executed: u32,
}

impl GenerationOutcome {
    pub(super) fn from_completion(
        meta: Option<&LlmCompletionMetadata>,
        has_tool_calls: bool,
    ) -> Self {
        let inferred = if has_tool_calls { "tool_calls" } else { "stop" };
        Self {
            finish_reason: meta
                .and_then(|m| m.finish_reason.clone())
                .unwrap_or_else(|| inferred.to_string()),
            provider_finish_reason: meta.and_then(|m| m.provider_finish_reason.clone()),
            output_tokens: meta.and_then(|m| m.completion_tokens),
            tool_calls_dropped: meta.map_or(0, |m| m.tool_calls_dropped),
            tool_calls_truncated_executed: meta.map_or(0, |m| m.tool_calls_truncated_executed),
        }
    }

    /// Stamp the outcome on an `llm.generation` payload.
    pub(super) fn apply(&self, data: LlmGenerationData) -> LlmGenerationData {
        data.with_stop_details(
            self.provider_finish_reason.clone(),
            self.tool_calls_dropped,
            self.tool_calls_truncated_executed,
        )
    }

    /// The `ReasonAtom: LLM call completed` line, now carrying the ending.
    pub(super) fn log_completed(
        &self,
        session_id: &dyn std::fmt::Display,
        turn_id: &dyn std::fmt::Display,
        provider: &str,
        model: &str,
        tool_count: usize,
    ) {
        tracing::info!(
            session_id = %session_id,
            turn_id = %turn_id,
            provider,
            model,
            has_tool_calls = tool_count > 0,
            tool_count,
            finish_reason = %self.finish_reason,
            provider_finish_reason = self.provider_finish_reason.as_deref(),
            output_tokens = self.output_tokens,
            tool_calls_dropped = self.tool_calls_dropped,
            tool_calls_truncated_executed = self.tool_calls_truncated_executed,
            "ReasonAtom: LLM call completed"
        );
    }
}

/// Log a provider error that is, or reads like, a context overflow, with the
/// provider and model the driver layer does not know. One place for every
/// driver; `llm_telemetry` decides whether the error qualifies.
pub(super) fn observe_overflow(
    model: &crate::engine::runtime_context::ResolvedModelExecution,
    error: &crate::engine::error::AgentLoopError,
) {
    everruns_contracts::llm_telemetry::observe_request_error(
        model.provider_type.as_str(),
        Some(&model.model),
        None,
        error,
    );
}

/// A provider call failed for good before its stream opened: log a possible
/// overflow, and say so when a transient error ends the turn only because the
/// engine's retry attempts ran out.
pub(super) fn observe_terminal(
    model: &crate::engine::runtime_context::ResolvedModelExecution,
    error: &crate::engine::error::AgentLoopError,
    attempts: u32,
    config: &crate::engine::llm_retry::LlmRetryConfig,
) {
    observe_overflow(model, error);
    if error.is_transient_llm_error()
        && !error.llm_retry_handled()
        && attempts >= config.max_retries
    {
        everruns_contracts::llm_telemetry::warn_retry_exhausted(
            model.provider_type.as_str(),
            attempts,
            config.max_retries,
            "attempts",
        );
    }
}

/// The engine's retry wait ran past its time budget.
pub(super) fn retry_time_exhausted(
    model: &crate::engine::runtime_context::ResolvedModelExecution,
    attempts: u32,
    config: &crate::engine::llm_retry::LlmRetryConfig,
) {
    everruns_contracts::llm_telemetry::warn_retry_exhausted(
        model.provider_type.as_str(),
        attempts,
        config.max_retries,
        "time",
    );
}

/// Retry information worth reporting: only when the call was retried.
pub(super) fn retry_info(meta: Option<&LlmCompletionMetadata>) -> Option<LlmRetryInfo> {
    meta.and_then(|meta| meta.retry_metadata.as_ref())
        .filter(|rm| rm.had_retries())
        .map(|rm| LlmRetryInfo {
            attempts: rm.attempts,
            total_wait_ms: rm.total_retry_wait.as_millis() as u64,
        })
}

pub(super) fn add_compaction_cost(usage: &mut TokenUsage, compaction_cost: f64) {
    let generation_cost = usage.effective_cost_usd();
    usage.effective_cost_usd = Some(generation_cost.unwrap_or(0.0) + compaction_cost);
    if let Some(actual_cost) = usage.actual_cost_usd.as_mut() {
        *actual_cost += compaction_cost;
    }
}

/// Add compaction info if compaction was performed. Compaction is a separate
/// billable model call on the same turn. Preserve whether the generation cost
/// was actual or estimated while recording their combined best-effort cost
/// for budgets and usage totals. `compaction.cost_usd` keeps the split
/// visible (EVE-895).
pub(super) fn with_compaction(
    mut data: LlmGenerationData,
    info: LlmCompactionInfo,
) -> LlmGenerationData {
    if let Some(compaction_cost) = info.cost_usd {
        match data.metadata.usage.as_mut() {
            Some(usage) => add_compaction_cost(usage, compaction_cost),
            // The generation itself reported no usage — a provider may price
            // compaction without returning usage on the retry. Carry the cost
            // on a usage record of its own rather than dropping it, which is
            // the failure this fixes.
            None => {
                data.metadata.usage = Some(TokenUsage {
                    input_tokens: 0,
                    output_tokens: 0,
                    cache_read_tokens: None,
                    cache_creation_tokens: None,
                    actual_cost_usd: Some(compaction_cost),
                    estimated_cost_usd: None,
                    effective_cost_usd: None,
                });
            }
        }
    }
    data.with_compaction(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn completion(finish: Option<&str>, raw: Option<&str>, dropped: u32) -> LlmCompletionMetadata {
        let mut meta = LlmCompletionMetadata::default();
        meta.finish_reason = finish.map(str::to_owned);
        meta.provider_finish_reason = raw.map(str::to_owned);
        meta.completion_tokens = Some(4096);
        meta.tool_calls_dropped = dropped;
        meta
    }

    fn generation() -> LlmGenerationData {
        LlmGenerationData::success(
            Vec::new(),
            Vec::new(),
            Some("partial".into()),
            Vec::new(),
            "claude-sonnet-4-6".into(),
            Some("anthropic".into()),
            None,
            None,
            None,
        )
    }

    #[test]
    fn truncated_generation_carries_raw_reason_and_dropped_calls() {
        let meta = completion(Some("length"), Some("max_tokens"), 2);
        let outcome = GenerationOutcome::from_completion(Some(&meta), false);
        assert_eq!(outcome.finish_reason, "length");
        assert_eq!(outcome.output_tokens, Some(4096));

        let data = outcome.apply(generation());
        let json = serde_json::to_value(&data.metadata).unwrap();
        assert_eq!(json["provider_finish_reason"], "max_tokens");
        assert_eq!(json["tool_calls_dropped"], 2);
        // Zero counts stay off the wire.
        assert!(json.get("tool_calls_truncated_executed").is_none());
    }

    #[test]
    fn clean_generation_adds_nothing_and_infers_missing_reason() {
        let outcome = GenerationOutcome::from_completion(None, true);
        assert_eq!(outcome.finish_reason, "tool_calls");
        assert_eq!(
            GenerationOutcome::from_completion(None, false).finish_reason,
            "stop"
        );

        let meta = completion(Some("stop"), None, 0);
        let data = GenerationOutcome::from_completion(Some(&meta), false).apply(generation());
        let json = serde_json::to_value(&data.metadata).unwrap();
        for absent in [
            "provider_finish_reason",
            "tool_calls_dropped",
            "tool_calls_truncated_executed",
        ] {
            assert!(json.get(absent).is_none(), "{absent}");
        }
    }
}
