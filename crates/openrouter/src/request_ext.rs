// OpenRouter request decoration
//
// OpenRouter accepts the OpenAI-compatible Open Responses request shape plus a
// handful of vendor extensions. Rather than teach the vendor-neutral core driver
// about OpenRouter, this module implements `OpenResponsesRequestExtension` and
// layers those extra top-level fields onto the serialized request body:
//   - `models` / `route` / `provider` — model-fallback and provider routing
//   - `plugins` — web-search / file-reader activations
//   - `session_id` — OpenRouter session grouping (the Everruns session id)
//   - `reasoning.exclude` — keep provider reasoning private by default
//   - `HTTP-Referer` / `X-Title` — app attribution headers

use std::borrow::Cow;

use everruns_core::OpenResponsesRequestExtension;
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::llm_driver_registry::{
    LlmCallConfig, OPENROUTER_HTTP_REFERER_METADATA_KEY, OPENROUTER_X_TITLE_METADATA_KEY,
    OpenRouterCapacityStrategy, OpenRouterPluginConfig, OpenRouterRoutingConfig,
};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Value, json};

const HTTP_REFERER_HEADER: HeaderName = HeaderName::from_static("http-referer");
const X_TITLE_HEADER: HeaderName = HeaderName::from_static("x-title");

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
        remove_attribution_metadata(obj);

        apply_private_reasoning_policy(obj, config);

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

    fn decorate_headers(&self, headers: &mut HeaderMap, config: &LlmCallConfig) -> Result<()> {
        insert_metadata_header(
            headers,
            HTTP_REFERER_HEADER,
            config.metadata.get(OPENROUTER_HTTP_REFERER_METADATA_KEY),
        )?;
        insert_metadata_header(
            headers,
            X_TITLE_HEADER,
            config.metadata.get(OPENROUTER_X_TITLE_METADATA_KEY),
        )?;

        Ok(())
    }
}

fn remove_attribution_metadata(obj: &mut serde_json::Map<String, Value>) {
    let Some(Value::Object(metadata)) = obj.get_mut("metadata") else {
        return;
    };

    metadata.remove(OPENROUTER_HTTP_REFERER_METADATA_KEY);
    metadata.remove(OPENROUTER_X_TITLE_METADATA_KEY);
    if metadata.is_empty() {
        obj.remove("metadata");
    }
}

fn insert_metadata_header(
    headers: &mut HeaderMap,
    name: HeaderName,
    value: Option<&String>,
) -> Result<()> {
    let Some(value) = value
        .map(String::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
    else {
        return Ok(());
    };

    let header_value = HeaderValue::from_str(value).map_err(|e| {
        AgentLoopError::llm(format!(
            "Invalid OpenRouter attribution header '{}': {}",
            name, e
        ))
    })?;
    headers.insert(name, header_value);
    Ok(())
}

fn apply_private_reasoning_policy(
    obj: &mut serde_json::Map<String, Value>,
    config: &LlmCallConfig,
) {
    let mut reasoning = match obj.remove("reasoning") {
        Some(Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    };

    // OpenRouter may return reasoning tokens even when no effort is requested.
    // Keep that reasoning internal unless a future explicit visible-reasoning
    // option asks for summaries.
    reasoning.remove("summary");
    reasoning.insert("exclude".to_string(), Value::Bool(true));

    if let Some(effort) = config
        .reasoning_effort
        .as_deref()
        .map(str::trim)
        .filter(|effort| !effort.is_empty())
    {
        reasoning.insert("effort".to_string(), Value::String(effort.to_string()));
    }

    obj.insert("reasoning".to_string(), Value::Object(reasoning));
}

/// Apply routing presets, then the capacity strategy, returning the resolved
/// config used to build the wire request. Borrows the original when both steps
/// are no-ops (the common case) and only allocates when a step changes routing.
fn resolve_effective_routing(
    routing: &OpenRouterRoutingConfig,
) -> Result<Cow<'_, OpenRouterRoutingConfig>> {
    let after_presets: Cow<'_, OpenRouterRoutingConfig> = if routing.presets.is_empty() {
        Cow::Borrowed(routing)
    } else {
        Cow::Owned(routing.apply_presets().map_err(AgentLoopError::llm)?)
    };
    match after_presets.capacity_strategy {
        None | Some(OpenRouterCapacityStrategy::SharedCapacity) => Ok(after_presets),
        _ => Ok(Cow::Owned(
            after_presets
                .apply_capacity_strategy()
                .map_err(AgentLoopError::llm)?,
        )),
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
