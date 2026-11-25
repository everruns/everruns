//! Compaction Capability
//!
//! Configurable context compaction strategy. Users choose between native provider
//! compaction (e.g., OpenAI /responses/compact) and our own strategies (observation
//! masking, LLM summarization).
//!
//! Design decisions:
//! - Strategy selection is per-agent/harness via `AgentCapabilityConfig`
//! - Native and our own strategies coexist as first-class options
//! - The `auto` cascade: observation masking → native → summarization
//! - Proactive compaction at a configurable budget threshold, not just on error

use super::{
    Capability, CapabilityLocalization, CapabilityStatus, ModelViewContext, ModelViewProvider,
};
use crate::builtins::events::TokenUsage;
use crate::builtins::message::{ContentPart, RuntimeMessage, RuntimeMessageRole};
use crate::builtins::message_filter::MessageFilterProvider;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
#[path = "compaction/cost_control.rs"]
mod cost_control;
use cost_control::*;
#[path = "compaction/scheduled_view.rs"]
mod scheduled_view;
#[path = "compaction/summarization.rs"]
mod summarization;
use crate::compaction_policy::CompactionPolicy;
#[path = "compaction/policy.rs"]
mod policy;
#[path = "compaction/recovery.rs"]
mod recovery;
use policy::ConfiguredCompactionPolicy;
use recovery::{answered_oversized_results, mask_tool_result_message, with_recovery_hint};
pub use summarization::{
    build_summarization_prompt, build_summary_message, compose_summary_with_recent,
    format_messages_for_summarization,
};
/// Capability ID for compaction.
pub const COMPACTION_CAPABILITY_ID: &str = "compaction";
const MAX_RELATED_RECENT_READ_RESULTS: usize = 4;

/// Compaction strategy selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CompactionStrategy {
    /// Cascade: observation masking → native → summarization → aggressive trim.
    #[default]
    Auto,
    /// Use provider's native compact endpoint only (e.g., OpenAI /responses/compact).
    Native,
    /// Strip old tool outputs, replace with one-line summaries.
    ObservationMasking,
    /// Use LLM to summarize older turns.
    Summarization,
}

impl std::fmt::Display for CompactionStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auto => write!(f, "auto"),
            Self::Native => write!(f, "native"),
            Self::ObservationMasking => write!(f, "observation_masking"),
            Self::Summarization => write!(f, "summarization"),
        }
    }
}

/// Format for masked tool output summaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MaskingSummaryFormat {
    /// `[tool_name(args_truncated) → OK]`
    #[default]
    OneLine,
    /// Keep first and last 3 lines of output.
    HeadTail,
}

/// Observation masking settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationMaskingConfig {
    /// Number of recent tool outputs to keep verbatim.
    #[serde(default = "default_keep_recent_tool_outputs")]
    pub keep_recent_tool_outputs: usize,

    /// Format for masked tool output summaries.
    #[serde(default)]
    pub summary_format: MaskingSummaryFormat,
}

impl Default for ObservationMaskingConfig {
    fn default() -> Self {
        Self {
            keep_recent_tool_outputs: default_keep_recent_tool_outputs(),
            summary_format: MaskingSummaryFormat::default(),
        }
    }
}

fn default_keep_recent_tool_outputs() -> usize {
    // Lowered from 5 to 2 (EVE-224). With EVE-221 capping exec output at 16 KiB,
    // keeping 2 recent (~8K tokens) instead of 5 (~20K tokens) significantly reduces
    // stale exec output accumulation. Older tool results are masked to one-line summaries.
    2
}

/// Cost-control masking settings.
///
/// Unlike proactive compaction, this is cost-oriented rather than
/// context-window-oriented: old bulky tool results should not stay verbatim in
/// every request just because the model still has room for them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostControlConfig {
    /// Enable low-cost tool-result masking before every LLM call.
    #[serde(default = "default_cost_control_enabled")]
    pub enabled: bool,

    /// Number of most-recent tool results to always keep verbatim.
    #[serde(default = "default_cost_control_keep_recent_tool_results")]
    pub keep_recent_tool_results: usize,

    /// Start masking once this many tool results are present.
    #[serde(default = "default_cost_control_mask_after_tool_results")]
    pub mask_after_tool_results: usize,

    /// Start masking once aggregate live tool-result payload exceeds this many bytes.
    #[serde(default = "default_cost_control_max_live_tool_result_bytes")]
    pub max_live_tool_result_bytes: usize,

    /// Never mask a tool result smaller than this many bytes.
    #[serde(default = "default_cost_control_min_masked_tool_result_bytes")]
    pub min_masked_tool_result_bytes: usize,

    /// Move the masked region forward this many tool results at a time, so the
    /// history is rewritten (and the prompt cache lost) once per batch.
    #[serde(default = "default_cost_control_mask_batch_results")]
    pub mask_batch_results: usize,

    /// Move it at once when the results waiting past it pass this many bytes.
    #[serde(default = "default_cost_control_mask_batch_bytes")]
    pub mask_batch_bytes: usize,

    /// If cumulative/session usage is available, mask when uncached input exceeds this.
    #[serde(default = "default_cost_control_max_uncached_input_tokens")]
    pub max_uncached_input_tokens: u32,

    /// If cumulative/session usage is available, mask when cache read ratio falls below this.
    #[serde(default = "default_cost_control_min_cache_read_ratio")]
    pub min_cache_read_ratio: f32,

    /// Consider durable compaction once raw tool-result history exceeds this many bytes.
    #[serde(default = "default_cost_control_compact_after_tool_result_bytes")]
    pub compact_after_tool_result_bytes: usize,

    /// Do not cost-trigger durable compaction for a smaller current prompt.
    #[serde(default = "default_cost_control_compact_min_input_tokens")]
    pub compact_min_input_tokens: usize,
}

impl Default for CostControlConfig {
    fn default() -> Self {
        Self {
            enabled: default_cost_control_enabled(),
            keep_recent_tool_results: default_cost_control_keep_recent_tool_results(),
            mask_after_tool_results: default_cost_control_mask_after_tool_results(),
            max_live_tool_result_bytes: default_cost_control_max_live_tool_result_bytes(),
            min_masked_tool_result_bytes: default_cost_control_min_masked_tool_result_bytes(),
            mask_batch_results: default_cost_control_mask_batch_results(),
            mask_batch_bytes: default_cost_control_mask_batch_bytes(),
            max_uncached_input_tokens: default_cost_control_max_uncached_input_tokens(),
            min_cache_read_ratio: default_cost_control_min_cache_read_ratio(),
            compact_after_tool_result_bytes: default_cost_control_compact_after_tool_result_bytes(),
            compact_min_input_tokens: default_cost_control_compact_min_input_tokens(),
        }
    }
}

/// Summarization settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummarizationConfig {
    /// Model to use for summarization. None = same model as agent.
    #[serde(default)]
    pub model: Option<String>,

    /// What to preserve in summaries.
    #[serde(default = "default_preserve")]
    pub preserve: Vec<String>,

    /// Custom instructions appended to summarization prompt.
    #[serde(default)]
    pub instructions: Option<String>,
}

impl Default for SummarizationConfig {
    fn default() -> Self {
        Self {
            model: None,
            preserve: default_preserve(),
            instructions: None,
        }
    }
}

fn default_preserve() -> Vec<String> {
    vec![
        "decisions".to_string(),
        "files_modified".to_string(),
        "errors".to_string(),
        "current_plan".to_string(),
        "skill_instructions".to_string(),
    ]
}

/// Fully hydrated compaction configuration used by the runtime implementation.
///
/// Framework applications should use the root-level
/// [`crate::builtins::CompactionConfig`] builder. This type represents the expanded
/// execution policy after capability JSON has been resolved, including
/// implementation-level masking and memory-tier settings.
///
/// Configured per agent/harness via `CapabilityRef`:
/// ```json
/// { "ref": "compaction", "config": { "strategy": "auto", "proactive": true } }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeCompactionConfig {
    /// Working history target for long sessions; provider hard limits still apply.
    #[serde(default = "default_working_context_tokens")]
    pub context_budget_tokens: usize,

    /// Which strategy to use.
    #[serde(default)]
    pub strategy: CompactionStrategy,

    /// Compact proactively at budget_percent, not just on RequestTooLarge.
    #[serde(default = "default_proactive")]
    pub proactive: bool,

    /// Trigger proactive compaction at this fraction of context budget.
    #[serde(default = "default_budget_percent")]
    pub budget_percent: f32,

    /// Observation masking settings.
    #[serde(default)]
    pub observation_masking: ObservationMaskingConfig,

    /// Summarization settings.
    #[serde(default)]
    pub summarization: SummarizationConfig,

    /// Hierarchical memory tier settings for hot/warm/cold management.
    #[serde(default)]
    pub memory_tiers: HierarchicalMemoryConfig,

    /// Always-on cost-oriented masking for stale tool results.
    #[serde(default)]
    pub cost_control: CostControlConfig,
}

impl Default for RuntimeCompactionConfig {
    fn default() -> Self {
        Self {
            context_budget_tokens: default_working_context_tokens(),
            strategy: CompactionStrategy::default(),
            proactive: default_proactive(),
            budget_percent: default_budget_percent(),
            observation_masking: ObservationMaskingConfig::default(),
            summarization: SummarizationConfig::default(),
            memory_tiers: HierarchicalMemoryConfig::default(),
            cost_control: CostControlConfig::default(),
        }
    }
}

fn default_working_context_tokens() -> usize {
    64_000
}

fn default_proactive() -> bool {
    true
}

fn default_budget_percent() -> f32 {
    0.85
}

impl RuntimeCompactionConfig {
    /// Parse from JSON value, falling back to defaults for invalid config.
    pub fn from_json(value: &serde_json::Value) -> Self {
        serde_json::from_value(value.clone()).unwrap_or_default()
    }
}

/// Compaction capability.
pub struct CompactionCapability;

impl Capability for CompactionCapability {
    fn id(&self) -> &str {
        COMPACTION_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Compaction"
    }

    fn dependencies(&self) -> Vec<&'static str> {
        // Legacy entry supplies retrieval and the bounded bootstrap load only;
        // its reducer defers to this capability whenever both are configured.
        vec![super::INFINITY_CONTEXT_CAPABILITY_ID]
    }

    fn provider_managed_reduction_budget(&self, config: &serde_json::Value) -> Option<usize> {
        Some(RuntimeCompactionConfig::from_json(config).context_budget_tokens)
    }

    fn description(&self) -> &str {
        r#"Configurable context compaction when conversations exceed LLM context windows.

Choose between native provider compaction (e.g., OpenAI /responses/compact), observation masking (strip old tool outputs), or LLM summarization. The `auto` strategy cascades through all available options."#
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("shrink")
    }

    fn category(&self) -> Option<&str> {
        Some("Optimization")
    }

    fn message_filter_provider(&self) -> Option<Arc<dyn MessageFilterProvider>> {
        Some(Arc::new(CompactionFilterProvider))
    }

    fn model_view_provider(&self) -> Option<Arc<dyn ModelViewProvider>> {
        Some(Arc::new(CompactionModelViewProvider))
    }

    /// Only the top-level knobs users meaningfully tune are exposed:
    /// `strategy`, `proactive`, and `budget_percent`. The nested
    /// `observation_masking` / `summarization` / `memory_tiers` /
    /// `cost_control` objects are advanced tuning with safe defaults and stay
    /// out of the schema, but `validate_config` still accepts them via the
    /// typed `RuntimeCompactionConfig` parse.
    fn config_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "context_budget_tokens": {"type": "integer", "minimum": 4096, "default": 64000, "description": "Working history budget; independent of the model context window."},
                "strategy": {
                    "type": "string",
                    "title": "Strategy",
                    "description": "Compaction strategy used when the conversation approaches the context window.",
                    "oneOf": [
                        { "const": "auto", "title": "Automatic" },
                        { "const": "native", "title": "Provider-native" },
                        { "const": "observation_masking", "title": "Observation masking" },
                        { "const": "summarization", "title": "LLM summarization" }
                    ],
                    "default": "auto"
                },
                "proactive": {
                    "type": "boolean",
                    "title": "Proactive compaction",
                    "description": "Compact at the budget threshold instead of waiting for a request-too-large error.",
                    "default": true
                },
                "budget_percent": {
                    "type": "number",
                    "title": "Context budget threshold",
                    "description": "Fraction of the model context window at which proactive compaction triggers.",
                    "minimum": 0.1,
                    "maximum": 1.0,
                    // Keep in sync with `default_budget_percent()`; the f32
                    // value is not used directly to avoid noisy f32->f64
                    // widening in the serialized schema.
                    "default": 0.85
                }
            }
        }))
    }

    fn validate_config(&self, config: &serde_json::Value) -> Result<(), String> {
        if config.is_null() {
            return Ok(());
        }
        let typed: RuntimeCompactionConfig = serde_json::from_value(config.clone())
            .map_err(|e| format!("invalid compaction config: {e}"))?;
        if typed.context_budget_tokens < 4096 {
            return Err("context_budget_tokens must be at least 4096".into());
        }
        if !(0.1..=1.0).contains(&typed.budget_percent) {
            return Err(format!(
                "budget_percent must be between 0.1 and 1.0, got {}",
                typed.budget_percent
            ));
        }
        Ok(())
    }

    fn compaction_policy(&self, config: &serde_json::Value) -> Option<Arc<dyn CompactionPolicy>> {
        Some(Arc::new(ConfiguredCompactionPolicy {
            config: RuntimeCompactionConfig::from_json(config),
        }))
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![
            CapabilityLocalization {
                locale: "en",
                name: None,
                description: None,
                config_description: Some(
                    "Controls the compaction strategy, proactive triggering, and the \
                     context-budget threshold.",
                ),
                config_overlay: None,
            },
            CapabilityLocalization {
                locale: "uk",
                name: Some("Ущільнення контексту"),
                description: Some(
                    "Налаштовуване ущільнення контексту, коли розмова перевищує контекстне \
                     вікно LLM. Доступні стратегії: нативне ущільнення провайдера, маскування \
                     результатів інструментів і підсумовування через LLM; стратегія auto \
                     перебирає всі доступні варіанти.",
                ),
                config_description: Some(
                    "Визначає стратегію ущільнення контексту, проактивний запуск і поріг \
                     бюджету контексту.",
                ),
                config_overlay: Some(serde_json::json!({
                    "properties": {
                        "strategy": {
                            "title": "Стратегія",
                            "description": "Стратегія ущільнення, коли розмова наближається до межі контекстного вікна.",
                            "enum_labels": {
                                "auto": "Автоматично",
                                "native": "Нативна (провайдер)",
                                "observation_masking": "Маскування результатів інструментів",
                                "summarization": "Підсумовування через LLM"
                            }
                        },
                        "proactive": {
                            "title": "Проактивне ущільнення",
                            "description": "Ущільнювати контекст при досягненні порогу бюджету, а не лише після помилки про завеликий запит."
                        },
                        "budget_percent": {
                            "title": "Поріг бюджету контексту",
                            "description": "Частка контекстного вікна моделі, після якої запускається проактивне ущільнення."
                        }
                    }
                })),
            },
        ]
    }
}

struct CompactionModelViewProvider;

impl ModelViewProvider for CompactionModelViewProvider {
    fn apply_model_view(
        &self,
        messages: Vec<RuntimeMessage>,
        config: &serde_json::Value,
        context: &ModelViewContext<'_>,
    ) -> Vec<RuntimeMessage> {
        if context.provider_managed_reduction {
            return messages;
        }
        let config = RuntimeCompactionConfig::from_json(config);
        let messages = scheduled_view::apply(messages);
        let masking = build_model_view_messages_owned(messages, &config, context.prior_usage);
        if masking.masked_count > 0 {
            tracing::info!(
                session_id = %context.session_id,
                masked_count = masking.masked_count,
                tool_result_bytes_before = masking.tool_result_bytes_before,
                tool_result_bytes_after = masking.tool_result_bytes_after,
                "CompactionCapability: masked stale tool results for model view"
            );
        }
        masking.messages
    }

    fn priority(&self) -> i32 {
        50
    }
}

// ============================================================================
// Message Filter Provider (proactive observation masking at message load time)
// ============================================================================

/// Applies observation masking as a message filter during message loading.
///
/// This runs *before* the LLM call, proactively reducing context size
/// by masking old tool outputs. Lower priority than infinity context (50 vs 100)
/// so it runs first — masking happens before trimming.
struct CompactionFilterProvider;

impl MessageFilterProvider for CompactionFilterProvider {
    fn apply_filters(
        &self,
        _query: &mut crate::builtins::message_filter::MessageQuery,
        _config: &serde_json::Value,
    ) {
        // The filter provider signals that compaction is active on this session.
        // Actual observation masking is applied at LLM message construction time
        // (in ReasonAtom) rather than at message query time, because masking
        // operates on the provider `Message` format, not the storage `RuntimeMessage` format.
        //
        // The proactive compaction check in ReasonAtom reads the compaction config
        // and applies masking + budget checks before the LLM call.
    }

    fn priority(&self) -> i32 {
        50 // Before infinity context (100)
    }
}

// ============================================================================
// Token Estimation
// ============================================================================

/// Estimate token count for an LLM message using char/4 approximation.
///
/// This is intentionally simple. More accurate estimation (tiktoken, etc.) can
/// be swapped in later, but char/4 is sufficient for budget decisions.
pub fn estimate_tokens(msg: &Message) -> usize {
    let text_len = match &msg.content {
        MessageContent::Text(t) => t.len(),
        MessageContent::Parts(parts) => parts
            .iter()
            .map(|p| match p {
                LlmContentPart::Text { text } => text.len(),
                _ => 50, // images, etc. — rough estimate
            })
            .sum(),
    };

    // Add tool call overhead
    let tool_call_len = msg
        .tool_calls
        .as_ref()
        .map(|calls| {
            calls
                .iter()
                .map(|tc| tc.name.len() + tc.arguments.to_string().len() + 20)
                .sum::<usize>()
        })
        .unwrap_or(0);

    (text_len + tool_call_len) / 4
}

/// Estimate total tokens for a slice of messages.
pub fn estimate_total_tokens(messages: &[Message]) -> usize {
    messages.iter().map(estimate_tokens).sum()
}

/// Check whether proactive compaction should trigger.
///
/// Returns `true` if the estimated tokens exceed `budget_percent` of the model's
/// context window.
pub fn should_compact_proactively(
    messages: &[Message],
    config: &RuntimeCompactionConfig,
    context_window_tokens: usize,
) -> bool {
    if !config.proactive {
        return false;
    }
    let budget = (context_window_tokens as f32 * config.budget_percent) as usize;
    let estimated = estimate_total_tokens(messages);
    estimated > budget
}

/// Check whether current raw tool evidence warrants a durable checkpoint.
///
/// The marginal prompt floor prevents a large lifetime counter from compacting
/// every short follow-up. Checkpoint suffix re-arming and retry watermarks remain
/// owned by the reason atom, so this predicate is pure and restart-safe.
pub fn should_compact_for_cost(
    estimated_input_tokens: usize,
    raw_tool_result_bytes: usize,
    config: &RuntimeCompactionConfig,
    _prior_usage: Option<&TokenUsage>,
) -> bool {
    if !config.proactive
        || !config.cost_control.enabled
        || estimated_input_tokens < config.cost_control.compact_min_input_tokens
    {
        return false;
    }

    // A lifetime usage counter cannot measure the remaining work in this epoch.
    // Window/working-budget pressure handles text; current raw evidence handles cost.
    raw_tool_result_bytes >= config.cost_control.compact_after_tool_result_bytes
}

// ============================================================================
// Aggressive Trim (last resort in cascade)
// ============================================================================

/// Drop oldest messages to fit within a target token count.
///
/// Preserves the system prompt (index 0 if present), protected messages
/// (e.g. `activate_skill` results and their tool call messages), and the
/// most recent messages. This is the last resort — lossy, no recovery.
pub fn aggressive_trim(
    messages: &[Message],
    target_tokens: usize,
    has_system_prompt: bool,
) -> Vec<Message> {
    let mut result = Vec::new();
    let mut token_budget = target_tokens;

    // Always keep system prompt
    let start_idx = if has_system_prompt && !messages.is_empty() {
        let sys_tokens = estimate_tokens(&messages[0]);
        if sys_tokens < token_budget {
            result.push(messages[0].clone());
            token_budget -= sys_tokens;
        }
        1
    } else {
        0
    };

    let conversation = &messages[start_idx..];

    // Identify protected messages (skill tool results and their call messages).
    // Reserve budget for them first so they are never dropped.
    let mut protected_indices: std::collections::HashSet<usize> = conversation
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            is_protected_tool_result(conversation, m) || is_protected_tool_call_message(m)
        })
        .map(|(i, _)| i)
        .collect();

    // Anchor the first conversation message (the original task / goal) by
    // adding it to the protected set, so its budget is reserved before any
    // non-protected message — like infinity context's head anchor, this is the
    // eviction we most want to avoid once the window slides. Under extreme
    // pressure (protected messages alone exceed the budget) the oldest
    // protected messages, including this one, may still be dropped, matching how
    // protected tool results are handled.
    if !conversation.is_empty() {
        protected_indices.insert(0);
    }
    if let Some(latest) = conversation
        .iter()
        .rposition(|message| message.role == MessageRole::User)
    {
        protected_indices.insert(latest);
    }

    let mut protected_budget: usize = 0;
    for &idx in &protected_indices {
        protected_budget += estimate_tokens(&conversation[idx]);
    }

    // If protected messages alone exceed the remaining budget, keep as many
    // protected messages as possible (newest first) and skip non-protected.
    if protected_budget > token_budget {
        let mut protected_with_indices: Vec<(usize, Message)> = protected_indices
            .iter()
            .map(|&idx| (idx, conversation[idx].clone()))
            .collect();
        protected_with_indices.sort_by_key(|(i, _)| *i);

        let mut remaining = token_budget;
        let mut kept: Vec<(usize, Message)> = Vec::new();
        for (idx, msg) in protected_with_indices.into_iter().rev() {
            let t = estimate_tokens(&msg);
            if t <= remaining {
                kept.push((idx, msg));
                remaining -= t;
            }
        }
        kept.sort_by_key(|(i, _)| *i);
        result.extend(kept.into_iter().map(|(_, m)| m));
        return crate::builtins::retain_complete_llm_tool_exchanges(result);
    }

    token_budget -= protected_budget;

    // Walk from newest to oldest, collecting non-protected messages that fit
    let mut keep_from_end = Vec::new();
    for (i, msg) in conversation.iter().enumerate().rev() {
        if protected_indices.contains(&i) {
            continue; // handled separately
        }
        let msg_tokens = estimate_tokens(msg);
        if msg_tokens <= token_budget {
            keep_from_end.push((i, msg.clone()));
            token_budget -= msg_tokens;
        } else {
            break;
        }
    }

    // Merge protected + kept messages in original order
    let mut all_kept: Vec<(usize, Message)> = Vec::new();
    for &idx in &protected_indices {
        all_kept.push((idx, conversation[idx].clone()));
    }
    all_kept.extend(keep_from_end);
    all_kept.sort_by_key(|(i, _)| *i);

    result.extend(all_kept.into_iter().map(|(_, m)| m));
    crate::builtins::retain_complete_llm_tool_exchanges(result)
}

// ============================================================================
// Session Compaction Metrics
// ============================================================================

/// Per-session compaction metrics, stored as session metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionCompactionMetrics {
    /// Total number of compaction events in this session.
    pub compaction_count: u32,
    /// Total messages saved across all compactions.
    pub total_messages_saved: u64,
    /// Breakdown by strategy.
    pub strategy_counts: HashMap<String, u32>,
    /// Total time spent compacting (ms).
    pub total_duration_ms: u64,
}

impl SessionCompactionMetrics {
    /// Record a completed compaction step.
    pub fn record(
        &mut self,
        strategy_used: &str,
        messages_before: usize,
        messages_after: usize,
        duration_ms: u64,
    ) {
        self.compaction_count += 1;
        self.total_messages_saved += (messages_before.saturating_sub(messages_after)) as u64;
        self.total_duration_ms += duration_ms;

        for strategy in strategy_used.split('+') {
            *self
                .strategy_counts
                .entry(strategy.to_string())
                .or_insert(0) += 1;
        }
    }
}

// ============================================================================
// Hierarchical Memory Tiers
// ============================================================================

/// Memory tier for a message in the hierarchy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryTier {
    /// Full verbatim text, always in context.
    Hot,
    /// Observation-masked (tool outputs replaced with summaries).
    Warm,
    /// Summarized to key facts. Queryable via `query_history` if Infinity Context enabled.
    Cold,
}

/// Configuration for hierarchical memory tiers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HierarchicalMemoryConfig {
    /// Number of most recent messages to keep in the hot tier (full verbatim).
    #[serde(default = "default_hot_messages")]
    pub hot_messages: usize,
    /// Number of messages in the warm tier (observation-masked).
    #[serde(default = "default_warm_messages")]
    pub warm_messages: usize,
    // Everything older → cold tier (summarized / queryable)
}

impl Default for HierarchicalMemoryConfig {
    fn default() -> Self {
        Self {
            hot_messages: default_hot_messages(),
            warm_messages: default_warm_messages(),
        }
    }
}

fn default_hot_messages() -> usize {
    20
}

fn default_warm_messages() -> usize {
    100
}

/// Classify messages into memory tiers based on position (newest-first).
///
/// Returns a vec of (tier, message) pairs in original order.
pub fn classify_memory_tiers<'a>(
    messages: &'a [Message],
    config: &HierarchicalMemoryConfig,
) -> Vec<(MemoryTier, &'a Message)> {
    let len = messages.len();
    messages
        .iter()
        .enumerate()
        .map(|(i, msg)| {
            let from_end = len - 1 - i;
            let tier = if from_end < config.hot_messages {
                MemoryTier::Hot
            } else if from_end < config.hot_messages + config.warm_messages {
                MemoryTier::Warm
            } else {
                MemoryTier::Cold
            };
            (tier, msg)
        })
        .collect()
}

/// Apply hierarchical memory: mask warm-tier tool outputs, summarize cold tier.
///
/// Returns the processed messages ready for LLM context. Cold-tier messages are
/// replaced with a `[CONVERSATION_SUMMARY]` if a summary is provided.
///
/// Protected messages (e.g. `activate_skill` results) in cold/warm tiers are
/// promoted to the output verbatim — they are never dropped or masked.
pub fn apply_hierarchical_memory(
    messages: &[Message],
    config: &HierarchicalMemoryConfig,
    masking_config: &ObservationMaskingConfig,
    cold_summary: Option<&str>,
) -> Vec<Message> {
    let len = messages.len();
    let hot_start = len.saturating_sub(config.hot_messages);
    let warm_start = hot_start.saturating_sub(config.warm_messages);

    let mut result = Vec::new();

    // Cold tier: replace with summary if available, but rescue protected messages
    if warm_start > 0 {
        // Extract protected messages from cold tier before dropping
        let cold_msgs = &messages[..warm_start];
        let protected_cold: Vec<Message> = cold_msgs
            .iter()
            .filter(|m| is_protected_tool_result(cold_msgs, m) || is_protected_tool_call_message(m))
            .cloned()
            .collect();

        if let Some(summary) = cold_summary {
            result.push(build_summary_message(summary));
        }

        // Re-insert protected messages after the summary
        result.extend(protected_cold);
    }

    // Warm tier: apply observation masking to tool outputs.
    // Use the full message slice for protected-tool detection so that a tool
    // result in warm tier whose assistant call is in cold tier is still recognized.
    if warm_start < hot_start {
        let warm_msgs = &messages[warm_start..hot_start];

        // Pre-identify protected tool_call_ids using the full message list
        let protected_call_ids: std::collections::HashSet<String> = warm_msgs
            .iter()
            .filter(|m| is_protected_tool_result(messages, m))
            .filter_map(|m| m.tool_call_id.clone())
            .collect();

        let masked = apply_observation_masking_with_protected(
            warm_msgs,
            masking_config,
            &protected_call_ids,
        );
        result.extend(masked.messages);
    }

    // Hot tier: verbatim
    if hot_start < len {
        result.extend_from_slice(&messages[hot_start..]);
    }

    crate::builtins::retain_complete_llm_tool_exchanges(result)
}

// ============================================================================
// Protected Tool Detection
// ============================================================================

use crate::builtins::driver_registry::{LlmContentPart, Message, MessageContent, MessageRole};

/// Tool names whose results must be protected from compaction.
///
/// Skill activation results contain durable behavioral instructions that silently
/// degrade agent behavior when masked, summarized, or trimmed. The agentskills.io
/// client implementation guide recommends exempting skill content from pruning.
///
// Design: knowledge/runtime-resources/compaction.md (Tier 3: tool-aware masking), knowledge/project/skills-registry.md
const PROTECTED_TOOL_NAMES: &[&str] = &["activate_skill"];

/// Check if a tool result message corresponds to a protected tool.
///
/// Looks up the tool_call_id in preceding assistant messages to find the tool name.
/// Returns `true` if the tool name is in `PROTECTED_TOOL_NAMES`.
fn is_protected_tool_result(messages: &[Message], tool_msg: &Message) -> bool {
    if tool_msg.role != MessageRole::Tool {
        return false;
    }
    let tool_name = find_tool_call_name(messages, tool_msg);
    PROTECTED_TOOL_NAMES.contains(&tool_name.as_str())
}

/// Check if an assistant message contains a tool call to a protected tool.
///
/// Returns `true` if any tool call in the message targets a protected tool name.
fn is_protected_tool_call_message(msg: &Message) -> bool {
    if msg.role != MessageRole::Assistant {
        return false;
    }
    msg.tool_calls.as_ref().is_some_and(|calls| {
        calls
            .iter()
            .any(|tc| PROTECTED_TOOL_NAMES.contains(&tc.name.as_str()))
    })
}

// ============================================================================
// Observation Masking
// ============================================================================

/// Result of applying observation masking to a message list.
#[derive(Debug)]
pub struct ObservationMaskingResult {
    /// The masked messages.
    pub messages: Vec<Message>,
    /// Number of tool outputs that were masked.
    pub masked_count: usize,
}

/// Apply observation masking: replace old tool outputs with one-line summaries.
///
/// Keeps the last `keep_recent_tool_outputs` tool results verbatim and replaces
/// older ones with compact summaries. Message count is preserved (replace, not remove).
///
/// Protected tool results (e.g. `activate_skill`) are never masked — they contain
/// durable behavioral instructions that must survive compaction.
pub fn apply_observation_masking(
    messages: &[Message],
    config: &ObservationMaskingConfig,
) -> ObservationMaskingResult {
    apply_observation_masking_with_protected(messages, config, &std::collections::HashSet::new())
}

/// Result of cost-control masking applied before provider serialization.
#[derive(Debug)]
pub struct CostControlMaskingResult {
    /// Messages after stale bulky tool results were replaced by summaries.
    pub messages: Vec<RuntimeMessage>,
    /// Number of tool-result messages that were masked.
    pub masked_count: usize,
    /// Tool-result payload bytes before masking.
    pub tool_result_bytes_before: usize,
    /// Tool-result payload bytes after masking.
    pub tool_result_bytes_after: usize,
}

/// Build the bounded model-view messages from lossless stored messages.
///
/// Storage keeps full tool results. This helper defines the cheaper prompt
/// view used for provider serialization when the compaction capability is
/// configured.
pub fn build_model_view_messages(
    stored_messages: &[RuntimeMessage],
    compaction_config: &RuntimeCompactionConfig,
    prior_usage: Option<&TokenUsage>,
) -> CostControlMaskingResult {
    apply_cost_control_masking(stored_messages, compaction_config, prior_usage)
}

/// Build the bounded model-view messages from owned stored messages.
///
/// This avoids cloning the message list when masking does not apply.
pub fn build_model_view_messages_owned(
    stored_messages: Vec<RuntimeMessage>,
    compaction_config: &RuntimeCompactionConfig,
    prior_usage: Option<&TokenUsage>,
) -> CostControlMaskingResult {
    apply_cost_control_masking_owned(stored_messages, compaction_config, prior_usage)
}

/// Apply cheap, generic cost-control masking to stored messages.
///
/// This runs before converting messages to provider-specific LLM messages, so
/// the llm.generation event can reflect the context actually sent. It is
/// deliberately separate from observation masking: observation masking is part
/// of the context-window compaction cascade, while this keeps stale tool output
/// from being paid for repeatedly even when a large-context model still has
/// room.
pub fn apply_cost_control_masking(
    messages: &[RuntimeMessage],
    config: &RuntimeCompactionConfig,
    prior_usage: Option<&TokenUsage>,
) -> CostControlMaskingResult {
    apply_cost_control_masking_owned(messages.to_vec(), config, prior_usage)
}

fn apply_cost_control_masking_owned(
    messages: Vec<RuntimeMessage>,
    config: &RuntimeCompactionConfig,
    prior_usage: Option<&TokenUsage>,
) -> CostControlMaskingResult {
    let cost_config = &config.cost_control;
    let tool_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| {
            message.role == RuntimeMessageRole::ToolResult
                && !is_protected_message_tool_result(&messages, message)
        })
        .map(|(index, _)| index)
        .collect();
    let tool_result_bytes_before = tool_indices
        .iter()
        .map(|index| message_tool_result_len(&messages[*index]))
        .sum();

    // An oversized result the model already answered is masked even among
    // the recent ones (see `answered_oversized`).
    let answered_oversized = answered_oversized_results(&messages, &tool_indices, cost_config);
    if !cost_config.enabled
        || (tool_indices.len() <= cost_config.keep_recent_tool_results
            && answered_oversized.is_empty())
        || !should_apply_cost_control_masking(
            tool_indices.len(),
            tool_result_bytes_before,
            cost_config,
            prior_usage,
        )
    {
        return CostControlMaskingResult {
            messages,
            masked_count: 0,
            tool_result_bytes_before,
            tool_result_bytes_after: tool_result_bytes_before,
        };
    }

    let keep_recent = cost_config.keep_recent_tool_results;
    let sizes: Vec<usize> = tool_indices
        .iter()
        .map(|index| message_tool_result_len(&messages[*index]))
        .collect();
    let to_mask_count = mask_boundary(&sizes, cost_config);
    let related_recent_reads =
        related_recent_paginated_read_results(&messages, &tool_indices, keep_recent);
    let indices_to_mask: HashSet<usize> = tool_indices[..to_mask_count]
        .iter()
        .copied()
        .filter(|index| !related_recent_reads.contains(index))
        .chain(answered_oversized)
        .filter(|index| {
            message_tool_result_len(&messages[*index]) >= cost_config.min_masked_tool_result_bytes
        })
        .collect();
    let tool_names: std::collections::HashMap<usize, String> = indices_to_mask
        .iter()
        .map(|index| {
            (
                *index,
                find_message_tool_call_name(&messages, &messages[*index]),
            )
        })
        .collect();

    let mut masked_count = 0;
    let mut masked_messages = Vec::with_capacity(messages.len());
    for (index, message) in messages.into_iter().enumerate() {
        if let Some(tool_name) = tool_names.get(&index) {
            masked_messages.push(mask_tool_result_message(&message, tool_name));
            masked_count += 1;
        } else {
            masked_messages.push(message);
        }
    }

    let tool_result_bytes_after = masked_messages
        .iter()
        .filter(|message| message.role == RuntimeMessageRole::ToolResult)
        .map(message_tool_result_len)
        .sum();

    CostControlMaskingResult {
        messages: masked_messages,
        masked_count,
        tool_result_bytes_before,
        tool_result_bytes_after,
    }
}

fn should_apply_cost_control_masking(
    tool_result_count: usize,
    tool_result_bytes: usize,
    config: &CostControlConfig,
    prior_usage: Option<&TokenUsage>,
) -> bool {
    if tool_result_count >= config.mask_after_tool_results {
        return true;
    }
    if tool_result_bytes >= config.max_live_tool_result_bytes {
        return true;
    }
    let Some(usage) = prior_usage else {
        return false;
    };
    // Token buckets are disjoint (see `TokenUsage`): `input_tokens` already
    // carries only the non-cached prompt, and the cache-read ratio is measured
    // against the full prompt (all buckets summed).
    let cache_read = usage.cache_read_tokens.unwrap_or(0);
    let cache_creation = usage.cache_creation_tokens.unwrap_or(0);
    if usage.input_tokens >= config.max_uncached_input_tokens {
        return true;
    }
    let total_prompt = usage.input_tokens + cache_read + cache_creation;
    total_prompt > 0 && (cache_read as f32 / total_prompt as f32) < config.min_cache_read_ratio
}

fn is_protected_message_tool_result(
    messages: &[RuntimeMessage],
    tool_msg: &RuntimeMessage,
) -> bool {
    if tool_msg.role != RuntimeMessageRole::ToolResult {
        return false;
    }
    let tool_name = find_message_tool_call_name(messages, tool_msg);
    PROTECTED_TOOL_NAMES.contains(&tool_name.as_str())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ReadResultKey {
    tool_name: String,
    path: String,
    content_hash: String,
}

fn related_recent_paginated_read_results(
    messages: &[RuntimeMessage],
    tool_indices: &[usize],
    keep_recent: usize,
) -> HashSet<usize> {
    if keep_recent == 0 || tool_indices.len() <= keep_recent {
        return HashSet::new();
    }

    let keep_start = tool_indices.len().saturating_sub(keep_recent);
    let recent_keys: HashSet<ReadResultKey> = tool_indices[keep_start..]
        .iter()
        .filter_map(|index| paginated_read_result_key(messages, &messages[*index]))
        .collect();
    if recent_keys.is_empty() {
        return HashSet::new();
    }

    let mut protected = HashSet::new();
    for index in tool_indices[..keep_start].iter().rev() {
        let Some(key) = paginated_read_result_key(messages, &messages[*index]) else {
            break;
        };
        if !recent_keys.contains(&key) {
            break;
        }
        protected.insert(*index);
        if protected.len() >= MAX_RELATED_RECENT_READ_RESULTS {
            break;
        }
    }
    protected
}

fn paginated_read_result_key(
    messages: &[RuntimeMessage],
    tool_msg: &RuntimeMessage,
) -> Option<ReadResultKey> {
    let tool_name = find_message_tool_call_name(messages, tool_msg);
    if !is_read_file_tool_name(&tool_name) {
        return None;
    }
    let value = tool_msg.tool_result_content()?.result.as_ref()?;
    let object = value.as_object()?;
    object.get("lines_shown").and_then(|v| v.as_object())?;
    Some(ReadResultKey {
        tool_name,
        path: object.get("path")?.as_str()?.to_string(),
        content_hash: object.get("content_hash")?.as_str()?.to_string(),
    })
}

fn is_read_file_tool_name(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "read_file"
            | "daytona_read_file"
            | "sandbox_read_file"
            | "e2b_read_file"
            | "docker_read_file"
            | "read_github_file"
    )
}

fn find_message_tool_call_name(messages: &[RuntimeMessage], tool_msg: &RuntimeMessage) -> String {
    let Some(call_id) = tool_msg.tool_call_id() else {
        return "unknown_tool".to_string();
    };

    for msg in messages.iter().rev() {
        if msg.role != RuntimeMessageRole::Agent {
            continue;
        }
        for tool_call in msg.tool_calls() {
            if tool_call.id == call_id {
                return tool_call.name.clone();
            }
        }
    }

    "unknown_tool".to_string()
}

fn message_tool_result_len(message: &RuntimeMessage) -> usize {
    let Some(result) = message.tool_result_content() else {
        return 0;
    };
    result
        .result
        .as_ref()
        .map(estimate_json_value_len)
        .unwrap_or(0)
        + result.error.as_ref().map_or(0, String::len)
}

/// Aggregate raw tool-result payload bytes without allocating or overflowing.
pub fn total_tool_result_bytes(messages: &[RuntimeMessage]) -> usize {
    messages
        .iter()
        .filter(|message| message.role == RuntimeMessageRole::ToolResult)
        .fold(0usize, |total, message| {
            total.saturating_add(message_tool_result_len(message))
        })
}

fn summarize_tool_result(
    tool_name: &str,
    result: Option<&serde_json::Value>,
    error: Option<&String>,
) -> String {
    if let Some(error) = error {
        return format!("[{tool_name} error: {}]", truncate_inline(error, 160));
    }
    let Some(value) = result else {
        return format!("[{tool_name} returned no result]");
    };
    let Some(object) = value.as_object() else {
        return format!(
            "[{tool_name} -> {}, {} bytes]",
            value_kind(value),
            estimate_json_value_len(value)
        );
    };

    match tool_name {
        tool_name if is_read_file_tool_name(tool_name) => {
            summarize_read_file_result(tool_name, object, value)
        }
        "bash" | "daytona_exec" | "sandbox_exec" | "e2b_exec" | "docker_exec" => {
            summarize_exec_result(tool_name, object, value)
        }
        "list_directory" => summarize_list_directory_result(tool_name, object, value),
        "grep_files" => summarize_grep_files_result(tool_name, object, value),
        _ => summarize_generic_tool_result(tool_name, object, value),
    }
}

fn summarize_read_file_result(
    tool_name: &str,
    object: &serde_json::Map<String, serde_json::Value>,
    value: &serde_json::Value,
) -> String {
    let path = object
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("(unknown path)");
    let lines = object.get("lines_shown").and_then(|v| v.as_object());
    let line_range = lines
        .and_then(|lines| {
            let start = lines.get("start")?.as_u64()?;
            let end = lines.get("end")?.as_u64()?;
            Some(format!(" lines {start}-{end}"))
        })
        .unwrap_or_default();
    let total_lines = object
        .get("total_lines")
        .and_then(|v| v.as_u64())
        .map(|lines| format!(", total_lines={lines}"))
        .unwrap_or_default();
    let next_offset = object
        .get("truncation")
        .and_then(|v| v.as_object())
        .and_then(|truncation| truncation.get("next_offset"))
        .and_then(|v| v.as_u64())
        .map(|offset| format!(", next_offset={offset}"))
        .unwrap_or_default();
    let hash = object
        .get("content_hash")
        .and_then(|v| v.as_str())
        .map(|hash| format!(", hash={hash}"))
        .unwrap_or_default();
    let truncated = object
        .get("truncated")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    format!(
        "[{tool_name} {path}{line_range}, {} bytes, truncated={truncated}{total_lines}{next_offset}{hash}]",
        estimate_json_value_len(value)
    )
}

fn summarize_exec_result(
    tool_name: &str,
    object: &serde_json::Map<String, serde_json::Value>,
    value: &serde_json::Value,
) -> String {
    let exit = object
        .get("exit_code")
        .and_then(|v| v.as_i64())
        .map(|code| format!(" exit={code}"))
        .unwrap_or_default();
    let stdout_len = object
        .get("stdout")
        .and_then(|v| v.as_str())
        .map(|stdout| stdout.len())
        .unwrap_or(0);
    let stderr_len = object
        .get("stderr")
        .and_then(|v| v.as_str())
        .map(|stderr| stderr.len())
        .unwrap_or(0);
    let full_output = object
        .get("full_output")
        .and_then(|v| v.as_str())
        .map(|path| format!(", full_output={path}"))
        .unwrap_or_default();
    let total_lines = object
        .get("total_lines")
        .and_then(|v| v.as_u64())
        .map(|lines| format!(", total_lines={lines}"))
        .unwrap_or_default();

    format!(
        "[{tool_name}{exit}, stdout={} bytes, stderr={} bytes, result={} bytes{full_output}{total_lines}]",
        stdout_len,
        stderr_len,
        estimate_json_value_len(value)
    )
}

fn summarize_list_directory_result(
    tool_name: &str,
    object: &serde_json::Map<String, serde_json::Value>,
    value: &serde_json::Value,
) -> String {
    let path = object
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("(unknown path)");
    let count = object
        .get("count")
        .and_then(|v| v.as_u64())
        .or_else(|| {
            object
                .get("entries")
                .and_then(|v| v.as_array())
                .map(|v| v.len() as u64)
        })
        .unwrap_or(0);
    format!(
        "[{tool_name} {path}, {count} entries, {} bytes]",
        estimate_json_value_len(value)
    )
}

fn summarize_grep_files_result(
    tool_name: &str,
    object: &serde_json::Map<String, serde_json::Value>,
    value: &serde_json::Value,
) -> String {
    let pattern = object
        .get("pattern")
        .and_then(|v| v.as_str())
        .map(|pattern| format!(" pattern={:?}", truncate_inline(pattern, 80)))
        .unwrap_or_default();
    let match_count = object
        .get("match_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    format!(
        "[{tool_name}{pattern}, matches={match_count}, {} bytes]",
        estimate_json_value_len(value)
    )
}

fn summarize_generic_tool_result(
    tool_name: &str,
    object: &serde_json::Map<String, serde_json::Value>,
    value: &serde_json::Value,
) -> String {
    let keys = object.keys().take(5).cloned().collect::<Vec<_>>().join(",");
    format!(
        "[{tool_name} result, {} bytes, keys={keys}]",
        estimate_json_value_len(value)
    )
}

fn value_kind(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

fn estimate_json_value_len(value: &serde_json::Value) -> usize {
    let mut writer = CountingWriter::default();
    serde_json::to_writer(&mut writer, value)
        .map(|_| writer.bytes)
        .unwrap_or(0)
}

#[derive(Default)]
struct CountingWriter {
    bytes: usize,
}

impl std::io::Write for CountingWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.bytes += buf.len();
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn truncate_inline(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut truncated = text.chars().take(max_chars).collect::<String>();
    truncated.push_str("...");
    truncated
}

/// Like `apply_observation_masking`, but accepts additional pre-identified protected
/// tool_call_ids. This is needed when the message slice doesn't contain the
/// assistant tool-call message (e.g. warm tier where the call is in cold tier).
fn apply_observation_masking_with_protected(
    messages: &[Message],
    config: &ObservationMaskingConfig,
    extra_protected_call_ids: &std::collections::HashSet<String>,
) -> ObservationMaskingResult {
    // Separate protected vs maskable tool result indices
    let tool_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            m.role == MessageRole::Tool
                && !is_protected_tool_result(messages, m)
                && !m
                    .tool_call_id
                    .as_ref()
                    .is_some_and(|id| extra_protected_call_ids.contains(id))
        })
        .map(|(i, _)| i)
        .collect();

    if tool_indices.len() <= config.keep_recent_tool_outputs {
        return ObservationMaskingResult {
            messages: messages.to_vec(),
            masked_count: 0,
        };
    }

    let to_mask_count = tool_indices.len() - config.keep_recent_tool_outputs;
    let indices_to_mask: std::collections::HashSet<usize> =
        tool_indices[..to_mask_count].iter().copied().collect();

    let mut result = Vec::with_capacity(messages.len());
    let mut masked_count = 0;

    for (i, msg) in messages.iter().enumerate() {
        if indices_to_mask.contains(&i) {
            let tool_name = find_tool_call_name(messages, msg);
            let summary = match config.summary_format {
                MaskingSummaryFormat::OneLine => format_one_line_summary(&tool_name, &msg.content),
                MaskingSummaryFormat::HeadTail => format_head_tail_summary(&msg.content),
            };
            result.push(Message {
                native_tool_calls: Vec::new(),
                role: MessageRole::Tool,
                content: MessageContent::Text(summary),
                tool_calls: msg.tool_calls.clone(),
                tool_call_id: msg.tool_call_id.clone(),
                phase: msg.phase,
                reasoning: Vec::new(),
                configuration_update: None,
                tool_changes: None,
            });
            masked_count += 1;
        } else {
            result.push(msg.clone());
        }
    }

    ObservationMaskingResult {
        messages: result,
        masked_count,
    }
}

/// Find the tool name from a preceding assistant message that issued the tool call.
fn find_tool_call_name(messages: &[Message], tool_msg: &Message) -> String {
    let Some(ref call_id) = tool_msg.tool_call_id else {
        return "unknown_tool".to_string();
    };

    for msg in messages.iter().rev() {
        if msg.role == MessageRole::Assistant
            && let Some(ref tool_calls) = msg.tool_calls
        {
            for tc in tool_calls {
                if tc.id == *call_id {
                    return tc.name.clone();
                }
            }
        }
    }

    "unknown_tool".to_string()
}

fn extract_text(content: &MessageContent) -> String {
    match content {
        MessageContent::Text(t) => t.clone(),
        MessageContent::Parts(parts) => parts
            .iter()
            .filter_map(|p| match p {
                LlmContentPart::Text { text } => Some(text.clone()),
                other => crate::attachment_labels::attachment_label(other),
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn format_one_line_summary(tool_name: &str, content: &MessageContent) -> String {
    let text = extract_text(content);
    let line_count = text.lines().count();
    let byte_len = text.len();

    if byte_len <= 100 {
        format!("[{tool_name} → {text}]")
    } else {
        format!("[{tool_name} → {line_count} lines, {byte_len} bytes]")
    }
}

fn format_head_tail_summary(content: &MessageContent) -> String {
    let text = extract_text(content);
    let lines: Vec<&str> = text.lines().collect();

    if lines.len() <= 6 {
        return text;
    }

    let head: Vec<&str> = lines[..3].to_vec();
    let tail: Vec<&str> = lines[lines.len() - 3..].to_vec();

    format!(
        "{}\n... ({} lines omitted) ...\n{}",
        head.join("\n"),
        lines.len() - 6,
        tail.join("\n")
    )
}

// ============================================================================
// Summarization
// ============================================================================

// ============================================================================
// Compaction Step Tracking
// ============================================================================

/// Record of a single compaction step in a cascade.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionStep {
    /// Strategy used in this step.
    pub strategy: String,
    /// Message count after this step.
    pub messages_after: usize,
    /// Duration of this step in milliseconds.
    pub duration_ms: u64,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[path = "compaction/provider_managed_tests.rs"]
mod provider_managed_tests;

#[cfg(test)]
#[path = "compaction/cost_control_tests.rs"]
mod cost_control_tests;

#[cfg(test)]
#[path = "compaction/tests.rs"]
mod tests;
