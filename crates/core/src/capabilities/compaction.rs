//! Compaction Capability
//!
//! Configurable context compaction strategy. Users choose between native provider
//! compaction (e.g., OpenAI /responses/compact) and our own strategies (observation
//! masking, LLM summarization). See knowledge/runtime-resources/compaction.md.
//!
//! Design decisions:
//! - Strategy selection is per-agent/harness via `AgentCapabilityConfig`
//! - Native and our own strategies coexist as first-class options
//! - The `auto` cascade: observation masking → native → summarization
//! - Proactive compaction at a configurable budget threshold, not just on error

use super::{
    Capability, CapabilityLocalization, CapabilityStatus, ModelViewContext, ModelViewProvider,
};
use crate::events::TokenUsage;
use crate::message::{ContentPart, Message, MessageRole};
use crate::message_filter::MessageFilterProvider;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::compaction_policy::{
    CompactionPolicy, CompactionSettings, CompactionStrategy as ExecutionCompactionStrategy,
    ObservationMaskingResult as ExecutionObservationMaskingResult,
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
            max_uncached_input_tokens: default_cost_control_max_uncached_input_tokens(),
            min_cache_read_ratio: default_cost_control_min_cache_read_ratio(),
            compact_after_tool_result_bytes: default_cost_control_compact_after_tool_result_bytes(),
            compact_min_input_tokens: default_cost_control_compact_min_input_tokens(),
        }
    }
}

fn default_cost_control_enabled() -> bool {
    true
}

fn default_cost_control_keep_recent_tool_results() -> usize {
    2
}

fn default_cost_control_mask_after_tool_results() -> usize {
    4
}

fn default_cost_control_max_live_tool_result_bytes() -> usize {
    24 * 1024
}

fn default_cost_control_max_uncached_input_tokens() -> u32 {
    100_000
}

fn default_cost_control_min_cache_read_ratio() -> f32 {
    0.35
}

fn default_cost_control_compact_after_tool_result_bytes() -> usize {
    256 * 1024
}

fn default_cost_control_compact_min_input_tokens() -> usize {
    8 * 1024
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

/// Compaction capability configuration.
///
/// Configured per agent/harness via `AgentCapabilityConfig`:
/// ```json
/// { "ref": "compaction", "config": { "strategy": "auto", "proactive": true } }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionConfig {
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

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
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

fn default_proactive() -> bool {
    true
}

fn default_budget_percent() -> f32 {
    0.85
}

impl CompactionConfig {
    /// Parse from JSON value, falling back to defaults for invalid config.
    pub fn from_json(value: &serde_json::Value) -> Self {
        serde_json::from_value(value.clone()).unwrap_or_default()
    }
}

/// Compaction capability.
pub struct CompactionCapability;

#[derive(Debug)]
struct ConfiguredCompactionPolicy {
    config: CompactionConfig,
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

    fn estimate_total_tokens(&self, messages: &[LlmMessage]) -> usize {
        estimate_total_tokens(messages)
    }

    fn should_compact_proactively(&self, messages: &[LlmMessage], context_window: usize) -> bool {
        should_compact_proactively(messages, &self.config, context_window)
    }

    fn should_compact_for_cost(
        &self,
        estimated_input_tokens: usize,
        raw_tool_result_bytes: usize,
        usage: Option<&crate::events::TokenUsage>,
    ) -> bool {
        should_compact_for_cost(
            estimated_input_tokens,
            raw_tool_result_bytes,
            &self.config,
            usage,
        )
    }

    fn apply_observation_masking(
        &self,
        messages: &[LlmMessage],
    ) -> ExecutionObservationMaskingResult {
        let result = apply_observation_masking(messages, &self.config.observation_masking);
        ExecutionObservationMaskingResult {
            messages: result.messages,
            masked_count: result.masked_count,
        }
    }

    fn aggressive_trim(
        &self,
        messages: &[LlmMessage],
        target_tokens: usize,
        preserve_system: bool,
    ) -> Vec<LlmMessage> {
        aggressive_trim(messages, target_tokens, preserve_system)
    }

    fn summarization_prompt(&self) -> String {
        build_summarization_prompt(&self.config.summarization)
    }

    fn format_messages_for_summarization(&self, messages: &[LlmMessage]) -> String {
        format_messages_for_summarization(messages)
    }

    fn compose_summary_with_recent(
        &self,
        system_message: Option<LlmMessage>,
        summary_text: &str,
        recent_messages: &[LlmMessage],
    ) -> Vec<LlmMessage> {
        compose_summary_with_recent(system_message, summary_text, recent_messages)
    }
}

impl Capability for CompactionCapability {
    fn id(&self) -> &str {
        COMPACTION_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Compaction"
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
    /// typed `CompactionConfig` parse.
    fn config_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
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
        let typed: CompactionConfig = serde_json::from_value(config.clone())
            .map_err(|e| format!("invalid compaction config: {e}"))?;
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
            config: CompactionConfig::from_json(config),
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
        messages: Vec<Message>,
        config: &serde_json::Value,
        context: &ModelViewContext<'_>,
    ) -> Vec<Message> {
        let config = CompactionConfig::from_json(config);
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
        _query: &mut crate::message_filter::MessageQuery,
        _config: &serde_json::Value,
    ) {
        // The filter provider signals that compaction is active on this session.
        // Actual observation masking is applied at LLM message construction time
        // (in ReasonAtom) rather than at message query time, because masking
        // operates on LlmMessage format, not the storage Message format.
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
pub fn estimate_tokens(msg: &LlmMessage) -> usize {
    let text_len = match &msg.content {
        LlmMessageContent::Text(t) => t.len(),
        LlmMessageContent::Parts(parts) => parts
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
pub fn estimate_total_tokens(messages: &[LlmMessage]) -> usize {
    messages.iter().map(estimate_tokens).sum()
}

/// Check whether proactive compaction should trigger.
///
/// Returns `true` if the estimated tokens exceed `budget_percent` of the model's
/// context window.
pub fn should_compact_proactively(
    messages: &[LlmMessage],
    config: &CompactionConfig,
    context_window_tokens: usize,
) -> bool {
    if !config.proactive {
        return false;
    }
    let budget = (context_window_tokens as f32 * config.budget_percent) as usize;
    let estimated = estimate_total_tokens(messages);
    estimated > budget
}

/// Check whether cumulative prompt cost or raw tool evidence warrants a durable checkpoint.
///
/// The marginal prompt floor prevents a large lifetime counter from compacting
/// every short follow-up. Checkpoint suffix re-arming and retry watermarks remain
/// owned by the reason atom, so this predicate is pure and restart-safe.
pub fn should_compact_for_cost(
    estimated_input_tokens: usize,
    raw_tool_result_bytes: usize,
    config: &CompactionConfig,
    prior_usage: Option<&TokenUsage>,
) -> bool {
    if !config.proactive
        || !config.cost_control.enabled
        || estimated_input_tokens < config.cost_control.compact_min_input_tokens
    {
        return false;
    }

    let cumulative_uncached_input = prior_usage.map_or(0, |usage| usage.input_tokens);
    cumulative_uncached_input >= config.cost_control.max_uncached_input_tokens
        || raw_tool_result_bytes >= config.cost_control.compact_after_tool_result_bytes
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
    messages: &[LlmMessage],
    target_tokens: usize,
    has_system_prompt: bool,
) -> Vec<LlmMessage> {
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

    let mut protected_budget: usize = 0;
    for &idx in &protected_indices {
        protected_budget += estimate_tokens(&conversation[idx]);
    }

    // If protected messages alone exceed the remaining budget, keep as many
    // protected messages as possible (newest first) and skip non-protected.
    if protected_budget > token_budget {
        let mut protected_with_indices: Vec<(usize, LlmMessage)> = protected_indices
            .iter()
            .map(|&idx| (idx, conversation[idx].clone()))
            .collect();
        protected_with_indices.sort_by_key(|(i, _)| *i);

        let mut remaining = token_budget;
        let mut kept: Vec<(usize, LlmMessage)> = Vec::new();
        for (idx, msg) in protected_with_indices.into_iter().rev() {
            let t = estimate_tokens(&msg);
            if t <= remaining {
                kept.push((idx, msg));
                remaining -= t;
            }
        }
        kept.sort_by_key(|(i, _)| *i);
        result.extend(kept.into_iter().map(|(_, m)| m));
        return crate::tool_call_integrity::retain_complete_llm_tool_exchanges(result);
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
    let mut all_kept: Vec<(usize, LlmMessage)> = Vec::new();
    for &idx in &protected_indices {
        all_kept.push((idx, conversation[idx].clone()));
    }
    all_kept.extend(keep_from_end);
    all_kept.sort_by_key(|(i, _)| *i);

    result.extend(all_kept.into_iter().map(|(_, m)| m));
    crate::tool_call_integrity::retain_complete_llm_tool_exchanges(result)
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
    messages: &'a [LlmMessage],
    config: &HierarchicalMemoryConfig,
) -> Vec<(MemoryTier, &'a LlmMessage)> {
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
    messages: &[LlmMessage],
    config: &HierarchicalMemoryConfig,
    masking_config: &ObservationMaskingConfig,
    cold_summary: Option<&str>,
) -> Vec<LlmMessage> {
    let len = messages.len();
    let hot_start = len.saturating_sub(config.hot_messages);
    let warm_start = hot_start.saturating_sub(config.warm_messages);

    let mut result = Vec::new();

    // Cold tier: replace with summary if available, but rescue protected messages
    if warm_start > 0 {
        // Extract protected messages from cold tier before dropping
        let cold_msgs = &messages[..warm_start];
        let protected_cold: Vec<LlmMessage> = cold_msgs
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

    crate::tool_call_integrity::retain_complete_llm_tool_exchanges(result)
}

// ============================================================================
// Protected Tool Detection
// ============================================================================

use crate::driver_registry::{LlmContentPart, LlmMessage, LlmMessageContent, LlmMessageRole};

/// Tool names whose results must be protected from compaction.
///
/// Skill activation results contain durable behavioral instructions that silently
/// degrade agent behavior when masked, summarized, or trimmed. The agentskills.io
/// client implementation guide recommends exempting skill content from pruning.
///
/// See: knowledge/runtime-resources/compaction.md (Tier 3: tool-aware masking), knowledge/project/skills-registry.md
const PROTECTED_TOOL_NAMES: &[&str] = &["activate_skill"];

/// Check if a tool result message corresponds to a protected tool.
///
/// Looks up the tool_call_id in preceding assistant messages to find the tool name.
/// Returns `true` if the tool name is in `PROTECTED_TOOL_NAMES`.
fn is_protected_tool_result(messages: &[LlmMessage], tool_msg: &LlmMessage) -> bool {
    if tool_msg.role != LlmMessageRole::Tool {
        return false;
    }
    let tool_name = find_tool_call_name(messages, tool_msg);
    PROTECTED_TOOL_NAMES.contains(&tool_name.as_str())
}

/// Check if an assistant message contains a tool call to a protected tool.
///
/// Returns `true` if any tool call in the message targets a protected tool name.
fn is_protected_tool_call_message(msg: &LlmMessage) -> bool {
    if msg.role != LlmMessageRole::Assistant {
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
    pub messages: Vec<LlmMessage>,
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
    messages: &[LlmMessage],
    config: &ObservationMaskingConfig,
) -> ObservationMaskingResult {
    apply_observation_masking_with_protected(messages, config, &std::collections::HashSet::new())
}

/// Result of cost-control masking applied before provider serialization.
#[derive(Debug)]
pub struct CostControlMaskingResult {
    /// Messages after stale bulky tool results were replaced by summaries.
    pub messages: Vec<Message>,
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
    stored_messages: &[Message],
    compaction_config: &CompactionConfig,
    prior_usage: Option<&TokenUsage>,
) -> CostControlMaskingResult {
    apply_cost_control_masking(stored_messages, compaction_config, prior_usage)
}

/// Build the bounded model-view messages from owned stored messages.
///
/// This avoids cloning the message list when masking does not apply.
pub fn build_model_view_messages_owned(
    stored_messages: Vec<Message>,
    compaction_config: &CompactionConfig,
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
    messages: &[Message],
    config: &CompactionConfig,
    prior_usage: Option<&TokenUsage>,
) -> CostControlMaskingResult {
    apply_cost_control_masking_owned(messages.to_vec(), config, prior_usage)
}

fn apply_cost_control_masking_owned(
    messages: Vec<Message>,
    config: &CompactionConfig,
    prior_usage: Option<&TokenUsage>,
) -> CostControlMaskingResult {
    let cost_config = &config.cost_control;
    let tool_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| {
            message.role == MessageRole::ToolResult
                && !is_protected_message_tool_result(&messages, message)
        })
        .map(|(index, _)| index)
        .collect();
    let tool_result_bytes_before = tool_indices
        .iter()
        .map(|index| message_tool_result_len(&messages[*index]))
        .sum();

    if !cost_config.enabled
        || tool_indices.len() <= cost_config.keep_recent_tool_results
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
    let to_mask_count = tool_indices.len().saturating_sub(keep_recent);
    let related_recent_reads =
        related_recent_paginated_read_results(&messages, &tool_indices, keep_recent);
    let indices_to_mask: HashSet<usize> = tool_indices[..to_mask_count]
        .iter()
        .copied()
        .filter(|index| !related_recent_reads.contains(index))
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
        .filter(|message| message.role == MessageRole::ToolResult)
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

fn is_protected_message_tool_result(messages: &[Message], tool_msg: &Message) -> bool {
    if tool_msg.role != MessageRole::ToolResult {
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
    messages: &[Message],
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

fn paginated_read_result_key(messages: &[Message], tool_msg: &Message) -> Option<ReadResultKey> {
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
            | "deno_read_file"
            | "sprites_read_file"
            | "read_github_file"
    )
}

fn find_message_tool_call_name(messages: &[Message], tool_msg: &Message) -> String {
    let Some(call_id) = tool_msg.tool_call_id() else {
        return "unknown_tool".to_string();
    };

    for msg in messages.iter().rev() {
        if msg.role != MessageRole::Agent {
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

fn message_tool_result_len(message: &Message) -> usize {
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
pub fn total_tool_result_bytes(messages: &[Message]) -> usize {
    messages
        .iter()
        .filter(|message| message.role == MessageRole::ToolResult)
        .fold(0usize, |total, message| {
            total.saturating_add(message_tool_result_len(message))
        })
}

fn mask_tool_result_message(message: &Message, tool_name: &str) -> Message {
    let Some(result) = message.tool_result_content() else {
        return message.clone();
    };
    let summary = summarize_tool_result(tool_name, result.result.as_ref(), result.error.as_ref());
    let was_error = result.error.is_some();
    let mut masked = message.clone();
    for part in &mut masked.content {
        if let ContentPart::ToolResult(tool_result) = part {
            if was_error {
                tool_result.result = None;
                tool_result.error = Some(summary);
            } else {
                tool_result.result = Some(serde_json::json!({
                    "masked": true,
                    "summary": summary,
                }));
                tool_result.error = None;
            }
            break;
        }
    }
    masked
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
        "bash" | "daytona_exec" | "sandbox_exec" | "e2b_exec" | "docker_exec" | "deno_exec" => {
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
    messages: &[LlmMessage],
    config: &ObservationMaskingConfig,
    extra_protected_call_ids: &std::collections::HashSet<String>,
) -> ObservationMaskingResult {
    // Separate protected vs maskable tool result indices
    let tool_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            m.role == LlmMessageRole::Tool
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
            result.push(LlmMessage {
                role: LlmMessageRole::Tool,
                content: LlmMessageContent::Text(summary),
                tool_calls: msg.tool_calls.clone(),
                tool_call_id: msg.tool_call_id.clone(),
                phase: msg.phase,
                thinking: None,
                thinking_signature: None,
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
fn find_tool_call_name(messages: &[LlmMessage], tool_msg: &LlmMessage) -> String {
    let Some(ref call_id) = tool_msg.tool_call_id else {
        return "unknown_tool".to_string();
    };

    for msg in messages.iter().rev() {
        if msg.role == LlmMessageRole::Assistant
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

fn extract_text(content: &LlmMessageContent) -> String {
    match content {
        LlmMessageContent::Text(t) => t.clone(),
        LlmMessageContent::Parts(parts) => parts
            .iter()
            .filter_map(|p| {
                if let LlmContentPart::Text { text } = p {
                    Some(text.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn format_one_line_summary(tool_name: &str, content: &LlmMessageContent) -> String {
    let text = extract_text(content);
    let line_count = text.lines().count();
    let byte_len = text.len();

    if byte_len <= 100 {
        format!("[{tool_name} → {text}]")
    } else {
        format!("[{tool_name} → {line_count} lines, {byte_len} bytes]")
    }
}

fn format_head_tail_summary(content: &LlmMessageContent) -> String {
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

/// Build the summarization system prompt.
pub fn build_summarization_prompt(config: &SummarizationConfig) -> String {
    let preserve_items = if config.preserve.is_empty() {
        default_preserve()
    } else {
        config.preserve.clone()
    };

    let preserve_list = preserve_items
        .iter()
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>()
        .join("\n");

    let custom_instructions = config
        .instructions
        .as_deref()
        .map(|instr| format!("\n- {instr}"))
        .unwrap_or_default();

    format!(
        r#"<task>
Summarize the following conversation history. The summary replaces these
messages in the agent's context window — it must contain everything the
agent needs to continue working.
</task>

<preserve>
{preserve_list}{custom_instructions}
</preserve>

<format>
Produce a structured summary. Use sections. Be concise but complete.
Do not include tool output verbatim — reference files by path.
IMPORTANT: Any activate_skill tool results contain durable skill instructions.
Include them verbatim in a dedicated "Active Skills" section — do not summarize
or paraphrase skill instructions.
</format>"#
    )
}

/// Format messages into a text block for the summarization prompt.
pub fn format_messages_for_summarization(messages: &[LlmMessage]) -> String {
    let mut parts = Vec::new();
    for msg in messages {
        let role = match msg.role {
            LlmMessageRole::System => "system",
            LlmMessageRole::User => "user",
            LlmMessageRole::Assistant => "assistant",
            LlmMessageRole::Tool => "tool",
        };

        let content = extract_text(&msg.content);

        // Protected tool results (skill instructions) are never truncated —
        // the summarizer must see the full text to reproduce them verbatim.
        let is_protected = is_protected_tool_result(messages, msg);

        // Truncate very long messages to avoid blowing up the summarization prompt
        let truncated = if !is_protected && content.len() > 2000 {
            let safe_prefix = truncate_at_char_boundary(&content, 2000);
            format!(
                "{}... [truncated, {} chars total]",
                safe_prefix,
                content.len()
            )
        } else {
            content
        };

        parts.push(format!("[{role}]: {truncated}"));
    }
    parts.join("\n\n")
}

fn truncate_at_char_boundary(content: &str, max_bytes: usize) -> &str {
    if content.len() <= max_bytes {
        return content;
    }

    if content.is_char_boundary(max_bytes) {
        return &content[..max_bytes];
    }

    let mut end = max_bytes;
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }

    &content[..end]
}

/// Build a summary system message that replaces compacted messages in context.
pub fn build_summary_message(summary_text: &str) -> LlmMessage {
    LlmMessage {
        role: LlmMessageRole::System,
        content: LlmMessageContent::Text(format!(
            "[CONVERSATION_SUMMARY]\n{summary_text}\n[/CONVERSATION_SUMMARY]"
        )),
        tool_calls: None,
        tool_call_id: None,
        phase: None,
        thinking: None,
        thinking_signature: None,
    }
}

/// Compose a summary with the verbatim recent tail without splitting tool exchanges.
///
/// The summary replaces the older prefix, so a result retained at the boundary has
/// no visible call unless that call is also in `recent_messages`. Incomplete pieces
/// are removed from this prompt-facing view; stored history remains unchanged.
pub fn compose_summary_with_recent(
    system_message: Option<LlmMessage>,
    summary_text: &str,
    recent_messages: &[LlmMessage],
) -> Vec<LlmMessage> {
    let mut messages = Vec::with_capacity(recent_messages.len() + 2);
    if let Some(system_message) = system_message {
        messages.push(system_message);
    }
    messages.push(build_summary_message(summary_text));
    messages.extend_from_slice(recent_messages);
    crate::tool_call_integrity::retain_complete_llm_tool_exchanges(messages)
}

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
mod tests {
    use super::*;
    use crate::tool_types::ToolCall;
    use serde_json::json;

    fn assert_complete_tool_exchanges(messages: &[LlmMessage]) {
        let calls: std::collections::HashSet<_> = messages
            .iter()
            .flat_map(|message| message.tool_calls.iter().flatten())
            .map(|call| call.id.as_str())
            .collect();
        let results: std::collections::HashSet<_> = messages
            .iter()
            .filter_map(|message| message.tool_call_id.as_deref())
            .collect();
        assert_eq!(calls, results, "tool calls and results must remain atomic");
    }

    fn make_user_msg(text: &str) -> LlmMessage {
        LlmMessage {
            role: LlmMessageRole::User,
            content: LlmMessageContent::Text(text.to_string()),
            tool_calls: None,
            tool_call_id: None,
            phase: None,
            thinking: None,
            thinking_signature: None,
        }
    }

    fn make_assistant_msg(text: &str) -> LlmMessage {
        LlmMessage {
            role: LlmMessageRole::Assistant,
            content: LlmMessageContent::Text(text.to_string()),
            tool_calls: None,
            tool_call_id: None,
            phase: None,
            thinking: None,
            thinking_signature: None,
        }
    }

    fn make_assistant_with_tool_call(call_id: &str, tool_name: &str) -> LlmMessage {
        LlmMessage {
            role: LlmMessageRole::Assistant,
            content: LlmMessageContent::Text(String::new()),
            tool_calls: Some(vec![ToolCall {
                id: call_id.to_string(),
                name: tool_name.to_string(),
                arguments: json!({"path": "src/main.rs"}),
            }]),
            tool_call_id: None,
            phase: None,
            thinking: None,
            thinking_signature: None,
        }
    }

    fn make_assistant_with_tool_calls(calls: &[(&str, &str)]) -> LlmMessage {
        LlmMessage {
            role: LlmMessageRole::Assistant,
            content: LlmMessageContent::Text(String::new()),
            tool_calls: Some(
                calls
                    .iter()
                    .map(|(call_id, tool_name)| ToolCall {
                        id: (*call_id).to_string(),
                        name: (*tool_name).to_string(),
                        arguments: json!({}),
                    })
                    .collect(),
            ),
            tool_call_id: None,
            phase: None,
            thinking: None,
            thinking_signature: None,
        }
    }

    fn make_tool_result(call_id: &str, output: &str) -> LlmMessage {
        LlmMessage {
            role: LlmMessageRole::Tool,
            content: LlmMessageContent::Text(output.to_string()),
            tool_calls: None,
            tool_call_id: Some(call_id.to_string()),
            phase: None,
            thinking: None,
            thinking_signature: None,
        }
    }

    // ====================================================================
    // CompactionConfig tests
    // ====================================================================

    #[test]
    fn test_capability_metadata() {
        let cap = CompactionCapability;
        assert_eq!(cap.id(), COMPACTION_CAPABILITY_ID);
        assert_eq!(cap.name(), "Compaction");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.category(), Some("Optimization"));
        assert!(cap.tools().is_empty());
        assert!(cap.message_filter_provider().is_some());
    }

    #[test]
    fn test_config_schema_and_validate_config() {
        let cap = CompactionCapability;

        let schema = cap.config_schema().expect("config schema");
        assert_eq!(schema["type"], "object");
        // Only the simple knobs are exposed; advanced nested objects stay out.
        assert!(schema["properties"]["strategy"].is_object());
        assert!(schema["properties"]["proactive"].is_object());
        assert!(schema["properties"]["budget_percent"].is_object());
        assert!(schema["properties"].get("observation_masking").is_none());
        assert!(schema["properties"].get("cost_control").is_none());

        // Null and valid configs are accepted.
        assert!(cap.validate_config(&serde_json::Value::Null).is_ok());
        assert!(
            cap.validate_config(&json!({
                "strategy": "native",
                "proactive": false,
                "budget_percent": 0.9
            }))
            .is_ok()
        );
        // Advanced nested fields are tolerated even though not in the schema.
        assert!(
            cap.validate_config(&json!({
                "strategy": "observation_masking",
                "observation_masking": { "keep_recent_tool_outputs": 4 },
                "cost_control": { "enabled": false }
            }))
            .is_ok()
        );

        // Invalid values are rejected.
        assert!(cap.validate_config(&json!({"strategy": "bogus"})).is_err());
        let err = cap
            .validate_config(&json!({"budget_percent": 5.0}))
            .unwrap_err();
        assert!(err.contains("budget_percent"));
    }

    #[test]
    fn test_localizations_resolve_uk() {
        let cap = CompactionCapability;
        assert_eq!(cap.localized_name(Some("uk-UA")), "Ущільнення контексту");
        assert!(cap.describe_schema(None).is_some());
    }

    #[test]
    fn test_default_config() {
        let config = CompactionConfig::default();
        assert_eq!(config.strategy, CompactionStrategy::Auto);
        assert!(config.proactive);
        assert!((config.budget_percent - 0.85).abs() < f32::EPSILON);
        assert_eq!(config.observation_masking.keep_recent_tool_outputs, 2);
        assert_eq!(
            config.observation_masking.summary_format,
            MaskingSummaryFormat::OneLine
        );
        assert!(config.summarization.model.is_none());
        assert_eq!(config.summarization.preserve.len(), 5);
        assert!(config.summarization.instructions.is_none());
        assert!(config.cost_control.enabled);
        assert_eq!(config.cost_control.keep_recent_tool_results, 2);
    }

    #[test]
    fn test_config_from_empty_json() {
        let config = CompactionConfig::from_json(&json!({}));
        assert_eq!(config.strategy, CompactionStrategy::Auto);
        assert!(config.proactive);
    }

    #[test]
    fn test_config_native_only() {
        let config = CompactionConfig::from_json(&json!({"strategy": "native"}));
        assert_eq!(config.strategy, CompactionStrategy::Native);
        assert!(config.proactive);
    }

    #[test]
    fn test_config_observation_masking_with_custom_settings() {
        let config = CompactionConfig::from_json(&json!({
            "strategy": "observation_masking",
            "proactive": false,
            "observation_masking": {
                "keep_recent_tool_outputs": 10,
                "summary_format": "head_tail"
            }
        }));
        assert_eq!(config.strategy, CompactionStrategy::ObservationMasking);
        assert!(!config.proactive);
        assert_eq!(config.observation_masking.keep_recent_tool_outputs, 10);
        assert_eq!(
            config.observation_masking.summary_format,
            MaskingSummaryFormat::HeadTail
        );
    }

    #[test]
    fn test_config_cost_control_with_custom_settings() {
        let config = CompactionConfig::from_json(&json!({
            "cost_control": {
                "enabled": true,
                "keep_recent_tool_results": 1,
                "mask_after_tool_results": 2,
                "max_live_tool_result_bytes": 4096,
                "max_uncached_input_tokens": 50000,
                "min_cache_read_ratio": 0.5
            }
        }));

        assert!(config.cost_control.enabled);
        assert_eq!(config.cost_control.keep_recent_tool_results, 1);
        assert_eq!(config.cost_control.mask_after_tool_results, 2);
        assert_eq!(config.cost_control.max_live_tool_result_bytes, 4096);
        assert_eq!(config.cost_control.max_uncached_input_tokens, 50000);
        assert!((config.cost_control.min_cache_read_ratio - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_config_summarization_with_custom_model() {
        let config = CompactionConfig::from_json(&json!({
            "strategy": "summarization",
            "summarization": {
                "model": "claude-haiku-4-5-20251001",
                "instructions": "Focus on API decisions",
                "preserve": ["decisions", "errors"]
            }
        }));
        assert_eq!(config.strategy, CompactionStrategy::Summarization);
        assert_eq!(
            config.summarization.model.as_deref(),
            Some("claude-haiku-4-5-20251001")
        );
        assert_eq!(
            config.summarization.instructions.as_deref(),
            Some("Focus on API decisions")
        );
        assert_eq!(config.summarization.preserve.len(), 2);
    }

    fn make_message_tool_turn(
        call_id: &str,
        tool_name: &str,
        result: serde_json::Value,
    ) -> Vec<Message> {
        vec![
            Message::assistant_with_tools(
                "",
                vec![ToolCall {
                    id: call_id.to_string(),
                    name: tool_name.to_string(),
                    arguments: json!({"path": "/workspace/src/lib.rs"}),
                }],
            ),
            Message::tool_result(call_id, Some(result), None),
        ]
    }

    #[test]
    fn test_cost_control_masks_old_read_file_results() {
        let mut messages = vec![Message::user("inspect files")];
        for index in 0..5 {
            messages.extend(make_message_tool_turn(
                &format!("call_{index}"),
                "read_file",
                json!({
                    "path": "/workspace/src/lib.rs",
                    "content": format!("{}{}", "line\n".repeat(400), index),
                    "total_lines": 900,
                    "lines_shown": {"start": 1, "end": 400},
                    "truncated": true,
                    "content_hash": format!("sha256:{index}"),
                    "truncation": {"truncated": true, "next_offset": 400, "reason": "line_cap"}
                }),
            ));
        }

        let config = CompactionConfig::from_json(&json!({
            "cost_control": {
                "keep_recent_tool_results": 2,
                "mask_after_tool_results": 4
            }
        }));
        let result = apply_cost_control_masking(&messages, &config, None);

        assert_eq!(result.masked_count, 3);
        assert!(result.tool_result_bytes_after < result.tool_result_bytes_before);

        let first_tool = result.messages[2].tool_result_content().unwrap();
        let masked = first_tool.result.as_ref().unwrap();
        assert_eq!(masked["masked"], true);
        let summary = masked["summary"].as_str().unwrap();
        assert!(summary.contains("read_file"));
        assert!(summary.contains("/workspace/src/lib.rs"));
        assert!(summary.contains("lines 1-400"));
        assert!(summary.contains("next_offset=400"));
        assert!(!summary.contains("line\nline"));

        let last_tool = result
            .messages
            .last()
            .unwrap()
            .tool_result_content()
            .unwrap();
        assert!(last_tool.result.as_ref().unwrap().get("content").is_some());
    }

    #[test]
    fn test_cost_control_keeps_recent_paginated_read_group() {
        let mut messages = vec![Message::user("inspect saved output")];
        messages.extend(make_message_tool_turn(
            "call_bash",
            "bash",
            json!({
                "stdout": "old command output",
                "stderr": "",
                "exit_code": 0,
                "success": true
            }),
        ));
        messages.extend(make_message_tool_turn(
            "call_read_first",
            "read_file",
            json!({
                "path": "/workspace/outputs/call_123.stdout",
                "content": "first page\n".repeat(200),
                "total_lines": 400,
                "lines_shown": {"start": 1, "end": 200},
                "truncated": true,
                "content_hash": "sha256:same-output",
                "truncation": {"truncated": true, "next_offset": 200, "reason": "line_cap"}
            }),
        ));
        messages.extend(make_message_tool_turn(
            "call_read_second",
            "read_file",
            json!({
                "path": "/workspace/outputs/call_123.stdout",
                "content": "second page\n".repeat(200),
                "total_lines": 400,
                "lines_shown": {"start": 201, "end": 400},
                "truncated": false,
                "content_hash": "sha256:same-output"
            }),
        ));

        let config = CompactionConfig::from_json(&json!({
            "cost_control": {
                "keep_recent_tool_results": 1,
                "mask_after_tool_results": 2
            }
        }));
        let result = build_model_view_messages(&messages, &config, None);

        assert_eq!(result.masked_count, 1);
        let bash_result = result.messages[2].tool_result_content().unwrap();
        assert_eq!(bash_result.result.as_ref().unwrap()["masked"], true);
        let first_page = result.messages[4].tool_result_content().unwrap();
        assert!(first_page.result.as_ref().unwrap().get("content").is_some());
        let second_page = result.messages[6].tool_result_content().unwrap();
        assert!(
            second_page
                .result
                .as_ref()
                .unwrap()
                .get("content")
                .is_some()
        );
    }

    #[test]
    fn test_model_view_masks_with_compaction_config() {
        let mut messages = vec![Message::user("inspect files repeatedly")];
        for index in 0..9 {
            messages.extend(make_message_tool_turn(
                &format!("call_{index}"),
                "read_file",
                json!({
                    "path": "/workspace/session_019e4c9dd1b17021af70ad3227361b16.jsonl",
                    "content": format!("{}{}", "large transcript line\n".repeat(1000), index),
                    "total_lines": 1000,
                    "lines_shown": {"start": 1, "end": 1000},
                    "truncated": false,
                    "content_hash": format!("sha256:{index}")
                }),
            ));
        }

        let config = CompactionConfig::default();
        let result = build_model_view_messages(&messages, &config, None);

        assert_eq!(result.masked_count, 7);
        assert!(result.tool_result_bytes_after < result.tool_result_bytes_before / 4);
        let first_tool = result.messages[2].tool_result_content().unwrap();
        let masked = first_tool.result.as_ref().unwrap();
        assert_eq!(masked["masked"], true);
        assert!(masked["summary"].as_str().unwrap().contains("read_file"));
        let last_tool = result
            .messages
            .last()
            .unwrap()
            .tool_result_content()
            .unwrap();
        assert!(last_tool.result.as_ref().unwrap().get("content").is_some());
    }

    #[test]
    fn test_compaction_capability_contributes_model_view_provider() {
        let mut messages = vec![Message::user("inspect files repeatedly")];
        for index in 0..9 {
            messages.extend(make_message_tool_turn(
                &format!("call_{index}"),
                "read_file",
                json!({
                    "path": "/workspace/src/lib.rs",
                    "content": format!("{}{}", "large file line\n".repeat(1000), index),
                    "total_lines": 1000,
                    "lines_shown": {"start": 1, "end": 1000},
                    "truncated": false
                }),
            ));
        }

        let capability = CompactionCapability;
        let provider = capability.model_view_provider().unwrap();
        let context = ModelViewContext {
            session_id: crate::typed_id::SessionId::new(),
            prior_usage: None,
        };
        let result = provider.apply_model_view(messages, &json!({}), &context);

        let first_tool = result[2].tool_result_content().unwrap();
        assert_eq!(first_tool.result.as_ref().unwrap()["masked"], true);
        let last_tool = result.last().unwrap().tool_result_content().unwrap();
        assert!(last_tool.result.as_ref().unwrap().get("content").is_some());
    }

    #[test]
    fn test_model_view_respects_disabled_cost_control_config() {
        let mut messages = vec![Message::user("inspect files repeatedly")];
        for index in 0..5 {
            messages.extend(make_message_tool_turn(
                &format!("call_{index}"),
                "read_file",
                json!({
                    "path": "/workspace/src/lib.rs",
                    "content": "line\n".repeat(400),
                    "total_lines": 400,
                    "lines_shown": {"start": 1, "end": 400},
                    "truncated": false
                }),
            ));
        }

        let config = CompactionConfig::from_json(&json!({
            "cost_control": {
                "enabled": false,
                "keep_recent_tool_results": 1,
                "mask_after_tool_results": 2
            }
        }));
        let result = build_model_view_messages(&messages, &config, None);

        assert_eq!(result.masked_count, 0);
        assert_eq!(
            result.tool_result_bytes_after,
            result.tool_result_bytes_before
        );
    }

    #[test]
    fn test_cost_control_uses_prior_usage_signal() {
        let mut messages = vec![Message::user("run commands")];
        for index in 0..3 {
            messages.extend(make_message_tool_turn(
                &format!("call_{index}"),
                "bash",
                json!({
                    "stdout": "small output",
                    "stderr": "",
                    "exit_code": 0,
                    "success": true
                }),
            ));
        }

        let config = CompactionConfig::from_json(&json!({
            "cost_control": {
                "keep_recent_tool_results": 1,
                "mask_after_tool_results": 99,
                "max_live_tool_result_bytes": 999999,
                "max_uncached_input_tokens": 1000
            }
        }));
        let usage = TokenUsage::with_cache(10_000, 100, Some(0), None);
        let result = apply_cost_control_masking(&messages, &config, Some(&usage));

        assert_eq!(result.masked_count, 2);
        let first_tool = result.messages[2].tool_result_content().unwrap();
        let summary = first_tool.result.as_ref().unwrap()["summary"]
            .as_str()
            .unwrap();
        assert!(summary.contains("bash exit=0"));
    }

    #[test]
    fn test_cost_control_masking_uses_disjoint_cache_buckets() {
        // Disjoint convention: `input_tokens` is the non-cached prompt and the
        // cache-read ratio is measured against the full prompt (all buckets).
        let config = CostControlConfig {
            mask_after_tool_results: usize::MAX,
            max_live_tool_result_bytes: usize::MAX,
            max_uncached_input_tokens: 50_000,
            min_cache_read_ratio: 0.35,
            ..CostControlConfig::default()
        };

        // 20K non-cached input + 180K cache reads: a well-cached run (90% hit,
        // non-cached under threshold) — usage signals must not trigger masking.
        let well_cached = TokenUsage::with_cache(20_000, 100, Some(180_000), None);
        assert!(!should_apply_cost_control_masking(
            0,
            0,
            &config,
            Some(&well_cached)
        ));

        // Same total prompt but cache-poor (10% hit): low ratio triggers masking.
        let cache_poor = TokenUsage::with_cache(20_000, 100, Some(2_000), None);
        assert!(should_apply_cost_control_masking(
            0,
            0,
            &config,
            Some(&cache_poor)
        ));

        // High non-cached input alone triggers masking regardless of cache ratio.
        let heavy_uncached = TokenUsage::with_cache(60_000, 100, Some(200_000), None);
        assert!(should_apply_cost_control_masking(
            0,
            0,
            &config,
            Some(&heavy_uncached)
        ));
    }

    #[test]
    fn test_model_view_uses_provider_cache_signal_from_compaction_config() {
        let mut messages = vec![Message::user("run commands")];
        for index in 0..3 {
            messages.extend(make_message_tool_turn(
                &format!("call_{index}"),
                "bash",
                json!({
                    "stdout": "small output",
                    "stderr": "",
                    "exit_code": 0,
                    "success": true
                }),
            ));
        }
        let usage = TokenUsage::with_cache(150_000, 100, Some(0), None);

        let config = CompactionConfig::default();
        let result = build_model_view_messages(&messages, &config, Some(&usage));

        assert_eq!(result.masked_count, 1);
        let first_tool = result.messages[2].tool_result_content().unwrap();
        assert_eq!(first_tool.result.as_ref().unwrap()["masked"], true);
    }

    #[test]
    fn test_config_falls_back_to_defaults_for_invalid_json() {
        let config = CompactionConfig::from_json(&json!({
            "strategy": "nonexistent_strategy",
            "budget_percent": "not-a-number"
        }));
        assert_eq!(config.strategy, CompactionStrategy::Auto);
        assert!(config.proactive);
    }

    #[test]
    fn test_config_partial_override() {
        let config = CompactionConfig::from_json(&json!({
            "budget_percent": 0.7,
            "observation_masking": {
                "keep_recent_tool_outputs": 3
            }
        }));
        assert_eq!(config.strategy, CompactionStrategy::Auto);
        assert!(config.proactive);
        assert!((config.budget_percent - 0.7).abs() < f32::EPSILON);
        assert_eq!(config.observation_masking.keep_recent_tool_outputs, 3);
        assert_eq!(
            config.observation_masking.summary_format,
            MaskingSummaryFormat::OneLine
        );
    }

    #[test]
    fn test_strategy_serialization_roundtrip() {
        for strategy in [
            CompactionStrategy::Auto,
            CompactionStrategy::Native,
            CompactionStrategy::ObservationMasking,
            CompactionStrategy::Summarization,
        ] {
            let json = serde_json::to_value(strategy).unwrap();
            let deserialized: CompactionStrategy = serde_json::from_value(json).unwrap();
            assert_eq!(strategy, deserialized);
        }
    }

    #[test]
    fn test_strategy_display() {
        assert_eq!(CompactionStrategy::Auto.to_string(), "auto");
        assert_eq!(CompactionStrategy::Native.to_string(), "native");
        assert_eq!(
            CompactionStrategy::ObservationMasking.to_string(),
            "observation_masking"
        );
        assert_eq!(
            CompactionStrategy::Summarization.to_string(),
            "summarization"
        );
    }

    #[test]
    fn test_masking_format_serialization_roundtrip() {
        for format in [
            MaskingSummaryFormat::OneLine,
            MaskingSummaryFormat::HeadTail,
        ] {
            let json = serde_json::to_value(format).unwrap();
            let deserialized: MaskingSummaryFormat = serde_json::from_value(json).unwrap();
            assert_eq!(format, deserialized);
        }
    }

    #[test]
    fn test_budget_percent_boundary_values() {
        let config = CompactionConfig::from_json(&json!({"budget_percent": 0.1}));
        assert!((config.budget_percent - 0.1).abs() < f32::EPSILON);

        let config = CompactionConfig::from_json(&json!({"budget_percent": 0.99}));
        assert!((config.budget_percent - 0.99).abs() < f32::EPSILON);
    }

    #[test]
    fn test_keep_recent_tool_outputs_zero() {
        let config = CompactionConfig::from_json(&json!({
            "observation_masking": {"keep_recent_tool_outputs": 0}
        }));
        assert_eq!(config.observation_masking.keep_recent_tool_outputs, 0);
    }

    // ====================================================================
    // Observation masking tests
    // ====================================================================

    #[test]
    fn test_masking_no_tool_messages() {
        let messages = vec![make_user_msg("hello"), make_assistant_msg("hi")];
        let config = ObservationMaskingConfig::default();
        let result = apply_observation_masking(&messages, &config);
        assert_eq!(result.masked_count, 0);
        assert_eq!(result.messages.len(), 2);
    }

    #[test]
    fn test_masking_fewer_than_keep_recent() {
        let messages = vec![
            make_user_msg("read file"),
            make_assistant_with_tool_call("call_1", "read_file"),
            make_tool_result("call_1", "file contents"),
            make_assistant_msg("done"),
        ];
        let config = ObservationMaskingConfig {
            keep_recent_tool_outputs: 5,
            summary_format: MaskingSummaryFormat::OneLine,
        };
        let result = apply_observation_masking(&messages, &config);
        assert_eq!(result.masked_count, 0);
    }

    #[test]
    fn test_masking_masks_old_outputs() {
        let messages = vec![
            make_user_msg("start"),
            make_assistant_with_tool_call("call_1", "read_file"),
            make_tool_result(
                "call_1",
                "old file contents that are very long and should be masked by the observation masking strategy because it exceeds 100 chars",
            ),
            make_assistant_msg("got it"),
            make_user_msg("next"),
            make_assistant_with_tool_call("call_2", "search"),
            make_tool_result("call_2", "search results"),
            make_assistant_msg("found it"),
            make_user_msg("more"),
            make_assistant_with_tool_call("call_3", "bash"),
            make_tool_result("call_3", "command output"),
        ];

        let config = ObservationMaskingConfig {
            keep_recent_tool_outputs: 2,
            summary_format: MaskingSummaryFormat::OneLine,
        };
        let result = apply_observation_masking(&messages, &config);

        assert_eq!(result.masked_count, 1);

        // First tool result should be masked
        let masked = &result.messages[2];
        assert_eq!(masked.role, LlmMessageRole::Tool);
        let text = extract_text(&masked.content);
        assert!(
            text.starts_with('['),
            "Expected masked summary, got: {text}"
        );
        assert!(text.contains("read_file"), "Expected tool name: {text}");

        // Last 2 tool results should be verbatim
        assert_eq!(extract_text(&result.messages[6].content), "search results");
        assert_eq!(extract_text(&result.messages[10].content), "command output");
    }

    #[test]
    fn test_masking_preserves_tool_call_id() {
        let messages = vec![
            make_assistant_with_tool_call("call_1", "read_file"),
            make_tool_result("call_1", "content"),
            make_assistant_with_tool_call("call_2", "bash"),
            make_tool_result("call_2", "output"),
        ];

        let config = ObservationMaskingConfig {
            keep_recent_tool_outputs: 1,
            summary_format: MaskingSummaryFormat::OneLine,
        };
        let result = apply_observation_masking(&messages, &config);
        assert_eq!(result.messages[1].tool_call_id, Some("call_1".to_string()));
    }

    #[test]
    fn test_masking_head_tail_format() {
        let long_output = (0..20)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");

        let messages = vec![
            make_assistant_with_tool_call("call_1", "bash"),
            make_tool_result("call_1", &long_output),
            make_assistant_with_tool_call("call_2", "bash"),
            make_tool_result("call_2", "recent output"),
        ];

        let config = ObservationMaskingConfig {
            keep_recent_tool_outputs: 1,
            summary_format: MaskingSummaryFormat::HeadTail,
        };
        let result = apply_observation_masking(&messages, &config);

        let text = extract_text(&result.messages[1].content);
        assert!(text.contains("line 0"), "Should contain first lines");
        assert!(text.contains("line 19"), "Should contain last lines");
        assert!(text.contains("lines omitted"), "Should indicate omissions");
    }

    #[test]
    fn test_masking_short_output_inline() {
        let messages = vec![
            make_assistant_with_tool_call("call_1", "get_time"),
            make_tool_result("call_1", "2024-01-01"),
            make_assistant_with_tool_call("call_2", "bash"),
            make_tool_result("call_2", "ok"),
        ];

        let config = ObservationMaskingConfig {
            keep_recent_tool_outputs: 1,
            summary_format: MaskingSummaryFormat::OneLine,
        };
        let result = apply_observation_masking(&messages, &config);
        let text = extract_text(&result.messages[1].content);
        assert!(text.contains("2024-01-01"), "Short output included: {text}");
    }

    #[test]
    fn test_masking_all_when_keep_zero() {
        let messages = vec![
            make_assistant_with_tool_call("call_1", "a"),
            make_tool_result("call_1", "output1"),
            make_assistant_with_tool_call("call_2", "b"),
            make_tool_result("call_2", "output2"),
        ];

        let config = ObservationMaskingConfig {
            keep_recent_tool_outputs: 0,
            summary_format: MaskingSummaryFormat::OneLine,
        };
        let result = apply_observation_masking(&messages, &config);
        assert_eq!(result.masked_count, 2);
    }

    #[test]
    fn test_masking_empty_messages() {
        let result = apply_observation_masking(&[], &ObservationMaskingConfig::default());
        assert_eq!(result.masked_count, 0);
        assert!(result.messages.is_empty());
    }

    #[test]
    fn test_masking_preserves_message_count() {
        let messages = vec![
            make_user_msg("start"),
            make_assistant_with_tool_call("c1", "read_file"),
            make_tool_result("c1", "content 1"),
            make_assistant_msg("ok"),
            make_user_msg("next"),
            make_assistant_with_tool_call("c2", "bash"),
            make_tool_result("c2", "content 2"),
            make_assistant_msg("done"),
        ];

        let config = ObservationMaskingConfig {
            keep_recent_tool_outputs: 1,
            summary_format: MaskingSummaryFormat::OneLine,
        };
        let result = apply_observation_masking(&messages, &config);
        assert_eq!(result.messages.len(), messages.len());
    }

    #[test]
    fn test_masking_unknown_tool_call_id() {
        let messages = vec![
            make_tool_result("orphan", "some output"),
            make_assistant_with_tool_call("call_2", "bash"),
            make_tool_result("call_2", "recent"),
        ];

        let config = ObservationMaskingConfig {
            keep_recent_tool_outputs: 1,
            summary_format: MaskingSummaryFormat::OneLine,
        };
        let result = apply_observation_masking(&messages, &config);
        assert_eq!(result.masked_count, 1);
        let text = extract_text(&result.messages[0].content);
        assert!(text.contains("unknown_tool"), "Fallback name: {text}");
    }

    #[test]
    fn test_masking_many_tool_calls_keeps_exactly_n() {
        let mut messages = Vec::new();
        for i in 0..10 {
            let id = format!("call_{i}");
            messages.push(make_assistant_with_tool_call(&id, &format!("tool_{i}")));
            messages.push(make_tool_result(&id, &format!("output {i}")));
        }

        let config = ObservationMaskingConfig {
            keep_recent_tool_outputs: 3,
            summary_format: MaskingSummaryFormat::OneLine,
        };
        let result = apply_observation_masking(&messages, &config);
        assert_eq!(result.masked_count, 7);

        // Last 3 tool results at indices 15, 17, 19 should be verbatim
        assert_eq!(extract_text(&result.messages[15].content), "output 7");
        assert_eq!(extract_text(&result.messages[17].content), "output 8");
        assert_eq!(extract_text(&result.messages[19].content), "output 9");
    }

    // ====================================================================
    // Summarization tests
    // ====================================================================

    #[test]
    fn test_summarization_prompt_default() {
        let config = SummarizationConfig::default();
        let prompt = build_summarization_prompt(&config);
        assert!(prompt.contains("<task>"));
        assert!(prompt.contains("decisions"));
        assert!(prompt.contains("files_modified"));
        assert!(prompt.contains("errors"));
        assert!(prompt.contains("current_plan"));
    }

    #[test]
    fn test_summarization_prompt_custom_instructions() {
        let config = SummarizationConfig {
            instructions: Some("Focus on API changes".to_string()),
            ..Default::default()
        };
        let prompt = build_summarization_prompt(&config);
        assert!(prompt.contains("Focus on API changes"));
    }

    #[test]
    fn test_summarization_prompt_custom_preserve() {
        let config = SummarizationConfig {
            preserve: vec!["auth_tokens".to_string(), "database_schema".to_string()],
            ..Default::default()
        };
        let prompt = build_summarization_prompt(&config);
        assert!(prompt.contains("auth_tokens"));
        assert!(prompt.contains("database_schema"));
        assert!(!prompt.contains("decisions"));
    }

    #[test]
    fn test_summarization_prompt_empty_preserve_uses_defaults() {
        let config = SummarizationConfig {
            preserve: vec![],
            ..Default::default()
        };
        let prompt = build_summarization_prompt(&config);
        assert!(prompt.contains("decisions"));
    }

    #[test]
    fn test_format_messages_for_summarization() {
        let messages = vec![
            make_user_msg("What is 2+2?"),
            make_assistant_msg("The answer is 4."),
        ];
        let formatted = format_messages_for_summarization(&messages);
        assert!(formatted.contains("[user]: What is 2+2?"));
        assert!(formatted.contains("[assistant]: The answer is 4."));
    }

    #[test]
    fn test_format_messages_truncates_long_content() {
        let long_content = "x".repeat(5000);
        let messages = vec![make_user_msg(&long_content)];
        let formatted = format_messages_for_summarization(&messages);
        assert!(formatted.contains("truncated"));
        assert!(formatted.len() < long_content.len());
    }

    #[test]
    fn test_format_messages_truncates_utf8_without_panic() {
        let multibyte = "é".repeat(1001); // 2002 bytes, 1001 chars
        let messages = vec![make_user_msg(&multibyte)];
        let formatted = format_messages_for_summarization(&messages);
        assert!(formatted.contains("truncated"));
        assert!(formatted.contains("[truncated, 2002 chars total]"));
    }

    #[test]
    fn test_build_summary_message() {
        let msg = build_summary_message("The user asked about APIs.");
        assert_eq!(msg.role, LlmMessageRole::System);
        let text = extract_text(&msg.content);
        assert!(text.contains("[CONVERSATION_SUMMARY]"));
        assert!(text.contains("The user asked about APIs."));
        assert!(text.contains("[/CONVERSATION_SUMMARY]"));
    }

    // ====================================================================
    // Head-tail format edge cases
    // ====================================================================

    #[test]
    fn test_head_tail_short_content_unchanged() {
        let content = LlmMessageContent::Text("line1\nline2\nline3".to_string());
        assert_eq!(format_head_tail_summary(&content), "line1\nline2\nline3");
    }

    #[test]
    fn test_head_tail_exactly_six_lines() {
        let content = LlmMessageContent::Text("1\n2\n3\n4\n5\n6".to_string());
        assert_eq!(format_head_tail_summary(&content), "1\n2\n3\n4\n5\n6");
    }

    #[test]
    fn test_head_tail_seven_lines() {
        let content = LlmMessageContent::Text("1\n2\n3\n4\n5\n6\n7".to_string());
        let result = format_head_tail_summary(&content);
        assert!(result.contains("1\n2\n3"));
        assert!(result.contains("5\n6\n7"));
        assert!(result.contains("1 lines omitted"));
    }

    // ====================================================================
    // One-line format edge cases
    // ====================================================================

    #[test]
    fn test_one_line_empty_output() {
        let result = format_one_line_summary("bash", &LlmMessageContent::Text(String::new()));
        assert_eq!(result, "[bash → ]");
    }

    #[test]
    fn test_one_line_exactly_100_chars() {
        let text = "x".repeat(100);
        let result = format_one_line_summary("bash", &LlmMessageContent::Text(text.clone()));
        assert!(result.contains(&text));
    }

    #[test]
    fn test_one_line_101_chars_summarized() {
        let text = "x".repeat(101);
        let result = format_one_line_summary("bash", &LlmMessageContent::Text(text));
        assert!(result.contains("lines"));
        assert!(result.contains("bytes"));
    }

    #[test]
    fn test_one_line_multipart_content() {
        let content = LlmMessageContent::Parts(vec![
            LlmContentPart::Text {
                text: "part1".to_string(),
            },
            LlmContentPart::Text {
                text: "part2".to_string(),
            },
        ]);
        let result = format_one_line_summary("tool", &content);
        assert!(result.contains("part1"));
        assert!(result.contains("part2"));
    }

    // ====================================================================
    // CompactionStep tests
    // ====================================================================

    #[test]
    fn test_compaction_step_serialization() {
        let step = CompactionStep {
            strategy: "observation_masking".to_string(),
            messages_after: 42,
            duration_ms: 12,
        };
        let json = serde_json::to_value(&step).unwrap();
        assert_eq!(json["strategy"], "observation_masking");
        assert_eq!(json["messages_after"], 42);
        assert_eq!(json["duration_ms"], 12);
    }

    // ====================================================================
    // Token estimation tests
    // ====================================================================

    #[test]
    fn test_estimate_tokens_text() {
        let msg = make_user_msg("hello world"); // 11 chars → ~2 tokens
        let tokens = estimate_tokens(&msg);
        assert_eq!(tokens, 11 / 4);
    }

    #[test]
    fn test_estimate_tokens_empty() {
        let msg = make_user_msg("");
        assert_eq!(estimate_tokens(&msg), 0);
    }

    #[test]
    fn test_estimate_total_tokens() {
        let messages = vec![
            make_user_msg("a".repeat(400).as_str()),      // 100 tokens
            make_assistant_msg("b".repeat(200).as_str()), // 50 tokens
        ];
        assert_eq!(estimate_total_tokens(&messages), 150);
    }

    #[test]
    fn test_estimate_tokens_with_tool_calls() {
        let msg = make_assistant_with_tool_call("call_1", "read_file");
        let tokens = estimate_tokens(&msg);
        assert!(tokens > 0, "Tool call should contribute tokens");
    }

    // ====================================================================
    // Proactive compaction check tests
    // ====================================================================

    #[test]
    fn test_should_compact_proactively_under_budget() {
        let messages = vec![make_user_msg("short")];
        let config = CompactionConfig::default(); // 85% budget
        assert!(!should_compact_proactively(&messages, &config, 128_000));
    }

    #[test]
    fn test_should_compact_proactively_over_budget() {
        // Create messages that exceed 85% of 1000 tokens = 850 tokens
        let big_text = "x".repeat(4000); // ~1000 tokens
        let messages = vec![make_user_msg(&big_text)];
        let config = CompactionConfig::default();
        assert!(should_compact_proactively(&messages, &config, 1000));
    }

    #[test]
    fn test_should_compact_proactively_disabled() {
        let big_text = "x".repeat(4000);
        let messages = vec![make_user_msg(&big_text)];
        let config = CompactionConfig {
            proactive: false,
            ..Default::default()
        };
        assert!(!should_compact_proactively(&messages, &config, 1000));
    }

    #[test]
    fn test_should_compact_for_cumulative_uncached_cost() {
        let config = CompactionConfig::default();
        let usage = TokenUsage::new(config.cost_control.max_uncached_input_tokens, 0);

        assert!(should_compact_for_cost(
            config.cost_control.compact_min_input_tokens,
            0,
            &config,
            Some(&usage),
        ));
    }

    #[test]
    fn test_should_compact_for_raw_tool_result_bytes() {
        let config = CompactionConfig::default();

        assert!(should_compact_for_cost(
            config.cost_control.compact_min_input_tokens,
            config.cost_control.compact_after_tool_result_bytes,
            &config,
            None,
        ));
    }

    #[test]
    fn test_should_not_cost_compact_below_marginal_prompt_floor() {
        let config = CompactionConfig::default();
        let usage = TokenUsage::new(u32::MAX, 0);

        assert!(!should_compact_for_cost(
            config.cost_control.compact_min_input_tokens - 1,
            usize::MAX,
            &config,
            Some(&usage),
        ));
    }

    // ====================================================================
    // Aggressive trim tests
    // ====================================================================

    #[test]
    fn test_aggressive_trim_keeps_newest() {
        // Use big messages so budget matters
        let messages = vec![
            make_user_msg(&"s".repeat(400)),      // system: 100 tokens
            make_user_msg(&"a".repeat(400)),      // old: 100 tokens
            make_assistant_msg(&"b".repeat(400)), // old: 100 tokens
            make_user_msg(&"c".repeat(400)),      // recent: 100 tokens
            make_assistant_msg(&"d".repeat(400)), // recent: 100 tokens
        ];
        // Target: enough for system + 2 recent messages only (300 tokens)
        let target_tokens = 300;
        let result = aggressive_trim(&messages, target_tokens, true);
        assert!(
            result.len() < messages.len(),
            "Expected trim, got {} messages",
            result.len()
        );
        // Should keep system prompt (first)
        assert_eq!(result[0].role, LlmMessageRole::User);
    }

    #[test]
    fn test_aggressive_trim_empty() {
        let result = aggressive_trim(&[], 100, false);
        assert!(result.is_empty());
    }

    #[test]
    fn test_aggressive_trim_anchors_first_conversation_message() {
        // messages[0] = system prompt; messages[1] = the original task (old, big);
        // the rest are newer. Under a tight budget the task must still survive so
        // the model does not lose track of what it is doing.
        let messages = vec![
            make_user_msg("sys"),                 // system prompt (small)
            make_user_msg(&"TASK ".repeat(80)),   // the task: old + big
            make_assistant_msg(&"x".repeat(400)), // filler old
            make_user_msg(&"c".repeat(400)),      // recent
            make_assistant_msg(&"d".repeat(400)), // recent
        ];
        let result = aggressive_trim(&messages, 250, true);

        assert!(
            result.len() < messages.len(),
            "expected a trim, got {} messages",
            result.len()
        );
        let kept_task = result.iter().any(|m| match &m.content {
            LlmMessageContent::Text(t) => t.contains("TASK"),
            _ => false,
        });
        assert!(kept_task, "the original task must be anchored, not dropped");
    }

    #[test]
    fn test_aggressive_trim_everything_fits() {
        let messages = vec![make_user_msg("hi"), make_assistant_msg("hello")];
        let result = aggressive_trim(&messages, 100_000, false);
        assert_eq!(result.len(), 2);
    }

    // ====================================================================
    // Session compaction metrics tests
    // ====================================================================

    #[test]
    fn test_session_metrics_record() {
        let mut metrics = SessionCompactionMetrics::default();
        metrics.record("observation_masking+native", 100, 50, 200);

        assert_eq!(metrics.compaction_count, 1);
        assert_eq!(metrics.total_messages_saved, 50);
        assert_eq!(metrics.total_duration_ms, 200);
        assert_eq!(metrics.strategy_counts["observation_masking"], 1);
        assert_eq!(metrics.strategy_counts["native"], 1);
    }

    #[test]
    fn test_session_metrics_accumulate() {
        let mut metrics = SessionCompactionMetrics::default();
        metrics.record("observation_masking", 100, 80, 10);
        metrics.record("summarization", 80, 40, 500);

        assert_eq!(metrics.compaction_count, 2);
        assert_eq!(metrics.total_messages_saved, 60);
        assert_eq!(metrics.total_duration_ms, 510);
        assert_eq!(metrics.strategy_counts["observation_masking"], 1);
        assert_eq!(metrics.strategy_counts["summarization"], 1);
    }

    #[test]
    fn test_session_metrics_serialization() {
        let mut metrics = SessionCompactionMetrics::default();
        metrics.record("auto", 50, 30, 100);
        let json = serde_json::to_value(&metrics).unwrap();
        assert_eq!(json["compaction_count"], 1);
        assert_eq!(json["total_messages_saved"], 20);
    }

    // ====================================================================
    // Hierarchical memory tier tests
    // ====================================================================

    #[test]
    fn test_classify_memory_tiers_basic() {
        let messages: Vec<LlmMessage> = (0..30)
            .map(|i| make_user_msg(&format!("msg {i}")))
            .collect();

        let config = HierarchicalMemoryConfig {
            hot_messages: 5,
            warm_messages: 10,
        };

        let classified = classify_memory_tiers(&messages, &config);
        assert_eq!(classified.len(), 30);

        // Last 5 = hot
        assert_eq!(classified[29].0, MemoryTier::Hot);
        assert_eq!(classified[25].0, MemoryTier::Hot);

        // Next 10 = warm
        assert_eq!(classified[24].0, MemoryTier::Warm);
        assert_eq!(classified[15].0, MemoryTier::Warm);

        // Rest = cold
        assert_eq!(classified[14].0, MemoryTier::Cold);
        assert_eq!(classified[0].0, MemoryTier::Cold);
    }

    #[test]
    fn test_classify_memory_tiers_all_hot() {
        let messages: Vec<LlmMessage> =
            (0..3).map(|i| make_user_msg(&format!("msg {i}"))).collect();

        let config = HierarchicalMemoryConfig::default(); // 20 hot

        let classified = classify_memory_tiers(&messages, &config);
        assert!(classified.iter().all(|(tier, _)| *tier == MemoryTier::Hot));
    }

    #[test]
    fn test_apply_hierarchical_memory_basic() {
        let mut messages = Vec::new();

        // Cold: old tool interactions
        for i in 0..5 {
            let id = format!("old_{i}");
            messages.push(make_assistant_with_tool_call(&id, "read_file"));
            messages.push(make_tool_result(&id, &format!("old content {i}")));
        }

        // Warm: mid tool interactions
        for i in 0..3 {
            let id = format!("mid_{i}");
            messages.push(make_assistant_with_tool_call(&id, "bash"));
            messages.push(make_tool_result(&id, &format!("mid output {i}")));
        }

        // Hot: recent
        messages.push(make_user_msg("what now?"));
        messages.push(make_assistant_msg("let me check"));

        let config = HierarchicalMemoryConfig {
            hot_messages: 2,
            warm_messages: 6,
        };
        let masking_config = ObservationMaskingConfig::default();

        let result = apply_hierarchical_memory(
            &messages,
            &config,
            &masking_config,
            Some("Summary of old work"),
        );

        // Should have: 1 summary + 6 warm messages + 2 hot messages
        assert!(result.len() <= 9);
        // First should be the summary
        let first_text = extract_text(&result[0].content);
        assert!(first_text.contains("CONVERSATION_SUMMARY"));
        // Last 2 should be hot (verbatim)
        let last = extract_text(&result[result.len() - 1].content);
        assert!(last.contains("let me check"));
    }

    #[test]
    fn test_apply_hierarchical_memory_no_cold() {
        let messages = vec![make_user_msg("hello"), make_assistant_msg("hi")];

        let config = HierarchicalMemoryConfig {
            hot_messages: 5,
            warm_messages: 5,
        };

        let result = apply_hierarchical_memory(
            &messages,
            &config,
            &ObservationMaskingConfig::default(),
            None,
        );
        // All hot, no summary needed
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_memory_tier_config_from_json() {
        let config: HierarchicalMemoryConfig = serde_json::from_value(json!({
            "hot_messages": 10,
            "warm_messages": 50
        }))
        .unwrap();
        assert_eq!(config.hot_messages, 10);
        assert_eq!(config.warm_messages, 50);
    }

    #[test]
    fn test_memory_tier_config_defaults() {
        let config = HierarchicalMemoryConfig::default();
        assert_eq!(config.hot_messages, 20);
        assert_eq!(config.warm_messages, 100);
    }

    #[test]
    fn test_compaction_config_with_memory_tiers() {
        let config = CompactionConfig::from_json(&json!({
            "strategy": "auto",
            "memory_tiers": {
                "hot_messages": 15,
                "warm_messages": 80
            }
        }));
        assert_eq!(config.memory_tiers.hot_messages, 15);
        assert_eq!(config.memory_tiers.warm_messages, 80);
    }

    #[test]
    fn test_memory_tier_serialization() {
        assert_eq!(serde_json::to_value(MemoryTier::Hot).unwrap(), json!("hot"));
        assert_eq!(
            serde_json::to_value(MemoryTier::Warm).unwrap(),
            json!("warm")
        );
        assert_eq!(
            serde_json::to_value(MemoryTier::Cold).unwrap(),
            json!("cold")
        );
    }

    // ====================================================================
    // Skill content protection tests
    // ====================================================================

    #[test]
    fn test_masking_skips_activate_skill_results() {
        // 3 tool results: activate_skill (protected), read_file, bash
        // With keep_recent=1, only read_file should be masked (activate_skill exempt)
        let messages = vec![
            make_assistant_with_tool_call("call_skill", "activate_skill"),
            make_tool_result(
                "call_skill",
                "You are a code review agent. Follow these instructions...",
            ),
            make_assistant_msg("Skill activated"),
            make_assistant_with_tool_call("call_read", "read_file"),
            make_tool_result(
                "call_read",
                "file contents that are long enough to be masked by observation masking because they exceed one hundred characters easily",
            ),
            make_assistant_msg("got it"),
            make_assistant_with_tool_call("call_bash", "bash"),
            make_tool_result("call_bash", "command output"),
        ];

        let config = ObservationMaskingConfig {
            keep_recent_tool_outputs: 1,
            summary_format: MaskingSummaryFormat::OneLine,
        };
        let result = apply_observation_masking(&messages, &config);

        // activate_skill result should be verbatim
        assert_eq!(
            extract_text(&result.messages[1].content),
            "You are a code review agent. Follow these instructions..."
        );
        // read_file result should be masked (it's the only maskable old one)
        assert!(extract_text(&result.messages[4].content).starts_with('['));
        // bash result should be verbatim (most recent maskable)
        assert_eq!(extract_text(&result.messages[7].content), "command output");
        assert_eq!(result.masked_count, 1);
    }

    #[test]
    fn test_masking_all_activate_skill_exempt_from_count() {
        // 2 activate_skill results + 1 regular tool result
        // With keep_recent=0, only the regular one should be masked
        let messages = vec![
            make_assistant_with_tool_call("s1", "activate_skill"),
            make_tool_result("s1", "Skill 1 instructions"),
            make_assistant_with_tool_call("s2", "activate_skill"),
            make_tool_result("s2", "Skill 2 instructions"),
            make_assistant_with_tool_call("c1", "bash"),
            make_tool_result("c1", "output"),
        ];

        let config = ObservationMaskingConfig {
            keep_recent_tool_outputs: 0,
            summary_format: MaskingSummaryFormat::OneLine,
        };
        let result = apply_observation_masking(&messages, &config);

        assert_eq!(result.masked_count, 1);
        // Both skill results preserved
        assert_eq!(
            extract_text(&result.messages[1].content),
            "Skill 1 instructions"
        );
        assert_eq!(
            extract_text(&result.messages[3].content),
            "Skill 2 instructions"
        );
        assert_complete_tool_exchanges(&result.messages);
    }

    #[test]
    fn test_aggressive_trim_preserves_skill_messages() {
        // Create messages where budget only fits ~2 messages, but skill messages
        // should always be preserved
        let messages = vec![
            make_user_msg(&"s".repeat(400)), // system: 100 tokens
            make_assistant_with_tool_call("skill1", "activate_skill"),
            make_tool_result("skill1", "Important skill instructions"),
            make_user_msg(&"a".repeat(400)),      // old: 100 tokens
            make_assistant_msg(&"b".repeat(400)), // old: 100 tokens
            make_user_msg(&"c".repeat(400)),      // recent: 100 tokens
            make_assistant_msg(&"d".repeat(400)), // recent: 100 tokens
        ];

        // Budget for system + skill call + skill result + 1 recent = ~400 tokens
        // Should keep: system, skill call, skill result, and as many recent as fit
        let target_tokens = 400;
        let result = aggressive_trim(&messages, target_tokens, true);

        // Verify skill messages are preserved
        let has_skill_result = result.iter().any(|m| {
            m.role == LlmMessageRole::Tool
                && extract_text(&m.content) == "Important skill instructions"
        });
        assert!(
            has_skill_result,
            "Skill tool result must survive aggressive trim"
        );

        let has_skill_call = result.iter().any(|m| {
            m.tool_calls
                .as_ref()
                .is_some_and(|calls| calls.iter().any(|tc| tc.name == "activate_skill"))
        });
        assert!(
            has_skill_call,
            "Skill tool call must survive aggressive trim"
        );
    }

    #[test]
    fn aggressive_trim_keeps_parallel_tool_calls_atomic() {
        let messages = vec![
            make_user_msg("system"),
            make_user_msg("original task"),
            make_assistant_with_tool_calls(&[
                ("call_skill", "activate_skill"),
                ("call_bash", "bash"),
            ]),
            make_tool_result("call_skill", "Important skill instructions"),
            make_tool_result("call_bash", &"output ".repeat(100)),
            make_user_msg(&"recent user ".repeat(40)),
            make_assistant_msg(&"recent answer ".repeat(40)),
        ];

        let result = aggressive_trim(&messages, 300, true);
        let visible_calls: Vec<_> = result
            .iter()
            .flat_map(|message| message.tool_calls.iter().flatten())
            .map(|call| call.id.as_str())
            .collect();
        let visible_results: Vec<_> = result
            .iter()
            .filter_map(|message| message.tool_call_id.as_deref())
            .collect();

        assert!(visible_calls.contains(&"call_skill"));
        assert!(visible_results.contains(&"call_skill"));
        assert_eq!(
            visible_calls, visible_results,
            "reduction must not expose a call without its result"
        );
        assert_complete_tool_exchanges(&result);
    }

    #[test]
    fn hierarchical_memory_prunes_only_evicted_calls_from_a_protected_parallel_batch() {
        let messages = vec![
            make_assistant_with_tool_calls(&[
                ("call_skill", "activate_skill"),
                ("call_bash", "bash"),
            ]),
            make_tool_result("call_skill", "Important skill instructions"),
            make_tool_result("call_bash", "ordinary output"),
            make_user_msg("recent question"),
            make_assistant_msg("recent answer"),
        ];
        let result = apply_hierarchical_memory(
            &messages,
            &HierarchicalMemoryConfig {
                hot_messages: 2,
                warm_messages: 0,
            },
            &ObservationMaskingConfig::default(),
            Some("Summary of old work"),
        );

        assert_complete_tool_exchanges(&result);
        let calls: Vec<_> = result
            .iter()
            .flat_map(|message| message.tool_calls.iter().flatten())
            .map(|call| call.id.as_str())
            .collect();
        assert_eq!(calls, vec!["call_skill"]);
    }

    #[test]
    fn summary_boundary_drops_a_recent_result_whose_call_was_summarized() {
        let recent = vec![
            make_tool_result("call_old", "old result"),
            make_user_msg("continue"),
        ];
        let result = compose_summary_with_recent(None, "Earlier work", &recent);

        assert_complete_tool_exchanges(&result);
        assert!(result.iter().all(|message| message.tool_call_id.is_none()));
        assert!(
            result
                .iter()
                .any(|message| extract_text(&message.content) == "continue")
        );
    }

    #[test]
    fn test_hierarchical_memory_rescues_skill_from_cold_tier() {
        let mut messages = Vec::new();

        // Cold tier: old messages including a skill activation
        messages.push(make_assistant_with_tool_call("skill1", "activate_skill"));
        messages.push(make_tool_result(
            "skill1",
            "You must always validate input.",
        ));
        for i in 0..8 {
            let id = format!("old_{i}");
            messages.push(make_assistant_with_tool_call(&id, "read_file"));
            messages.push(make_tool_result(&id, &format!("old content {i}")));
        }

        // Warm tier
        for i in 0..3 {
            let id = format!("mid_{i}");
            messages.push(make_assistant_with_tool_call(&id, "bash"));
            messages.push(make_tool_result(&id, &format!("mid output {i}")));
        }

        // Hot tier
        messages.push(make_user_msg("what now?"));
        messages.push(make_assistant_msg("let me check"));

        let config = HierarchicalMemoryConfig {
            hot_messages: 2,
            warm_messages: 6,
        };
        let masking_config = ObservationMaskingConfig::default();

        let result = apply_hierarchical_memory(
            &messages,
            &config,
            &masking_config,
            Some("Summary of old work"),
        );

        // The protected skill messages from cold tier should be rescued
        let has_skill_instructions = result
            .iter()
            .any(|m| extract_text(&m.content).contains("You must always validate input."));
        assert!(
            has_skill_instructions,
            "Skill instructions from cold tier must be rescued into output"
        );

        // Summary should still be present
        assert!(extract_text(&result[0].content).contains("CONVERSATION_SUMMARY"));
    }

    #[test]
    fn test_is_protected_tool_result_detection() {
        let messages = vec![
            make_assistant_with_tool_call("s1", "activate_skill"),
            make_tool_result("s1", "skill content"),
            make_assistant_with_tool_call("r1", "read_file"),
            make_tool_result("r1", "file content"),
        ];

        // activate_skill result is protected
        assert!(is_protected_tool_result(&messages, &messages[1]));
        // read_file result is not
        assert!(!is_protected_tool_result(&messages, &messages[3]));
        // non-tool message is not
        assert!(!is_protected_tool_result(&messages, &messages[0]));
    }

    #[test]
    fn test_is_protected_tool_call_message_detection() {
        let skill_call = make_assistant_with_tool_call("s1", "activate_skill");
        let regular_call = make_assistant_with_tool_call("r1", "read_file");
        let user_msg = make_user_msg("hello");

        assert!(is_protected_tool_call_message(&skill_call));
        assert!(!is_protected_tool_call_message(&regular_call));
        assert!(!is_protected_tool_call_message(&user_msg));
    }

    #[test]
    fn test_default_preserve_includes_skill_instructions() {
        let config = SummarizationConfig::default();
        assert!(
            config.preserve.contains(&"skill_instructions".to_string()),
            "Default preserve list must include skill_instructions"
        );
    }

    #[test]
    fn test_summarization_prompt_mentions_skill_protection() {
        let config = SummarizationConfig::default();
        let prompt = build_summarization_prompt(&config);
        assert!(
            prompt.contains("activate_skill"),
            "Summarization prompt must instruct LLM to preserve skill content"
        );
    }

    #[test]
    fn test_aggressive_trim_protected_exceed_budget() {
        // When protected messages alone exceed the budget, keep as many as
        // fit (newest first) and drop non-protected entirely.
        let messages = vec![
            make_user_msg(&"s".repeat(400)), // system ~100 tokens
            make_assistant_with_tool_call("skill1", "activate_skill"), // protected
            make_tool_result("skill1", &"x".repeat(800)), // protected ~200 tokens
            make_assistant_with_tool_call("skill2", "activate_skill"), // protected
            make_tool_result("skill2", &"y".repeat(800)), // protected ~200 tokens
            make_user_msg(&"z".repeat(400)), // non-protected
        ];

        // Budget only fits system + ~1 protected pair
        let result = aggressive_trim(&messages, 200, true);

        // Must not exceed budget — non-protected messages dropped
        let has_non_protected = result
            .iter()
            .any(|m| m.role == LlmMessageRole::User && extract_text(&m.content).contains('z'));
        assert!(
            !has_non_protected,
            "Non-protected messages must be dropped when protected exceed budget"
        );
    }

    #[test]
    fn test_format_messages_no_truncate_protected_tool_result() {
        // Protected tool results should not be truncated at 2000 chars
        let long_instructions = "a".repeat(5000);
        let messages = vec![
            make_assistant_with_tool_call("s1", "activate_skill"),
            make_tool_result("s1", &long_instructions),
            make_assistant_with_tool_call("r1", "read_file"),
            make_tool_result("r1", &"b".repeat(5000)),
        ];

        let formatted = format_messages_for_summarization(&messages);

        // Skill result: full 5000-char content present, not truncated
        assert!(
            formatted.contains(&long_instructions),
            "Protected tool result must not be truncated"
        );
        // Regular result: should be truncated
        assert!(
            formatted.contains("[truncated, 5000 chars total]"),
            "Non-protected tool result should be truncated"
        );
    }

    #[test]
    fn test_hierarchical_memory_cross_tier_boundary_protection() {
        // The activate_skill tool-call is in cold tier, but its tool-result
        // lands in warm tier. The result must still be protected from masking.
        let mut messages = Vec::new();

        // Cold tier: skill call + filler to push result into warm tier
        messages.push(make_assistant_with_tool_call("skill1", "activate_skill"));
        for i in 0..9 {
            let id = format!("cold_{i}");
            messages.push(make_assistant_with_tool_call(&id, "read_file"));
            messages.push(make_tool_result(&id, &format!("cold content {i}")));
        }

        // Warm tier starts here — skill result is first warm message
        messages.push(make_tool_result(
            "skill1",
            "Cross-tier skill instructions that must survive",
        ));
        for i in 0..2 {
            let id = format!("warm_{i}");
            messages.push(make_assistant_with_tool_call(&id, "bash"));
            messages.push(make_tool_result(&id, &format!("warm output {i}")));
        }

        // Hot tier
        messages.push(make_user_msg("continue"));
        messages.push(make_assistant_msg("ok"));

        let config = HierarchicalMemoryConfig {
            hot_messages: 2,
            warm_messages: 5, // skill result + 2 bash pairs
        };
        let masking_config = ObservationMaskingConfig {
            keep_recent_tool_outputs: 0,
            summary_format: MaskingSummaryFormat::OneLine,
        };

        let result = apply_hierarchical_memory(&messages, &config, &masking_config, None);

        let has_skill_instructions = result.iter().any(|m| {
            extract_text(&m.content).contains("Cross-tier skill instructions that must survive")
        });
        assert!(
            has_skill_instructions,
            "Skill result in warm tier with call in cold tier must be protected"
        );
    }
}
