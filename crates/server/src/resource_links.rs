//! Resource link building: absolute `self_url` (API) and `view_url` (UI) for
//! recognizable entity objects in JSON output.
//!
//! Decision: the URL builder and the protocol-agnostic link decoration live at
//! the crate root, not in the HTTP layer, because the command catalog (MCP,
//! the Platform capability, the worker shell) decorates its output too. The
//! HTTP-only parts (typed `wrap`, pagination links, response middleware) stay
//! in `api::common`, which re-exports `UrlBuilder`.

use serde_json::Value;

/// Builds absolute `url` (API) and `view_url` (UI) for resources.
#[derive(Debug, Clone)]
pub struct UrlBuilder {
    pub(crate) api_base: String,
    pub(crate) ui_base: String,
}

impl UrlBuilder {
    pub fn new(api_base: &str, ui_base: &str) -> Self {
        Self {
            api_base: api_base.trim_end_matches('/').to_string(),
            ui_base: ui_base.trim_end_matches('/').to_string(),
        }
    }

    /// Create from an `AuthConfig`.
    pub fn from_auth_config(config: &crate::auth::config::AuthConfig) -> Self {
        Self::new(&config.base_url, &config.frontend_url)
    }

    /// Add resource links to any recognizable entity objects in a JSON value.
    ///
    /// This is the protocol-agnostic link aspect used for command/MCP output
    /// and final API responses. It is additive only: existing link fields win.
    pub fn decorate_value_links(&self, value: &mut Value) -> bool {
        decorate_value_links(value, self)
    }
}

fn decorate_value_links(value: &mut Value, builder: &UrlBuilder) -> bool {
    match value {
        Value::Object(map) => {
            let mut changed = false;
            if !map.contains_key("ui_link")
                && let Some(view_url) = map.get("view_url").and_then(Value::as_str)
            {
                map.insert("ui_link".to_string(), Value::String(view_url.to_string()));
                changed = true;
            }
            if let Some(route) = route_for_object(map) {
                if !map.contains_key("self_url")
                    && let Some(api_path) = route.api_path
                {
                    map.insert(
                        "self_url".to_string(),
                        Value::String(format!("{}/{}", builder.api_base, api_path)),
                    );
                    changed = true;
                }
                if !map.contains_key("view_url") {
                    let view_url = format!("{}/{}", builder.ui_base, route.ui_path);
                    map.insert("view_url".to_string(), Value::String(view_url.clone()));
                    changed = true;
                    if !map.contains_key("ui_link") {
                        map.insert("ui_link".to_string(), Value::String(view_url));
                        changed = true;
                    }
                } else if !map.contains_key("ui_link")
                    && let Some(view_url) = map.get("view_url").and_then(Value::as_str)
                {
                    map.insert("ui_link".to_string(), Value::String(view_url.to_string()));
                    changed = true;
                }
            }

            for child in map.values_mut() {
                changed |= decorate_value_links(child, builder);
            }
            changed
        }
        Value::Array(items) => {
            let mut changed = false;
            for item in items {
                changed |= decorate_value_links(item, builder);
            }
            changed
        }
        _ => false,
    }
}

struct LinkRoute {
    api_path: Option<String>,
    ui_path: String,
}

struct ResourceId {
    value: String,
    include_self_url: bool,
}

fn route_for_object(map: &serde_json::Map<String, Value>) -> Option<LinkRoute> {
    let id = own_resource_id(map)?;
    let mut route = route_for_id(&id.value, map)?;
    if !id.include_self_url {
        route.api_path = None;
    }
    Some(route)
}

fn own_resource_id(map: &serde_json::Map<String, Value>) -> Option<ResourceId> {
    for key in ["id", "public_id"] {
        if let Some(id) = map.get(key).and_then(Value::as_str)
            && route_for_id(id, map).is_some()
        {
            return Some(ResourceId {
                value: id.to_string(),
                include_self_url: true,
            });
        }
    }

    for key in [
        "session_id",
        "agent_id",
        "harness_id",
        "app_id",
        "identity_id",
        "mcp_server_id",
        "skill_id",
        "provider_id",
        "model_id",
        "eval_id",
        "budget_id",
    ] {
        if let Some(id) = map.get(key).and_then(Value::as_str)
            && route_for_id(id, map).is_some()
        {
            return Some(ResourceId {
                value: id.to_string(),
                include_self_url: false,
            });
        }
    }

    None
}

fn route_for_id(id: &str, map: &serde_json::Map<String, Value>) -> Option<LinkRoute> {
    let Some((prefix, _)) = id.split_once('_') else {
        return looks_like_capability(map).then(|| LinkRoute {
            api_path: Some(format!("v1/capabilities/{id}")),
            ui_path: format!("capabilities/{id}"),
        });
    };
    let route = match prefix {
        "agent" => ("v1/agents", format!("agents/{id}")),
        "harness" => ("v1/harnesses", format!("harnesses/{id}")),
        "session" => ("v1/sessions", format!("sessions/{id}/chat")),
        "app" => ("v1/apps", format!("apps/{id}")),
        "identity" => ("v1/virtual-users", format!("virtual-users/{id}")),
        "mcp" => ("v1/mcp-servers", "settings/mcp-catalog".to_string()),
        "skill" => ("v1/skills", "skills".to_string()),
        "provider" => ("v1/providers", format!("models/providers/{id}")),
        "model" => ("v1/models", "models".to_string()),
        "eval" => ("v1/evals", format!("evals/{id}")),
        "bdgt" => ("v1/budgets", "budgets".to_string()),
        "sched" => {
            let session_id = map.get("session_id").and_then(Value::as_str)?;
            return Some(LinkRoute {
                api_path: Some(format!("v1/sessions/{session_id}/schedules/{id}")),
                ui_path: format!("sessions/{session_id}/schedules"),
            });
        }
        _ if looks_like_capability(map) => {
            return Some(LinkRoute {
                api_path: Some(format!("v1/capabilities/{id}")),
                ui_path: format!("capabilities/{id}"),
            });
        }
        _ => return None,
    };
    Some(LinkRoute {
        api_path: Some(format!("{}/{id}", route.0)),
        ui_path: route.1,
    })
}

fn looks_like_capability(map: &serde_json::Map<String, Value>) -> bool {
    map.contains_key("tool_definitions")
        || map.contains_key("tool_count")
        || map.contains_key("dependencies")
        || map.contains_key("config_schema")
        || map
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|value| matches!(value, "builtin" | "mcp_server" | "skill"))
        || map.contains_key("is_mcp")
        || map.contains_key("is_skill")
        || map.contains_key("is_guardrail")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_server_links_open_the_settings_catalog() {
        let builder = UrlBuilder::new("https://api.example/api", "https://console.example");
        let mut value = serde_json::json!({ "id": "mcp_01h9", "name": "visti" });
        assert!(builder.decorate_value_links(&mut value));
        assert_eq!(
            value["self_url"],
            "https://api.example/api/v1/mcp-servers/mcp_01h9"
        );
        assert_eq!(
            value["view_url"],
            "https://console.example/settings/mcp-catalog"
        );
    }
}
