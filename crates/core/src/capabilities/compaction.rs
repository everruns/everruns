//! Compaction Capability
//!
//! Configurable context compaction strategy. Users choose between native provider
//! compaction (e.g., OpenAI /responses/compact) and our own strategies (observation
//! masking, LLM summarization). See specs/compaction.md.
//!
//! Design decisions:
//! - Strategy selection is per-agent/harness via `AgentCapabilityConfig`
//! - Native and our own strategies coexist as first-class options
//! - The `auto` cascade: observation masking → native → summarization
//! - Proactive compaction at a configurable budget threshold, not just on error

use super::{Capability, CapabilityStatus};
use serde::{Deserialize, Serialize};

/// Capability ID for compaction.
pub const COMPACTION_CAPABILITY_ID: &str = "compaction";

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
    5
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
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            strategy: CompactionStrategy::default(),
            proactive: default_proactive(),
            budget_percent: default_budget_percent(),
            observation_masking: ObservationMaskingConfig::default(),
            summarization: SummarizationConfig::default(),
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
}

// ============================================================================
// Observation Masking
// ============================================================================

use crate::llm_driver_registry::{LlmContentPart, LlmMessage, LlmMessageContent, LlmMessageRole};

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
pub fn apply_observation_masking(
    messages: &[LlmMessage],
    config: &ObservationMaskingConfig,
) -> ObservationMaskingResult {
    let tool_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == LlmMessageRole::Tool)
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
                MaskingSummaryFormat::OneLine => {
                    format_one_line_summary(&tool_name, &msg.content)
                }
                MaskingSummaryFormat::HeadTail => format_head_tail_summary(&msg.content),
            };
            result.push(LlmMessage {
                role: LlmMessageRole::Tool,
                content: LlmMessageContent::Text(summary),
                tool_calls: msg.tool_calls.clone(),
                tool_call_id: msg.tool_call_id.clone(),
                phase: msg.phase.clone(),
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
        if msg.role == LlmMessageRole::Assistant {
            if let Some(ref tool_calls) = msg.tool_calls {
                for tc in tool_calls {
                    if tc.id == *call_id {
                        return tc.name.clone();
                    }
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

        // Truncate very long messages to avoid blowing up the summarization prompt
        let truncated = if content.len() > 2000 {
            format!(
                "{}... [truncated, {} chars total]",
                &content[..2000],
                content.len()
            )
        } else {
            content
        };

        parts.push(format!("[{role}]: {truncated}"));
    }
    parts.join("\n\n")
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
        assert!(cap.message_filter_provider().is_none());
    }

    #[test]
    fn test_default_config() {
        let config = CompactionConfig::default();
        assert_eq!(config.strategy, CompactionStrategy::Auto);
        assert!(config.proactive);
        assert!((config.budget_percent - 0.85).abs() < f32::EPSILON);
        assert_eq!(config.observation_masking.keep_recent_tool_outputs, 5);
        assert_eq!(
            config.observation_masking.summary_format,
            MaskingSummaryFormat::OneLine
        );
        assert!(config.summarization.model.is_none());
        assert_eq!(config.summarization.preserve.len(), 4);
        assert!(config.summarization.instructions.is_none());
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
        for format in [MaskingSummaryFormat::OneLine, MaskingSummaryFormat::HeadTail] {
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
        assert!(text.starts_with('['), "Expected masked summary, got: {text}");
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
}
