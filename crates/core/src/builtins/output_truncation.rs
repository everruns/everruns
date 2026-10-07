// Output truncation capability
//
// Selects what a turn does when a model response loses tool calls because it
// hit the output-token limit (finish reason `length`) or emitted arguments
// that were not valid JSON. Drivers never run such a call and never substitute
// `{}` for its arguments; this only picks the turn's next step:
//
// - `continue` (default, also without the capability): tell the model the
//   calls did not run and run another generation, at most `max_retries`
//   consecutive times, then fail the turn.
// - `fail`: end the turn with an error.
// - `off`: the previous turn behaviour; the lost calls are simply not run.
//
// The engine reads the config itself (`crate::output_truncation`), so the
// capability adds no tools or prompt text. It exists so the setting has a home
// on the agent and harness config surface, with validation and a schema.

use super::{Capability, CapabilityLocalization, SystemPromptContext};
use crate::output_truncation::{
    DEFAULT_MAX_RETRIES, MAX_RETRIES_LIMIT, OUTPUT_TRUNCATION_CAPABILITY_ID, OutputTruncationConfig,
};
use async_trait::async_trait;

/// Output truncation capability: configures the engine's truncation gate.
pub struct OutputTruncationCapability;

#[async_trait]
impl Capability for OutputTruncationCapability {
    fn id(&self) -> &str {
        OUTPUT_TRUNCATION_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Output Truncation"
    }

    fn description(&self) -> &str {
        "Chooses what a turn does when a response hits the output token limit in the middle \
         of a tool call: continue (tell the model and retry, the default), fail (end the turn \
         with an error), or off (just skip the cut-off call). A cut-off call never runs."
    }

    fn category(&self) -> Option<&str> {
        Some("Safety")
    }

    fn config_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "policy": {
                    "type": "string",
                    "title": "Policy",
                    "description": "continue: tell the model its cut-off calls did not run and retry (default); fail: end the turn with an error; off: skip the cut-off calls.",
                    "enum": ["continue", "fail", "off"],
                    "default": "continue"
                },
                "max_retries": {
                    "type": "integer",
                    "title": "Max retries",
                    "description": "Consecutive cut-off responses `continue` retries before the turn fails.",
                    "minimum": 0,
                    "maximum": MAX_RETRIES_LIMIT,
                    "default": DEFAULT_MAX_RETRIES
                }
            }
        }))
    }

    fn validate_config(&self, config: &serde_json::Value) -> Result<(), String> {
        OutputTruncationConfig::from_capability_config(config).map(|_| ())
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![
            CapabilityLocalization {
                locale: "en",
                name: None,
                description: None,
                config_description: Some(
                    "Chooses whether a turn retries, fails, or carries on when a response is cut off mid tool call.",
                ),
                config_overlay: None,
            },
            CapabilityLocalization {
                locale: "uk",
                name: Some("Обрізання виводу"),
                description: Some(
                    "Визначає, що робить хід, коли відповідь досягає ліміту вихідних токенів посеред виклику інструмента: continue (повідомити модель і повторити, типово), fail (завершити хід з помилкою) або off (просто пропустити обрізаний виклик). Обрізаний виклик ніколи не виконується.",
                ),
                config_description: Some(
                    "Обирає, чи хід повторює спробу, завершується з помилкою, чи продовжується, коли відповідь обрізано посеред виклику інструмента.",
                ),
                config_overlay: Some(serde_json::json!({
                    "properties": {
                        "policy": {
                            "title": "Політика",
                            "description": "continue: повідомити модель, що обрізані виклики не виконано, і повторити (типово); fail: завершити хід з помилкою; off: пропустити обрізані виклики."
                        },
                        "max_retries": {
                            "title": "Максимум повторів",
                            "description": "Скільки обрізаних відповідей поспіль continue повторює, перш ніж хід завершиться з помилкою."
                        }
                    }
                })),
            },
        ]
    }

    async fn system_prompt_contribution(&self, _ctx: &SystemPromptContext) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validation_matches_the_engine_parser() {
        let cap = OutputTruncationCapability;
        for (config, valid) in [
            (json!(null), true),
            (json!({}), true),
            (json!({"policy": "continue", "max_retries": 3}), true),
            (json!({"policy": "fail"}), true),
            (json!({"policy": "off"}), true),
            (json!({"policy": "retry"}), false),
            (json!({"max_retries": 11}), false),
            (json!({"extra": true}), false),
            (json!([]), false),
        ] {
            assert_eq!(cap.validate_config(&config).is_ok(), valid, "{config}");
        }
        assert!(cap.tool_definitions().is_empty());
    }

    #[test]
    fn localizations_resolve_uk() {
        assert_eq!(
            OutputTruncationCapability.localized_name(Some("uk-UA")),
            "Обрізання виводу"
        );
    }
}
