use std::sync::Arc;

use everruns_core::capabilities::{
    Capability, CapabilityLocalization, ToolCallHook, ToolDefinitionHook,
};
use everruns_core::tool_narration::ToolNarrationPhase;
use everruns_core::tool_types::{
    ToolCall, ToolDefinition, add_human_intent_to_tool_definitions, human_intent,
};

pub const HUMAN_INTENT_CAPABILITY_ID: &str = "human_intent";

pub struct HumanIntentCapability;

impl Capability for HumanIntentCapability {
    fn id(&self) -> &'static str {
        HUMAN_INTENT_CAPABILITY_ID
    }

    fn name(&self) -> &'static str {
        "Human Intent"
    }

    fn description(&self) -> &'static str {
        "Adds model-authored human_intent narration to every active tool call for UI rendering."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Людський намір",
            "Додає до кожного активного виклику інструмента написаний моделлю опис наміру human_intent для відображення в інтерфейсі.",
        )]
    }

    fn category(&self) -> Option<&'static str> {
        Some("Core")
    }

    fn tool_definition_hooks(&self) -> Vec<Arc<dyn ToolDefinitionHook>> {
        vec![Arc::new(HumanIntentToolDefinitionHook)]
    }

    fn tool_call_hooks(&self) -> Vec<Arc<dyn ToolCallHook>> {
        vec![Arc::new(HumanIntentToolCallHook)]
    }
}

struct HumanIntentToolDefinitionHook;

impl ToolDefinitionHook for HumanIntentToolDefinitionHook {
    fn transform(&self, tools: Vec<ToolDefinition>) -> Vec<ToolDefinition> {
        add_human_intent_to_tool_definitions(&tools)
    }
}

struct HumanIntentToolCallHook;

impl ToolCallHook for HumanIntentToolCallHook {
    fn narration(
        &self,
        _tool_def: Option<&ToolDefinition>,
        tool_call: &ToolCall,
        _phase: ToolNarrationPhase,
        _locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        human_intent(&tool_call.arguments).map(truncate_intent)
    }

    fn transform_for_execution(&self, mut tool_call: ToolCall) -> ToolCall {
        tool_call.arguments = tool_call.execution_arguments();
        tool_call
    }
}

fn truncate_intent(intent: &str) -> String {
    const MAX_LEN: usize = 120;
    const ELLIPSIS: &str = "...";
    let clean = intent.trim();
    if clean.chars().count() <= MAX_LEN {
        return clean.to_string();
    }

    let truncated: String = clean
        .chars()
        .take(MAX_LEN - ELLIPSIS.chars().count())
        .collect();
    format!("{truncated}{ELLIPSIS}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::tool_types::{BuiltinTool, DeferrablePolicy, ToolPolicy};
    use serde_json::json;

    #[test]
    fn human_intent_capability_adds_optional_schema_argument() {
        let capability = HumanIntentCapability;
        let hook = capability.tool_definition_hooks().pop().unwrap();
        let tool = ToolDefinition::Builtin(BuiltinTool {
            name: "manage_harnesses".to_string(),
            display_name: Some("Manage Harnesses".to_string()),
            description: "Manage harnesses".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "operation": { "type": "string" }
                },
                "required": ["operation"],
                "additionalProperties": false
            }),
            policy: ToolPolicy::Auto,
            category: None,
            deferrable: DeferrablePolicy::default(),
            hints: Default::default(),
            full_parameters: None,
        });

        let transformed = hook.transform(vec![tool]);
        let params = transformed[0].parameters();

        assert_eq!(params["properties"]["human_intent"]["type"], "string");
        assert_eq!(params["properties"]["human_intent"]["maxLength"], 120);
        assert!(
            !params["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item.as_str() == Some("human_intent"))
        );
        assert_eq!(params["additionalProperties"], false);
    }

    #[test]
    fn human_intent_tool_call_hook_reads_and_strips_argument() {
        let capability = HumanIntentCapability;
        let hook = capability.tool_call_hooks().pop().unwrap();
        let tool_call = ToolCall {
            id: "call_1".to_string(),
            name: "manage_harnesses".to_string(),
            arguments: json!({
                "operation": "list",
                "human_intent": "Listing all harnesses"
            }),
        };

        assert_eq!(
            hook.narration(
                None,
                &tool_call,
                ToolNarrationPhase::Started,
                None,
                Default::default()
            ),
            Some("Listing all harnesses".to_string())
        );

        let execution_call = hook.transform_for_execution(tool_call);
        assert_eq!(execution_call.arguments, json!({ "operation": "list" }));
    }

    #[test]
    fn truncate_intent_stays_within_cap() {
        let long_intent = "x".repeat(130);
        let truncated = truncate_intent(&long_intent);

        assert_eq!(truncated.chars().count(), 120);
        assert!(truncated.ends_with("..."));
    }
}
