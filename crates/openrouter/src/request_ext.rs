// OpenRouter request decoration
//
// OpenRouter accepts the OpenAI-compatible Open Responses request shape plus a
// handful of vendor extensions. Rather than teach the vendor-neutral core driver
// about OpenRouter, this module implements `OpenResponsesRequestExtension` and
// layers those extra top-level fields onto the serialized request body:
//   - `models` / `route` / `provider` — model-fallback and provider routing
//   - `plugins` — web-search / file-reader activations
//   - `session_id` — OpenRouter session grouping (the Everruns session id)

use everruns_core::OpenResponsesRequestExtension;
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::llm_driver_registry::{
    LlmCallConfig, OpenRouterCapacityStrategy, OpenRouterPluginConfig, OpenRouterRoutingConfig,
};
use serde_json::{Value, json};

/// Layers OpenRouter-specific fields onto an Open Responses request body.
#[derive(Debug, Default, Clone)]
pub struct OpenRouterRequestExtension;

impl OpenResponsesRequestExtension for OpenRouterRequestExtension {
    fn decorate(&self, body: &mut Value, config: &LlmCallConfig) -> Result<()> {
        let Some(obj) = body.as_object_mut() else {
            // The base driver always serializes the request as a JSON object;
            // anything else is not something we can decorate.
            return Ok(());
        };

        // Group related generations under the Everruns session in OpenRouter's
        // dashboard by forwarding the session id as the top-level `session_id`.
        if let Some(session_id) = config.metadata.get("session_id") {
            obj.insert("session_id".to_string(), json!(session_id));
        }

        let Some(routing) = config.openrouter_routing.as_ref() else {
            return Ok(());
        };

        routing
            .validate_for_primary_model(&config.model)
            .map_err(AgentLoopError::llm)?;

        // Apply routing presets, then capacity strategy. Avoid cloning when both
        // are no-ops by resolving to an owned `effective` config only as needed.
        let effective = resolve_effective_routing(routing)?;

        if !effective.models.is_empty() {
            obj.insert("models".to_string(), json!(effective.models));
        }
        if let Some(route) = effective.route {
            obj.insert("route".to_string(), to_value(&route)?);
        }
        if let Some(provider) = effective.provider.as_ref().filter(|p| !p.is_empty()) {
            obj.insert("provider".to_string(), to_value(provider)?);
        }
        if let Some(plugins) = effective
            .plugins
            .as_ref()
            .filter(|p| !p.is_empty())
            .and_then(plugins_to_wire)
        {
            obj.insert("plugins".to_string(), Value::Array(plugins));
        }

        Ok(())
    }
}

/// Apply routing presets, then the capacity strategy, returning the resolved
/// config used to build the wire request.
fn resolve_effective_routing(routing: &OpenRouterRoutingConfig) -> Result<OpenRouterRoutingConfig> {
    let after_presets = if routing.presets.is_empty() {
        routing.clone()
    } else {
        routing.apply_presets().map_err(AgentLoopError::llm)?
    };
    match after_presets.capacity_strategy {
        None | Some(OpenRouterCapacityStrategy::SharedCapacity) => Ok(after_presets),
        _ => after_presets
            .apply_capacity_strategy()
            .map_err(AgentLoopError::llm),
    }
}

fn to_value<T: serde::Serialize>(value: &T) -> Result<Value> {
    serde_json::to_value(value)
        .map_err(|e| AgentLoopError::llm(format!("Failed to serialize OpenRouter field: {}", e)))
}

/// Convert an [`OpenRouterPluginConfig`] into the wire-format `plugins` array.
///
/// Each active plugin becomes a JSON object with an `"id"` field plus any
/// plugin-specific options. Plugins whose struct is `None` are omitted.
/// Returns `None` when no plugins are enabled so the field is skipped entirely.
fn plugins_to_wire(config: &OpenRouterPluginConfig) -> Option<Vec<Value>> {
    let mut items: Vec<Value> = Vec::new();

    if let Some(web) = &config.web {
        let mut obj = serde_json::Map::new();
        obj.insert("id".to_string(), json!("web"));
        if let Some(max_results) = web.max_results {
            obj.insert("max_results".to_string(), json!(max_results));
        }
        if let Some(ref prompt) = web.search_prompt {
            obj.insert("search_prompt".to_string(), json!(prompt));
        }
        items.push(Value::Object(obj));
    }

    if config.file.is_some() {
        items.push(json!({"id": "file"}));
    }

    if items.is_empty() { None } else { Some(items) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::llm_driver_registry::{OpenRouterFilePlugin, OpenRouterWebSearchPlugin};

    #[test]
    fn empty_plugin_config_serializes_to_none() {
        let cfg = OpenRouterPluginConfig::default();
        assert!(plugins_to_wire(&cfg).is_none());
    }

    #[test]
    fn web_plugin_serializes_with_options() {
        let cfg = OpenRouterPluginConfig {
            web: Some(OpenRouterWebSearchPlugin {
                max_results: Some(3),
                search_prompt: Some("find docs".to_string()),
            }),
            ..Default::default()
        };
        let wire = plugins_to_wire(&cfg).expect("plugins present");
        assert_eq!(
            wire,
            vec![json!({
                "id": "web",
                "max_results": 3,
                "search_prompt": "find docs",
            })]
        );
    }

    #[test]
    fn web_plugin_omits_absent_options() {
        let cfg = OpenRouterPluginConfig {
            web: Some(OpenRouterWebSearchPlugin {
                max_results: None,
                search_prompt: None,
            }),
            ..Default::default()
        };
        let wire = plugins_to_wire(&cfg).expect("plugins present");
        assert_eq!(wire, vec![json!({ "id": "web" })]);
    }

    #[test]
    fn file_plugin_serializes_as_id_only() {
        let cfg = OpenRouterPluginConfig {
            file: Some(OpenRouterFilePlugin {}),
            ..Default::default()
        };
        let wire = plugins_to_wire(&cfg).expect("plugins present");
        assert_eq!(wire, vec![json!({ "id": "file" })]);
    }

    #[test]
    fn web_and_file_plugins_serialize_together() {
        let cfg = OpenRouterPluginConfig {
            web: Some(OpenRouterWebSearchPlugin {
                max_results: Some(1),
                search_prompt: None,
            }),
            file: Some(OpenRouterFilePlugin {}),
        };
        let wire = plugins_to_wire(&cfg).expect("plugins present");
        assert_eq!(
            wire,
            vec![
                json!({ "id": "web", "max_results": 1 }),
                json!({ "id": "file" })
            ]
        );
    }
}
