use crate::capabilities::{CapabilityRegistry, SystemPromptContext};
use crate::events::{LlmGenerationData, TokenUsage, ToolDefinitionSummary};
use crate::mcp_server::parse_mcp_tool_name;
use crate::message::{ContentPart, Message, MessageRole};
use crate::model_profiles::get_model_profile;
use crate::runtime_context::AssembledTurnContext;
use crate::tool_types::ToolDefinition;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One logical section of the assembled LLM context (system prompt, tool
/// definitions, message history, etc.) with its rolled-up token budget.
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

/// Single-source token contribution within a `ContextReportSection` — the
/// per-tool / per-capability / per-message attribution that lets operators
/// see which source is eating the context window.
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

/// Token-budget report for a session — a model-aware breakdown of the
/// context window into named sections plus per-source contributions, so
/// callers can answer "what's filling the context?" without reverse-
/// engineering the prompt assembly.
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
    let tool_calls_by_id = tool_calls_by_id(&generation.messages);

    for message in &generation.messages {
        if message.role == MessageRole::System {
            add_system_prompt_breakdown(&mut builder, &message.content_to_llm_string());
        } else {
            add_message_breakdown(&mut builder, message, &tool_calls_by_id);
        }
    }

    for tool in &generation.tools {
        let key = classify_tool_summary(tool);
        let tokens = estimate_serialized_tokens(tool);
        let (source_id, label) = tool_summary_contribution_source(tool, key);
        builder.add_contribution(key, source_id, label, tokens, 1);
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
        let tokens = estimate_tool_tokens(tool);
        let (source_id, label) = tool_definition_contribution_source(tool, section_key);
        builder.add_contribution(section_key, source_id, label, tokens, 1);
    }

    let tool_calls_by_id = tool_calls_by_id(&assembled.messages);
    for message in &assembled.messages {
        add_message_breakdown(&mut builder, message, &tool_calls_by_id);
    }

    let sections = builder.sections();
    let estimated_input_tokens = sections.iter().map(|section| section.tokens).sum();
    let context_window_tokens = get_model_profile(
        &assembled.model.provider_type,
        &assembled.runtime_agent.model,
    )
    .and_then(|profile| profile.limits)
    .and_then(|limits| u32::try_from(limits.context).ok());

    SessionContextReport {
        session_id: assembled.snapshot.session_id.to_string(),
        model: assembled.runtime_agent.model.clone(),
        context_window_tokens,
        estimated_input_tokens,
        sections,
        contributions: builder.contributions,
        cumulative_usage: assembled.snapshot.cumulative_usage.clone(),
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
        builder.add_contribution(
            key,
            capability_id.to_string(),
            capability_label(capability_id),
            tokens,
            1,
        );
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

    fn add_contribution(
        &mut self,
        section_key: &str,
        source_id: String,
        label: String,
        tokens: u32,
        items: u32,
    ) {
        self.add(section_key, section_label(section_key), tokens, items);
        if tokens == 0 {
            return;
        }
        if let Some(contribution) = self.contributions.iter_mut().find(|contribution| {
            contribution.section_key == section_key && contribution.source_id == source_id
        }) {
            contribution.tokens = contribution.tokens.saturating_add(tokens);
            return;
        }
        self.contributions.push(ContextReportContribution {
            section_key: section_key.to_string(),
            source_id,
            label,
            tokens,
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
            "plugins",
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
        "plugins" => "Plugins",
        "conversation" => "Conversation",
        _ => "Tools",
    }
}

fn capability_label(capability_id: &str) -> String {
    if let Some(skill_id) = capability_id.strip_prefix("skill:") {
        format!("/{skill_id}")
    } else if let Some(mcp_id) = capability_id.strip_prefix("mcp:") {
        mcp_id.to_string()
    } else {
        capability_id.to_string()
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
    let capability_id = tool
        .capability_attribution()
        .map(|(capability_id, _)| capability_id)
        .unwrap_or_default();
    if is_mcp_tool_source(name, category, capability_id) {
        "mcp"
    } else if is_agent_delegation_tool_name(name) {
        "subagents"
    } else if is_skill_tool_source(name, category, capability_id) {
        "skills"
    } else if category.eq_ignore_ascii_case("plugins") || category.eq_ignore_ascii_case("plugin") {
        "plugins"
    } else {
        "tools"
    }
}

fn classify_tool_summary(tool: &ToolDefinitionSummary) -> &'static str {
    let category = tool.category.as_deref().unwrap_or_default();
    let capability_id = tool.capability_id.as_deref().unwrap_or_default();
    if is_mcp_tool_source(&tool.name, category, capability_id) {
        "mcp"
    } else if is_agent_delegation_tool_name(&tool.name) {
        "subagents"
    } else if is_skill_tool_source(&tool.name, category, capability_id) {
        "skills"
    } else if category.eq_ignore_ascii_case("plugins") || category.eq_ignore_ascii_case("plugin") {
        "plugins"
    } else {
        "tools"
    }
}

fn is_mcp_tool_source(name: &str, category: &str, capability_id: &str) -> bool {
    name.starts_with("mcp_")
        || category.eq_ignore_ascii_case("mcp")
        || category.eq_ignore_ascii_case("mcp servers")
        || capability_id.starts_with("mcp:")
}

fn is_skill_tool_source(name: &str, category: &str, capability_id: &str) -> bool {
    matches!(name, "list_skills" | "activate_skill")
        || category.eq_ignore_ascii_case("skills")
        || capability_id == "skills"
        || capability_id.starts_with("skill:")
}

fn is_agent_delegation_tool_name(name: &str) -> bool {
    matches!(name, "spawn_agent")
}

fn tool_definition_contribution_source(
    tool: &ToolDefinition,
    section_key: &str,
) -> (String, String) {
    let capability_attribution = tool.capability_attribution();
    tool_contribution_source(
        tool.name(),
        tool.display_name(),
        capability_attribution.map(|(id, _)| id),
        capability_attribution.and_then(|(_, name)| name),
        section_key,
    )
}

fn tool_summary_contribution_source(
    tool: &ToolDefinitionSummary,
    section_key: &str,
) -> (String, String) {
    tool_contribution_source(
        &tool.name,
        tool.display_name.as_deref(),
        tool.capability_id.as_deref(),
        tool.capability_name.as_deref(),
        section_key,
    )
}

fn tool_contribution_source(
    tool_name: &str,
    display_name: Option<&str>,
    capability_id: Option<&str>,
    capability_name: Option<&str>,
    section_key: &str,
) -> (String, String) {
    match section_key {
        "mcp" => {
            let server = parse_mcp_tool_name(tool_name).map(|(server, _)| server);
            let source_id = capability_id
                .map(str::to_string)
                .or_else(|| server.as_ref().map(|server| format!("mcp:{server}")))
                .unwrap_or_else(|| format!("tool:{tool_name}"));
            let label = capability_name
                .map(str::to_string)
                .or(server)
                .unwrap_or_else(|| display_name.unwrap_or(tool_name).to_string());
            (source_id, label)
        }
        "skills" => {
            let source_id = capability_id
                .map(str::to_string)
                .unwrap_or_else(|| "skills:tools".to_string());
            let label = capability_name
                .map(str::to_string)
                .unwrap_or_else(|| "Skills tools".to_string());
            (source_id, label)
        }
        "subagents" => ("subagents:tools".to_string(), "Subagent tools".to_string()),
        "plugins" => {
            let source_id = capability_id
                .map(str::to_string)
                .unwrap_or_else(|| format!("plugin:{tool_name}"));
            let label = capability_name
                .or(display_name)
                .unwrap_or(tool_name)
                .to_string();
            (source_id, label)
        }
        _ => (
            format!("tool:{tool_name}"),
            display_name.unwrap_or(tool_name).to_string(),
        ),
    }
}

fn tool_calls_by_id(messages: &[Message]) -> BTreeMap<String, String> {
    let mut tool_calls = BTreeMap::new();
    for message in messages {
        for tool_call in message.tool_calls() {
            tool_calls.insert(tool_call.id.clone(), tool_call.name.clone());
        }
    }
    tool_calls
}

fn add_message_breakdown(
    builder: &mut ContextReportBuilder,
    message: &Message,
    tool_calls_by_id: &BTreeMap<String, String>,
) {
    let tokens = estimate_serialized_tokens(message);
    if let Some((section_key, source_id, label)) =
        message_contribution_source(message, tool_calls_by_id)
    {
        builder.add_contribution(section_key, source_id, label, tokens, 1);
        return;
    }

    builder.add("conversation", "Conversation", tokens, 1);
}

fn message_contribution_source(
    message: &Message,
    tool_calls_by_id: &BTreeMap<String, String>,
) -> Option<(&'static str, String, String)> {
    if message.role != MessageRole::ToolResult {
        return None;
    }
    let tool_call_id = message.tool_call_id()?;
    let tool_name = tool_calls_by_id.get(tool_call_id)?;
    if tool_name == "activate_skill" {
        let skill_name = extract_json_string_field(message, "skill")?;
        return Some((
            "skills",
            format!("skill:{skill_name}"),
            format!("/{skill_name}"),
        ));
    }
    if is_agent_delegation_tool_name(tool_name) {
        let name = extract_json_string_field(message, "name").unwrap_or_else(|| "Subagent".into());
        return Some(("subagents", format!("subagent:{name}"), name));
    }
    if let Some((server, _)) = parse_mcp_tool_name(tool_name) {
        return Some(("mcp", format!("mcp:{server}"), server));
    }
    None
}

fn extract_json_string_field(message: &Message, field: &str) -> Option<String> {
    message.content.iter().find_map(|part| {
        let ContentPart::ToolResult(result) = part else {
            return None;
        };
        result
            .result
            .as_ref()
            .and_then(|value| value.get(field))
            .and_then(|value| value.as_str())
            .map(str::to_string)
    })
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
    fn classifies_mcp_and_delegation_tools() {
        let mcp = ToolDefinition::Builtin(BuiltinTool {
            name: "mcp_docs__search".into(),
            display_name: None,
            description: "Search docs".into(),
            parameters: json!({"type": "object"}),
            policy: Default::default(),
            category: None,
            deferrable: Default::default(),
            hints: Default::default(),
            full_parameters: None,
        });
        let delegation = ToolDefinition::Builtin(BuiltinTool {
            name: "spawn_agent".into(),
            display_name: None,
            description: "Spawn".into(),
            parameters: json!({"type": "object"}),
            policy: Default::default(),
            category: None,
            deferrable: Default::default(),
            hints: Default::default(),
            full_parameters: None,
        });

        assert_eq!(classify_tool(&mcp), "mcp");
        assert_eq!(classify_tool(&delegation), "subagents");
    }

    #[test]
    fn classifies_skill_tools() {
        let skill = ToolDefinition::Builtin(BuiltinTool {
            name: "activate_skill".into(),
            display_name: None,
            description: "Activate".into(),
            parameters: json!({"type": "object"}),
            policy: Default::default(),
            category: None,
            deferrable: Default::default(),
            hints: Default::default(),
            full_parameters: None,
        });

        assert_eq!(classify_tool(&skill), "skills");
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
                "<system-prompt>\nBase\n</system-prompt>\n\n<capability id=\"agent_instructions\">Rules</capability>",
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
    fn generation_report_attributes_tool_definitions_by_source() {
        let data = LlmGenerationData::success(
            vec![crate::Message::user("hello")],
            vec![
                crate::events::ToolDefinitionSummary {
                    name: "mcp_docs__search".into(),
                    display_name: None,
                    category: Some("MCP Servers".into()),
                    capability_id: None,
                    capability_name: None,
                    description: "Search docs".into(),
                },
                crate::events::ToolDefinitionSummary {
                    name: "mcp_docs__read".into(),
                    display_name: None,
                    category: Some("MCP Servers".into()),
                    capability_id: None,
                    capability_name: None,
                    description: "Read docs".into(),
                },
                crate::events::ToolDefinitionSummary {
                    name: "activate_skill".into(),
                    display_name: Some("Activate Skill".into()),
                    category: Some("Skills".into()),
                    capability_id: Some("skills".into()),
                    capability_name: Some("Agent Skills".into()),
                    description: "Activate".into(),
                },
            ],
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
        assert!(report.contributions.iter().any(|contribution| {
            contribution.section_key == "mcp" && contribution.source_id == "mcp:docs"
        }));
        assert!(report.contributions.iter().any(|contribution| {
            contribution.section_key == "skills"
                && contribution.source_id == "skills"
                && contribution.label == "Agent Skills"
        }));
    }

    #[test]
    fn generation_report_attributes_skill_activation_results() {
        let data = LlmGenerationData::success(
            vec![
                crate::Message::assistant_with_tools(
                    "",
                    vec![crate::ToolCall {
                        id: "call_skill".into(),
                        name: "activate_skill".into(),
                        arguments: json!({"name": "pdf-tool"}),
                    }],
                ),
                crate::Message::tool_result(
                    "call_skill",
                    Some(json!({
                        "skill": "pdf-tool",
                        "instructions": "<skill name=\"pdf-tool\">Use the PDF flow.</skill>",
                    })),
                    None,
                ),
            ],
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
        assert!(report.contributions.iter().any(|contribution| {
            contribution.section_key == "skills"
                && contribution.source_id == "skill:pdf-tool"
                && contribution.label == "/pdf-tool"
        }));
    }

    #[test]
    fn generation_report_attributes_subagent_results_by_name() {
        let data = LlmGenerationData::success(
            vec![
                crate::Message::assistant_with_tools(
                    "",
                    vec![crate::ToolCall {
                        id: "call_subagent".into(),
                        name: "spawn_agent".into(),
                        arguments: json!({
                            "name": "Scout",
                            "instructions": "look around",
                            "target": {"type": "subagent"}
                        }),
                    }],
                ),
                crate::Message::tool_result(
                    "call_subagent",
                    Some(json!({
                        "name": "Scout",
                        "status": "completed",
                        "result": "Found the answer.",
                    })),
                    None,
                ),
            ],
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
        assert!(report.contributions.iter().any(|contribution| {
            contribution.section_key == "subagents"
                && contribution.source_id == "subagent:Scout"
                && contribution.label == "Scout"
        }));
    }

    #[test]
    fn empty_system_prompt_does_not_add_section() {
        let mut builder = ContextReportBuilder::default();
        add_system_prompt_breakdown(&mut builder, "");
        assert!(builder.sections().is_empty());
    }
}
