//! Opt-in host-managed titles, independent of the agent's tool loop.
use super::Capability;
use everruns_contracts::typed_id::ModelId;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const AUTOMATIC_SESSION_TITLES_CAPABILITY_ID: &str = "automatic_session_titles";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TitleModelDefault {
    Session,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TitleModelSelection {
    Default(TitleModelDefault),
    Model(ModelId),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AutomaticSessionTitlesConfig {
    pub model_id: Option<TitleModelSelection>,
    pub retitle: bool,
}

impl Default for AutomaticSessionTitlesConfig {
    fn default() -> Self {
        Self {
            model_id: None,
            retitle: true,
        }
    }
}

pub struct AutomaticSessionTitlesCapability;

impl Capability for AutomaticSessionTitlesCapability {
    fn id(&self) -> &str {
        AUTOMATIC_SESSION_TITLES_CAPABILITY_ID
    }
    fn name(&self) -> &str {
        "Automatic session titles"
    }
    fn description(&self) -> &str {
        "Name sessions automatically and update automatic titles when the main topic changes. Explicit titles always take priority."
    }
    fn icon(&self) -> Option<&str> {
        Some("text-cursor-input")
    }
    fn category(&self) -> Option<&str> {
        Some("Session")
    }
    fn config_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "model_id": {
                    "type": "string",
                    "title": "Title model and provider",
                    "description": "Defaults to Small model, then the session model. Choose a model to pin its provider."
                },
                "retitle": {
                    "type": "boolean", "default": true,
                    "title": "Update titles when the topic changes",
                    "description": "Explicit titles always take priority."
                }
            }
        }))
    }
    fn config_ui_schema(&self) -> Option<Value> {
        Some(json!({"model_id": {"ui:widget": "SessionTitleModel"}}))
    }
    fn validate_config(&self, config: &Value) -> Result<(), String> {
        if config.is_null() {
            return Ok(());
        }
        serde_json::from_value::<AutomaticSessionTitlesConfig>(config.clone())
            .map(|_| ())
            .map_err(|error| format!("Invalid automatic titles config: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_selection_without_granting_tools() {
        let cap = AutomaticSessionTitlesCapability;
        for config in [
            json!({}),
            json!({"model_id": "session"}),
            json!({"model_id": ModelId::new(), "retitle": false}),
            Value::Null,
        ] {
            assert!(cap.validate_config(&config).is_ok());
        }
        for config in [
            json!({"model_id": "cheapest"}),
            json!({"model_id": 1}),
            json!({"retitle": "yes"}),
            json!({"extra": true}),
        ] {
            assert!(cap.validate_config(&config).is_err());
        }
        assert!(cap.tools().is_empty());
        assert!(cap.dependencies().is_empty());
    }
}
