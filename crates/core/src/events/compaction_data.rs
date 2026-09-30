//! Payloads for context compaction: reasons, triggers and outcomes.

use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Reason why compaction was triggered.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CompactionReason {
    /// Triggered proactively at budget threshold.
    ProactiveBudget,
    /// Triggered reactively on RequestTooLarge error.
    RequestTooLarge,
    /// Triggered manually by user command.
    Manual,
}

impl std::fmt::Display for CompactionReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProactiveBudget => write!(f, "proactive_budget"),
            Self::RequestTooLarge => write!(f, "request_too_large"),
            Self::Manual => write!(f, "manual"),
        }
    }
}

/// What triggered a compaction lifecycle: the context-window budget or cost pressure.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CompactionTrigger {
    /// Window/budget pressure: at or approaching the model's context limit.
    #[default]
    ContextBudget,
    /// Cost pressure: compact early to keep prompts small while headroom remains.
    CostPressure,
}

/// Why a pressured compaction evaluation did not install compacted context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CompactionSkipReason {
    /// Configured strategy excludes native compaction (masking/summarization-only).
    StrategyExcludesNative,
    /// Active driver has no native compact endpoint.
    DriverUnsupported,
    /// No durable checkpoint store available to install the compacted context.
    CheckpointStoreUnavailable,
    /// A recent native attempt is still inside its rearm window.
    CooldownActive,
    /// Native compaction ran but produced no checkpoint to install.
    NativeReturnedNone,
    /// Compacted output was not materially smaller than the input.
    NoMaterialReduction,
    /// Reactive preconditions (window, strategy, usage) rejected the attempt.
    GuardRejected,
}

/// Which stage of a compaction attempt failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CompactionFailStage {
    /// The native compact endpoint call failed.
    NativeCompaction,
    /// Installing the compacted checkpoint failed.
    CheckpointInstall,
    /// Fallback summarization failed.
    Summarization,
}

/// Data for context.compaction.skipped: pressure observed, nothing installed.
///
/// Emitted when context-budget or cost pressure is present but the evaluation
/// does not install compacted context. The envelope timestamp records when the
/// decision was made; every pressured evaluation closes with exactly one of
/// skipped, installed (context.compacted), or failed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ContextCompactionSkippedData {
    /// Why compaction was evaluated.
    #[cfg_attr(feature = "openapi", schema(example = "proactive_budget"))]
    pub reason: CompactionReason,
    /// Whether window/budget or cost pressure triggered the evaluation.
    #[cfg_attr(feature = "openapi", schema(example = "context_budget"))]
    pub trigger: CompactionTrigger,
    /// Why nothing was installed.
    #[cfg_attr(feature = "openapi", schema(example = "cooldown_active"))]
    pub skip_reason: CompactionSkipReason,
    /// Strategy requested.
    #[cfg_attr(feature = "openapi", schema(example = "summary_then_trim"))]
    pub strategy: String,
    /// Model the evaluation ran under.
    #[cfg_attr(feature = "openapi", schema(example = "gpt-5-mini"))]
    pub model: String,
    /// Provider backend (e.g. "openai"), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "openai"))]
    pub provider: Option<String>,
    /// Local driver identifier, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "openai-chat"))]
    pub driver: Option<String>,
    /// Estimated input tokens observed at evaluation time.
    #[cfg_attr(feature = "openapi", schema(example = 184320))]
    pub tokens_observed: u64,
    /// Tokens of headroom remaining, when measurable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = 8192))]
    pub budget_remaining_tokens: Option<u64>,
    /// Source message sequence the evaluation ran at, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = 481))]
    pub source_sequence: Option<i64>,
    /// Number of messages observed.
    #[cfg_attr(feature = "openapi", schema(example = 120))]
    pub messages_observed: usize,
}

/// Data for context.compaction.failed: an attempt errored before installing.
///
/// Terminal event for an emitted context.compacting attempt that did not
/// install. The envelope timestamp records when the failure surfaced.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ContextCompactionFailedData {
    /// Why compaction was attempted.
    #[cfg_attr(feature = "openapi", schema(example = "proactive_budget"))]
    pub reason: CompactionReason,
    /// Whether window/budget or cost pressure triggered the attempt.
    #[cfg_attr(feature = "openapi", schema(example = "context_budget"))]
    pub trigger: CompactionTrigger,
    /// Which stage failed.
    #[cfg_attr(feature = "openapi", schema(example = "summarization"))]
    pub stage: CompactionFailStage,
    /// Human-readable failure.
    #[cfg_attr(
        feature = "openapi",
        schema(example = "summarizer request failed: upstream timed out")
    )]
    pub error: String,
    /// Strategy requested.
    #[cfg_attr(feature = "openapi", schema(example = "summary_then_trim"))]
    pub strategy: String,
    /// Model the attempt ran under.
    #[cfg_attr(feature = "openapi", schema(example = "gpt-5-mini"))]
    pub model: String,
    /// Provider backend (e.g. "openai"), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "openai"))]
    pub provider: Option<String>,
    /// Local driver identifier, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "openai-chat"))]
    pub driver: Option<String>,
    /// Estimated or provider-reported input tokens before the attempt.
    #[cfg_attr(feature = "openapi", schema(example = 184320))]
    pub tokens_before: u64,
    /// Tokens of headroom remaining, when measurable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = 8192))]
    pub budget_remaining_tokens: Option<u64>,
    /// Source message sequence the attempt ran at, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = 481))]
    pub source_sequence: Option<i64>,
    /// Number of messages before the attempt.
    #[cfg_attr(feature = "openapi", schema(example = 120))]
    pub messages_before: usize,
    /// Durable checkpoint being installed when the failure hit, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "openapi",
        schema(example = "01934c2f-9f2e-7c1b-8d3e-4f5a6b7c8d9e")
    )]
    pub checkpoint_id: Option<String>,
}

/// Data for context.compacting event (compaction starting).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ContextCompactingData {
    /// Why compaction was triggered.
    #[cfg_attr(feature = "openapi", schema(example = "proactive_budget"))]
    pub reason: CompactionReason,
    /// Strategy requested (may differ from strategy_used in the completed event).
    #[cfg_attr(feature = "openapi", schema(example = "summary_then_trim"))]
    pub strategy: String,
    /// Number of messages before compaction.
    #[cfg_attr(feature = "openapi", schema(example = 120))]
    pub messages_before: usize,
    /// Estimated or provider-reported input tokens before compaction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = 184320))]
    pub tokens_before: Option<u64>,
    /// Serialized request-context bytes before compaction, when measurable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_before: Option<u64>,
    /// What triggered this attempt: context-window budget or cost pressure.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(example = "context_budget"))]
    pub trigger: CompactionTrigger,
    /// Model performing the compaction.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(example = "gpt-5-mini"))]
    pub model: String,
    /// Provider backend (e.g. "openai"), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "openai"))]
    pub provider: Option<String>,
    /// Local driver identifier, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "openai-chat"))]
    pub driver: Option<String>,
    /// Tokens of headroom remaining when the attempt started, when measurable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = 8192))]
    pub budget_remaining_tokens: Option<u64>,
    /// Source message sequence the attempt ran at, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = 481))]
    pub source_sequence: Option<i64>,
    /// Cached input tokens read before compaction, when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = 90210))]
    pub cache_read_tokens: Option<u32>,
    /// Cache-creation tokens written before compaction, when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = 1024))]
    pub cache_creation_tokens: Option<u32>,
}

/// A single step in a compaction cascade.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CompactionStepData {
    /// Strategy used in this step.
    pub strategy: String,
    /// Number of messages after this step.
    pub messages_after: usize,
    /// Duration of this step in milliseconds.
    pub duration_ms: u64,
}

/// Data for context.compacted event (compaction completed).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ContextCompactedData {
    /// Durable checkpoint installed by this compaction, when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "openapi",
        schema(example = "01934c2f-9f2e-7c1b-8d3e-4f5a6b7c8d9e")
    )]
    pub checkpoint_id: Option<String>,
    /// Combined strategy description (e.g., "observation_masking+native").
    pub strategy_used: String,
    /// Number of messages before compaction.
    #[cfg_attr(feature = "openapi", schema(example = 120))]
    pub messages_before: usize,
    /// Number of messages after compaction.
    pub messages_after: usize,
    /// Estimated or provider-reported input tokens before compaction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = 184320))]
    pub tokens_before: Option<u64>,
    /// Provider-reported output tokens after compaction, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_after: Option<u64>,
    /// Serialized provider checkpoint size in bytes, when measured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_bytes: Option<u64>,
    /// Source used to reconstruct the provider prefix (`checkpoint` or `raw`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replay_source: Option<String>,
    /// Serialized request-context bytes before compaction, when measurable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_before: Option<u64>,
    /// Serialized compact output bytes, when measurable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_after: Option<u64>,
    /// Total duration of all compaction steps in milliseconds.
    pub duration_ms: u64,
    /// Individual steps in the cascade.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<CompactionStepData>,
    /// What triggered this compaction: context-window budget or cost pressure.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(example = "context_budget"))]
    pub trigger: CompactionTrigger,
    /// Model that performed the compaction.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(example = "gpt-5-mini"))]
    pub model: String,
    /// Provider backend (e.g. "openai"), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "openai"))]
    pub provider: Option<String>,
    /// Local driver identifier, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "openai-chat"))]
    pub driver: Option<String>,
    /// Tokens of headroom remaining when the install completed, when measurable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = 8192))]
    pub budget_remaining_tokens: Option<u64>,
    /// Source message sequence the compaction ran at, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = 481))]
    pub source_sequence: Option<i64>,
    /// Cached input tokens read after compaction, when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = 90210))]
    pub cache_read_tokens: Option<u32>,
    /// Cache-creation tokens written after compaction, when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = 1024))]
    pub cache_creation_tokens: Option<u32>,
}

// ============================================================================
// File event data
// ============================================================================
