//! Neutral execution seam for capability-owned context-compaction policy.

use std::fmt::Debug;

use serde::{Deserialize, Serialize};

use crate::driver_registry::Message;
use crate::events::TokenUsage;
// `total_tool_result_bytes` measures the raw stored history; every other hook
// on this trait operates on the provider-facing wire messages. The alias keeps
// the two apart at a glance.
use crate::message::Message as StoredMessage;

/// Strategy selected by a configured compaction policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CompactionStrategy {
    #[default]
    Auto,
    Native,
    ObservationMasking,
    Summarization,
}

impl std::fmt::Display for CompactionStrategy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auto => write!(formatter, "auto"),
            Self::Native => write!(formatter, "native"),
            Self::ObservationMasking => write!(formatter, "observation_masking"),
            Self::Summarization => write!(formatter, "summarization"),
        }
    }
}

/// Small execution settings the reason atom needs to orchestrate a policy.
#[derive(Debug, Clone)]
pub struct CompactionSettings {
    pub strategy: CompactionStrategy,
    pub budget_percent: f32,
    pub summarization_model: Option<String>,
}

/// Result of applying policy-owned observation masking.
#[derive(Debug, Clone)]
pub struct ObservationMaskingResult {
    pub messages: Vec<Message>,
    pub masked_count: usize,
}

/// Capability-owned context-compaction behavior consumed by the reason atom.
///
/// Core owns orchestration, provider calls, checkpoints, and events. The
/// implementation bundle owns thresholds and deterministic message transforms.
pub trait CompactionPolicy: Send + Sync + Debug {
    fn settings(&self) -> CompactionSettings;
    fn estimate_total_tokens(&self, messages: &[Message]) -> usize;
    fn total_tool_result_bytes(&self, messages: &[StoredMessage]) -> usize;
    fn should_compact_proactively(&self, messages: &[Message], context_window: usize) -> bool;
    fn should_compact_for_cost(
        &self,
        estimated_input_tokens: usize,
        raw_tool_result_bytes: usize,
        usage: Option<&TokenUsage>,
    ) -> bool;
    fn apply_observation_masking(&self, messages: &[Message]) -> ObservationMaskingResult;
    fn aggressive_trim(
        &self,
        messages: &[Message],
        target_tokens: usize,
        preserve_system: bool,
    ) -> Vec<Message>;
    fn summarization_prompt(&self) -> String;
    fn format_messages_for_summarization(&self, messages: &[Message]) -> String;
    fn compose_summary_with_recent(
        &self,
        system_message: Option<Message>,
        summary_text: &str,
        recent_messages: &[Message],
    ) -> Vec<Message>;
}
