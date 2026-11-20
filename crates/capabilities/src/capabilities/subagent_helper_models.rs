//! Helper models for `spawn_agent`'s subagent target: the `model` enum the
//! spawning model sees, and how one spawn's key is chosen.

use super::subagents::SpawnSubagentAsAgentTool;
use everruns_core::ToolContext;
use everruns_core::helper_models::{
    HelperModelPick, HelperModelPickedBy, HelperModelSettings, HelperModelTarget, SAME_HELPER_KEY,
    SpawnHelperModel,
};
use everruns_core::tools::ToolExecutionResult;
use serde_json::{Value, json};

impl SpawnSubagentAsAgentTool {
    /// The `model` property: an enum of the helper keys with when to use each.
    /// None when the agent cannot switch models.
    pub(crate) fn model_property(&self) -> Option<Value> {
        if !self.helpers.can_switch() {
            return None;
        }
        let keys = self
            .helpers
            .models
            .iter()
            .map(|model| format!("{}: {}", model.key, model.when))
            .collect::<Vec<_>>()
            .join(" ");
        let omitted = match self.helpers.pick {
            HelperModelPick::Agent => "Omit to run on your own model (`same`).",
            HelperModelPick::Auto => "Omit to let the platform pick a key from the instructions.",
        };
        Some(json!({
            "type": "string",
            "enum": self.helpers.keys(),
            "description": format!("Subagent-only helper model key. {keys} {omitted}")
        }))
    }
}

/// Fixed blueprints always run on their own model, so a helper key is refused
/// before any model call is spent picking one.
pub(crate) fn fixed_blueprint_refuses_model(
    arguments: &Value,
    blueprint: Option<&str>,
    context: &ToolContext,
) -> Option<ToolExecutionResult> {
    let bp_id = blueprint?;
    let named = arguments
        .get("model")
        .and_then(Value::as_str)
        .is_some_and(|key| key != SAME_HELPER_KEY);
    let fixed = context
        .capability_registry
        .as_ref()
        .and_then(|registry| registry.blueprint_with_capability(bp_id))
        .is_some_and(|(_, blueprint)| {
            matches!(
                blueprint.model,
                everruns_core::capabilities::BlueprintModel::Fixed(_)
            )
        });
    (named && fixed).then(|| {
        ToolExecutionResult::tool_error(format!(
            "Blueprint \"{bp_id}\" always runs on its own model; omit `model`."
        ))
    })
}

/// Pick the helper model for one spawn: the key the spawning model named,
/// else the decision model's pick when the agent is on Auto, else `same`.
pub(crate) async fn choose_helper_model(
    arguments: &Value,
    context: &ToolContext,
    helpers: &HelperModelSettings,
    name: &str,
    instructions: &str,
) -> Result<SpawnHelperModel, ToolExecutionResult> {
    let same = || SpawnHelperModel {
        key: SAME_HELPER_KEY.to_string(),
        target: HelperModelTarget::Same,
        picked_by: HelperModelPickedBy::Default,
        reason: None,
    };
    let named = arguments
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|key| !key.is_empty());
    if let Some(key) = named {
        let Some(model) = helpers.get(key) else {
            if key == SAME_HELPER_KEY {
                return Ok(same());
            }
            return Err(ToolExecutionResult::tool_error(format!(
                "Unknown helper model \"{key}\". This agent's keys: {}.",
                helpers.keys().join(", ")
            )));
        };
        return Ok(SpawnHelperModel {
            key: model.key.clone(),
            target: model.target.clone(),
            picked_by: HelperModelPickedBy::Agent,
            reason: None,
        });
    }
    if helpers.pick != HelperModelPick::Auto || !helpers.can_switch() || helpers.models.len() < 2 {
        return Ok(same());
    }
    // Auto: one small decision call reads the brief and picks a key. A
    // missing or failing decision service keeps the spawn on `same` rather
    // than refusing it.
    let Some(decisions) = context.decisions.as_ref().filter(|d| d.is_configured()) else {
        return Ok(same());
    };
    let request = everruns_core::decisions::DecisionRequest::new(json!({
        "subagent": name,
        "instructions": instructions.chars().take(4000).collect::<String>(),
    }))
    .ask(
        "helper_model",
        everruns_core::decisions::DecisionQuestion::Choice {
            instructions:
                "Which model should this subagent run on? Pick the cheapest one that fits the work."
                    .to_string(),
            options: helpers
                .models
                .iter()
                .map(|model| (model.key.clone(), Some(model.when.clone())))
                .collect(),
        },
    )
    .with_metadata("purpose", "helper_model_pick");
    let picked = match decisions.evaluate(request).await {
        Ok(outcome) => match outcome.get("helper_model") {
            Some(everruns_core::decisions::DecisionAnswer::Choice {
                selected,
                confidence,
                ..
            }) => helpers
                .get(selected)
                .cloned()
                .map(|model| (model, *confidence)),
            _ => None,
        },
        Err(error) => {
            tracing::warn!(error = %error, "helper model auto pick failed; using same");
            None
        }
    };
    Ok(
        picked.map_or_else(same, |(model, confidence)| SpawnHelperModel {
            reason: Some(format!(
                "{} ({:.0}% sure)",
                model.when.trim_end_matches('.'),
                confidence * 100.0
            )),
            key: model.key,
            target: model.target,
            picked_by: HelperModelPickedBy::Auto,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::typed_id::SessionId;
    use everruns_core::Tool;
    use everruns_core::helper_models::{HelperModel, HelperModelSettings};

    fn only_same() -> HelperModelSettings {
        HelperModelSettings {
            models: vec![HelperModel {
                key: SAME_HELPER_KEY.into(),
                when: "Most work.".into(),
                target: HelperModelTarget::Same,
            }],
            pick: HelperModelPick::Agent,
        }
    }

    #[test]
    fn schema_lists_helper_keys_with_when_to_use() {
        let tool = SpawnSubagentAsAgentTool::default();
        let schema = tool.parameters_schema();
        let model = &schema["properties"]["model"];
        let keys: Vec<&str> = model["enum"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(keys, ["same", "fast", "coding", "deep", "second_opinion"]);
        let description = model["description"].as_str().unwrap();
        assert!(description.contains("second_opinion:"), "{description}");
        assert!(description.contains("Omit to run on your own model"));
    }

    #[test]
    fn agent_with_only_same_gets_no_model_field() {
        let tool = SpawnSubagentAsAgentTool {
            helpers: only_same(),
        };
        assert!(
            tool.parameters_schema()["properties"]
                .get("model")
                .is_none()
        );
    }

    #[tokio::test]
    async fn named_key_is_picked_by_the_agent() {
        let context = ToolContext::new(SessionId::new());
        let picked = choose_helper_model(
            &json!({"model": "second_opinion"}),
            &context,
            &HelperModelSettings::default(),
            "Verifier",
            "Check the fix",
        )
        .await
        .unwrap();
        assert_eq!(picked.key, "second_opinion");
        assert_eq!(picked.picked_by, HelperModelPickedBy::Agent);
        assert!(matches!(
            picked.target,
            HelperModelTarget::OtherProvider { .. }
        ));
    }

    #[tokio::test]
    async fn omitted_key_runs_on_same() {
        let context = ToolContext::new(SessionId::new());
        let picked = choose_helper_model(
            &json!({}),
            &context,
            &HelperModelSettings::default(),
            "Runner",
            "Do it",
        )
        .await
        .unwrap();
        assert_eq!(picked.key, SAME_HELPER_KEY);
        assert_eq!(picked.picked_by, HelperModelPickedBy::Default);
    }

    #[tokio::test]
    async fn auto_without_decision_service_falls_back_to_same() {
        let context = ToolContext::new(SessionId::new());
        let helpers = HelperModelSettings {
            pick: HelperModelPick::Auto,
            ..HelperModelSettings::default()
        };
        let picked = choose_helper_model(&json!({}), &context, &helpers, "Runner", "Do it")
            .await
            .unwrap();
        assert_eq!(picked.key, SAME_HELPER_KEY);
    }

    #[tokio::test]
    async fn unknown_key_lists_the_agent_keys() {
        let context = ToolContext::new(SessionId::new());
        let Err(error) = choose_helper_model(
            &json!({"model": "gpt-6.2-sol"}),
            &context,
            &HelperModelSettings::default(),
            "Runner",
            "Do it",
        )
        .await
        else {
            panic!("raw model names must be refused");
        };
        let ToolExecutionResult::ToolError(text) = error else {
            panic!("expected a tool error");
        };
        assert!(text.contains("Unknown helper model"), "{text}");
        assert!(
            text.contains("same, fast, coding, deep, second_opinion"),
            "{text}"
        );
    }
}
