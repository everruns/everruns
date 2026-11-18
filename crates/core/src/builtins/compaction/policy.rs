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
    fn proactive_enabled(&self) -> bool {
        self.config.proactive
    }
    fn working_context_tokens(&self) -> Option<usize> {
        Some(self.config.context_budget_tokens)
    }

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
        let indices: Vec<_> = messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m.role == MessageRole::Tool && !is_protected_tool_result(messages, m))
            .map(|(index, _)| index)
            .collect();
        let sizes: Vec<_> = indices
            .iter()
            .map(|i| extract_text(&messages[*i].content).len())
            .collect();
        let mut config = self.config.cost_control.clone();
        config.keep_recent_tool_results = self.config.observation_masking.keep_recent_tool_outputs;
        let boundary = mask_boundary(&sizes, &config);
        let mut result = messages.to_vec();
        let mut masked_count = 0;
        for index in indices.into_iter().take(boundary) {
            let original = extract_text(&messages[index].content);
            if original.len() < config.min_masked_tool_result_bytes {
                continue;
            }
            let summary = match self.config.observation_masking.summary_format {
                MaskingSummaryFormat::OneLine => format_one_line_summary(
                    &find_tool_call_name(messages, &messages[index]),
                    &messages[index].content,
                ),
                MaskingSummaryFormat::HeadTail => {
                    format_head_tail_summary(&messages[index].content)
                }
            };
            if summary.len() < original.len() {
                result[index].content = MessageContent::Text(summary);
                masked_count += 1;
            }
        }
        ExecutionObservationMaskingResult {
            messages: result,
            masked_count,
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

    fn pinned_context(&self, messages: &[Message]) -> Vec<String> {
        messages
            .iter()
            .enumerate()
            .filter(|(index, message)| is_protected_tool_result(&messages[..*index], message))
            .map(|(_, message)| extract_text(&message.content))
            .collect()
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
