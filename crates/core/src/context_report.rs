use crate::capabilities::{CapabilityRegistry, SystemPromptContext};
use crate::events::{LlmGenerationData, TokenUsage};
use crate::llm_model_profiles::get_model_profile;
use crate::message::MessageRole;
use crate::runtime_context::AssembledTurnContext;
use crate::tool_types::ToolDefinition;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ContextReportSection {
    /// Stable section key (e.g. `system_prompt`, `tools`, `history`). Used as a join key for contributions.
    pub key: String,
    /// Human-readable section label suitable for UI display.
    pub label: String,
    /// Total tokens this section contributes to the assembled context.
    pub tokens: u32,
    /// Number of items this section comprises (messages, tool defs, etc.).
    pub items: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ContextReportContribution {
    /// Section this contribution rolls up into; matches `ContextReportSection.key`.
    pub section_key: String,
    /// Stable id of the contributing source (capability id, tool name, message id, etc.).
    pub source_id: String,
    /// Human-readable label suitable for UI display.
    pub label: String,
    /// Tokens this single source contributes to the assembled context.
    pub tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SessionContextReport {
    /// Prefixed session identifier this report describes.
    pub session_id: String,
    /// Model identifier the report's token estimates target (used to scope context-window math).
    pub model: String,
    /// Total context window size in tokens for `model`. `None` if the model's profile lacks limits data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window_tokens: Option<u32>,
    /// Estimated number of input tokens consumed by the next generation given the current context.
    pub estimated_input_tokens: u32,
    /// Logical sections of the assembled context (system prompt, tool defs, message history, etc.) for inspection.
    pub sections: Vec<ContextReportSection>,
    /// Per-source token contributions (per-tool, per-capability, per-message) for attribution.
    pub contributions: Vec<ContextReportContribution>,
    /// Cumulative LLM usage observed across the session so far (token + cost rollup).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cumulative_usage: Option<TokenUsage>,
}

pub fn build_session_context_report_from_generation(
    session_id: impl Into<String>,
    generation: &LlmGenerationData,
    context_window_tokens: Option<u32>,
    cumulative_usage: Option<TokenUsage>,
) -> SessionContextReport {
    let mut builder = ContextReportBuilder::default();

    for message in &generation.messages {
        if message.role == MessageRole::System {
            add_system_prompt_breakdown(&mut builder, &message.content_to_llm_string());
        } else {
            builder.add(
                "conversation",
                "Conversation",
                estimate_serialized_tokens(message),
                1,
            );
        }
    }

    for tool in &generation.tools {
        let key = if tool.name.starts_with("mcp_") {
            "mcp"
        } else if matches!(
            tool.name.as_str(),
            "spawn_subagent" | "get_subagents" | "message_subagent"
        ) {
            "subagents"
        } else {
            "tools"
        };
        builder.add(key, section_label(key), estimate_serialized_tokens(tool), 1);
    }

    let sections = builder.sections();
    let estimated_input_tokens = sections.iter().map(|section| section.tokens).sum();
    let contributions = builder.contributions;

    SessionContextReport {
        session_id: session_id.into(),
        model: generation.metadata.model.clone(),
        context_window_tokens,
        estimated_input_tokens,
        sections,
        contributions,
        cumulative_usage,
    }
}

pub async fn build_session_context_report(
    assembled: &AssembledTurnContext,
    _capability_registry: &CapabilityRegistry,
    _prompt_ctx: &SystemPromptContext,
) -> SessionContextReport {
    let mut builder = ContextReportBuilder::default();

    add_system_prompt_breakdown(&mut builder, &assembled.runtime_agent.system_prompt);

    for tool in &assembled.runtime_agent.tools {
        let section_key = classify_tool(tool);
        builder.add(
            section_key,
            section_label(section_key),
            estimate_tool_tokens(tool),
            1,
        );
    }

    for message in &assembled.messages {
        builder.add(
            "conversation",
            "Conversation",
            estimate_serialized_tokens(message),
            1,
        );
    }

    let sections = builder.sections();
    let estimated_input_tokens = sections.iter().map(|section| section.tokens).sum();
    let context_window_tokens = get_model_profile(
        &assembled.model_with_provider.provider_type,
        &assembled.runtime_agent.model,
    )
    .and_then(|profile| profile.limits)
    .and_then(|limits| u32::try_from(limits.context).ok());

    SessionContextReport {
        session_id: assembled.session.id.to_string(),
        model: assembled.runtime_agent.model.clone(),
        context_window_tokens,
        estimated_input_tokens,
        sections,
        contributions: builder.contributions,
        cumulative_usage: assembled.session.usage.clone(),
    }
}

fn add_system_prompt_breakdown(builder: &mut ContextReportBuilder, prompt: &str) {
    let mut cursor = 0usize;
    while let Some(relative_start) = prompt[cursor..].find("<capability id=\"") {
        let start = cursor + relative_start;
        if start > cursor {
            builder.add(
                "system_prompt",
                "System prompt",
                estimate_text_tokens(&prompt[cursor..start]),
                1,
            );
        }

        let id_start = start + "<capability id=\"".len();
        let Some(relative_id_end) = prompt[id_start..].find('"') else {
            break;
        };
        let id_end = id_start + relative_id_end;
        let capability_id = &prompt[id_start..id_end];
        let Some(relative_end) = prompt[id_end..].find("</capability>") else {
            break;
        };
        let end = id_end + relative_end + "</capability>".len();
        let key = classify_capability_prompt(capability_id);
        let tokens = estimate_text_tokens(&prompt[start..end]);
        builder.add(key, section_label(key), tokens, 1);
        builder.contributions.push(ContextReportContribution {
            section_key: key.to_string(),
            source_id: capability_id.to_string(),
            label: capability_id.to_string(),
            tokens,
        });
        cursor = end;
    }

    if cursor < prompt.len() {
        builder.add(
            "system_prompt",
            "System prompt",
            estimate_text_tokens(&prompt[cursor..]),
            1,
        );
    }
}

#[derive(Default)]
struct ContextReportBuilder {
    sections: Vec<ContextReportSection>,
    contributions: Vec<ContextReportContribution>,
}

impl ContextReportBuilder {
    fn add(&mut self, key: &str, label: &str, tokens: u32, items: u32) {
        if tokens == 0 && items == 0 {
            return;
        }
        if let Some(section) = self.sections.iter_mut().find(|section| section.key == key) {
            section.tokens = section.tokens.saturating_add(tokens);
            section.items = section.items.saturating_add(items);
            return;
        }
        self.sections.push(ContextReportSection {
            key: key.to_string(),
            label: label.to_string(),
            tokens,
            items,
        });
    }

    fn sections(&self) -> Vec<ContextReportSection> {
        let mut sections = self.sections.clone();
        let order = [
            "system_prompt",
            "tools",
            "rules",
            "skills",
            "mcp",
            "subagents",
            "conversation",
        ];
        sections.sort_by_key(|section| {
            order
                .iter()
                .position(|key| *key == section.key)
                .unwrap_or(order.len())
        });
        sections
    }
}

fn section_label(key: &str) -> &'static str {
    match key {
        "system_prompt" => "System prompt",
        "rules" => "Rules",
        "skills" => "Skills",
        "mcp" => "MCP",
        "subagents" => "Subagents",
        "conversation" => "Conversation",
        _ => "Tools",
    }
}

fn classify_capability_prompt(capability_id: &str) -> &'static str {
    if capability_id == "agent_instructions" {
        "rules"
    } else if capability_id == "skills" || capability_id.starts_with("skill:") {
        "skills"
    } else if capability_id == "subagents" {
        "subagents"
    } else if capability_id.starts_with("mcp:") {
        "mcp"
    } else {
        "tools"
    }
}

fn classify_tool(tool: &ToolDefinition) -> &'static str {
    let name = tool.name();
    let category = tool.category().unwrap_or_default();
    if name.starts_with("mcp_") || category.eq_ignore_ascii_case("mcp") {
        "mcp"
    } else if matches!(
        name,
        "spawn_subagent" | "get_subagents" | "message_subagent"
    ) {
        "subagents"
    } else {
        "tools"
    }
}

fn estimate_tool_tokens(tool: &ToolDefinition) -> u32 {
    estimate_serialized_tokens(tool)
}

fn estimate_serialized_tokens(value: &impl Serialize) -> u32 {
    serde_json::to_string(value)
        .ok()
        .map(|text| estimate_text_tokens(&text))
        .unwrap_or(0)
}

pub fn estimate_text_tokens(text: &str) -> u32 {
    let chars = text.chars().count();
    if chars == 0 {
        0
    } else {
        u32::try_from(chars.div_ceil(4)).unwrap_or(u32::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BuiltinTool;
    use serde_json::json;

    #[test]
    fn classifies_attribution_sections() {
        assert_eq!(classify_capability_prompt("agent_instructions"), "rules");
        assert_eq!(classify_capability_prompt("skills"), "skills");
        assert_eq!(classify_capability_prompt("skill:abc"), "skills");
        assert_eq!(classify_capability_prompt("mcp:abc"), "mcp");
        assert_eq!(classify_capability_prompt("subagents"), "subagents");
    }

    #[test]
    fn classifies_mcp_and_subagent_tools() {
        let mcp = ToolDefinition::Builtin(BuiltinTool {
            name: "mcp_docs_search".into(),
            display_name: None,
            description: "Search docs".into(),
            parameters: json!({"type": "object"}),
            policy: Default::default(),
            category: None,
            deferrable: Default::default(),
            hints: Default::default(),
        });
        let subagent = ToolDefinition::Builtin(BuiltinTool {
            name: "spawn_subagent".into(),
            display_name: None,
            description: "Spawn".into(),
            parameters: json!({"type": "object"}),
            policy: Default::default(),
            category: None,
            deferrable: Default::default(),
            hints: Default::default(),
        });

        assert_eq!(classify_tool(&mcp), "mcp");
        assert_eq!(classify_tool(&subagent), "subagents");
    }

    #[test]
    fn estimates_tokens_with_minimum_for_nonempty_text() {
        assert_eq!(estimate_text_tokens(""), 0);
        assert_eq!(estimate_text_tokens("abc"), 1);
        assert_eq!(estimate_text_tokens("abcd"), 1);
        assert_eq!(estimate_text_tokens("abcde"), 2);
    }

    #[test]
    fn generation_report_attributes_capability_prompt_blocks() {
        let data = LlmGenerationData::success(
            vec![crate::Message::system(
                "<capability id=\"agent_instructions\">Rules</capability>\n\n<system-prompt>\nBase\n</system-prompt>",
            )],
            vec![],
            Some("ok".into()),
            vec![],
            "gpt-test".into(),
            Some("openai".into()),
            None,
            None,
            None,
        );

        let report =
            build_session_context_report_from_generation("session_test", &data, None, None);
        assert!(report.sections.iter().any(|section| section.key == "rules"));
        assert!(
            report
                .contributions
                .iter()
                .any(|contribution| contribution.source_id == "agent_instructions")
        );
    }

    #[test]
    fn empty_system_prompt_does_not_add_section() {
        let mut builder = ContextReportBuilder::default();
        add_system_prompt_breakdown(&mut builder, "");
        assert!(builder.sections().is_empty());
    }
}
