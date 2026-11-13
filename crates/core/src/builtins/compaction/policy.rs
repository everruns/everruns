use super::*;
use crate::compaction_policy::{
    CompactionSettings, CompactionStrategy as ExecutionCompactionStrategy,
    ObservationMaskingResult as ExecutionObservationMaskingResult,
};

#[derive(Debug)]
pub(super) struct ConfiguredCompactionPolicy {
    pub(super) config: RuntimeCompactionConfig,
}

impl CompactionPolicy for ConfiguredCompactionPolicy {
    fn settings(&self) -> CompactionSettings {
        CompactionSettings {
            strategy: match self.config.strategy {
                CompactionStrategy::Auto => ExecutionCompactionStrategy::Auto,
                CompactionStrategy::Native => ExecutionCompactionStrategy::Native,
                CompactionStrategy::ObservationMasking => {
                    ExecutionCompactionStrategy::ObservationMasking
                }
                CompactionStrategy::Summarization => ExecutionCompactionStrategy::Summarization,
            },
            budget_percent: self.config.budget_percent,
            summarization_model: self.config.summarization.model.clone(),
        }
    }

    fn estimate_total_tokens(&self, messages: &[Message]) -> usize {
        estimate_total_tokens(messages)
    }

    fn total_tool_result_bytes(&self, messages: &[RuntimeMessage]) -> usize {
        total_tool_result_bytes(messages)
    }

    fn should_compact_proactively(&self, messages: &[Message], context_window: usize) -> bool {
        should_compact_proactively(messages, &self.config, context_window)
    }

    fn should_compact_with_fixed_input(
        &self,
        messages: &[Message],
        context_window: usize,
        fixed_input_tokens: usize,
    ) -> bool {
        self.config.proactive
            && estimate_total_tokens(messages).saturating_add(fixed_input_tokens)
                > (context_window as f32 * self.config.budget_percent) as usize
    }

    fn should_compact_for_cost(
        &self,
        estimated_input_tokens: usize,
        raw_tool_result_bytes: usize,
        usage: Option<&crate::builtins::events::TokenUsage>,
    ) -> bool {
        should_compact_for_cost(
            estimated_input_tokens,
            raw_tool_result_bytes,
            &self.config,
            usage,
        )
    }

    fn apply_observation_masking(&self, messages: &[Message]) -> ExecutionObservationMaskingResult {
        let result = apply_observation_masking(messages, &self.config.observation_masking);
        ExecutionObservationMaskingResult {
            messages: result.messages,
            masked_count: result.masked_count,
        }
    }

    fn aggressive_trim(
        &self,
        messages: &[Message],
        target_tokens: usize,
        preserve_system: bool,
    ) -> Vec<Message> {
        aggressive_trim(messages, target_tokens, preserve_system)
    }

    fn summarization_prompt(&self) -> String {
        build_summarization_prompt(&self.config.summarization)
    }

    fn format_messages_for_summarization(&self, messages: &[Message]) -> String {
        format_messages_for_summarization(messages)
    }

    fn compose_summary_with_recent(
        &self,
        system_message: Option<Message>,
        summary_text: &str,
        recent_messages: &[Message],
    ) -> Vec<Message> {
        compose_summary_with_recent(system_message, summary_text, recent_messages)
    }
}
