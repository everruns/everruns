use std::sync::Arc;

use crate::capabilities::{Capability, CapabilityLocalization, ToolCallHook, ToolDefinitionHook};
use crate::tool_narration::ToolNarrationPhase;
use crate::tool_types::{
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
        _ctx: crate::tool_narration::ToolNarrationContext<'_>,
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
    use crate::tool_types::{BuiltinTool, DeferrablePolicy, ToolPolicy};
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

        let original = serde_json::to_value(&tool).unwrap();
        let transformed = hook.transform(vec![tool]);
        assert_eq!(transformed.len(), 1);
        let mut restored = serde_json::to_value(&transformed[0]).unwrap();
        restored["parameters"]["properties"]
            .as_object_mut()
            .unwrap()
            .remove("human_intent");
        assert_eq!(restored, original);
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
    fn narration_trims_unicode_boundaries_and_execution_preserves_original_call() {
        let hook = HumanIntentCapability.tool_call_hooks().pop().unwrap();
        for (intent, expected) in [
            (json!(null), None),
            (json!(17), None),
            (json!("  "), None),
            (
                json!("  Listing harnesses  "),
                Some("Listing harnesses".to_owned()),
            ),
            (json!("界".repeat(119)), Some("界".repeat(119))),
            (json!("界".repeat(120)), Some("界".repeat(120))),
            (
                json!(format!("  {}  ", "界".repeat(121))),
                Some(format!("{}...", "界".repeat(117))),
            ),
        ] {
            let call = ToolCall {
                id: "call-original".into(),
                name: "manage_harnesses".into(),
                arguments: json!({"operation":"list","nested":{"keep":true},"human_intent":intent}),
            };
            for phase in [
                ToolNarrationPhase::Started,
                ToolNarrationPhase::Waiting,
                ToolNarrationPhase::Completed,
                ToolNarrationPhase::Failed,
            ] {
                assert_eq!(
                    hook.narration(None, &call, phase, Some("uk-UA"), Default::default()),
                    expected
                );
            }
            let execution = hook.transform_for_execution(call);
            assert_eq!(
                serde_json::to_value(execution).unwrap(),
                json!({"id":"call-original","name":"manage_harnesses","arguments":{"operation":"list","nested":{"keep":true}}})
            );
        }
        let call = ToolCall {
            id: "no-intent".into(),
            name: "plain".into(),
            arguments: json!({"operation":"list"}),
        };
        assert_eq!(
            hook.narration(
                None,
                &call,
                ToolNarrationPhase::Started,
                None,
                Default::default()
            ),
            None
        );
        assert_eq!(
            serde_json::to_value(hook.transform_for_execution(call.clone())).unwrap(),
            serde_json::to_value(call).unwrap()
        );
    }
}
