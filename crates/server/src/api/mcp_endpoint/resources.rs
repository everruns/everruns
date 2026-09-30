// MCP resources capability: `resources/list` and `resources/read`.
//
// JSON catalogs (`everruns://...`) are read fresh on every call and scoped to
// the resolved org. `ui://everruns/app/...` templates are MCP Apps views
// (`apps`), static per server build.

use super::{AppState, JsonRpcResponse, ResolvedOrg, apps, link_builder, mcp_ctx, resource_error};
use crate::domains::common::Command;
use serde_json::{Value, json};

/// Static resource catalog — returned by resources/list.
pub(super) fn handle_resources_list(id: Option<Value>) -> JsonRpcResponse {
    let mut resources = json!([
        {
            "uri": "everruns://capabilities",
            "name": "Capabilities",
            "description": "Available capabilities (tools, sandboxes, integrations)",
            "mimeType": "application/json"
        },
        {
            "uri": "everruns://harnesses",
            "name": "Harnesses",
            "description": "Available harnesses (base environments for sessions)",
            "mimeType": "application/json"
        },
        {
            "uri": "everruns://models",
            "name": "LLM Models",
            "description": "Available LLM models and providers",
            "mimeType": "application/json"
        },
        {
            "uri": "everruns://agents",
            "name": "Agents",
            "description": "Agent summaries (id, name, description)",
            "mimeType": "application/json"
        }
    ]);
    if let Some(list) = resources.as_array_mut() {
        list.extend(apps::resource_list_entries());
    }
    JsonRpcResponse::success(id, json!({ "resources": resources }))
}

/// Read a resource by URI — fetches fresh data on each call.
pub(super) async fn handle_resources_read(
    id: Option<Value>,
    params: Value,
    org: &ResolvedOrg,
    state: &AppState,
) -> JsonRpcResponse {
    let uri = match params.get("uri").and_then(|v| v.as_str()) {
        Some(uri) => uri,
        None => return JsonRpcResponse::invalid_params(id, "Missing 'uri' in params"),
    };

    if let Some(contents) = apps::read_resource(uri) {
        return JsonRpcResponse::success(id, contents);
    }

    let result = match uri {
        "everruns://capabilities" => read_capabilities(org, state).await,
        "everruns://harnesses" => read_harnesses(org, state).await,
        "everruns://models" => read_models(org, state).await,
        "everruns://agents" => read_agents(org, state).await,
        _ => return JsonRpcResponse::invalid_params(id, format!("Unknown resource URI: {uri}")),
    };

    match result {
        Ok(text) => JsonRpcResponse::success(
            id,
            json!({
                "contents": [{
                    "uri": uri,
                    "mimeType": "application/json",
                    "text": text
                }]
            }),
        ),
        // -32603 = internal error (service failure, not client error)
        Err(msg) => JsonRpcResponse::error(id, -32603, &msg),
    }
}

async fn read_capabilities(org: &ResolvedOrg, state: &AppState) -> Result<String, String> {
    let ctx = mcp_ctx(org, state);
    let capabilities = crate::domains::capabilities::ListCapabilities {
        search: None,
        offset: Some(0),
        limit: Some(200),
        include_retired: false,
    }
    .run(&ctx)
    .await
    .map_err(|e| resource_error("capabilities", e))?;

    let summary: Vec<Value> = capabilities
        .data
        .into_iter()
        .map(|c| {
            json!({
                "id": c.id.as_str(),
                "name": c.name,
                "description": c.description,
                "status": c.status,
            })
        })
        .collect();

    let mut value = Value::Array(summary);
    link_builder(state).decorate_value_links(&mut value);
    serde_json::to_string(&value).map_err(|e| format!("Serialization error: {e}"))
}

async fn read_harnesses(org: &ResolvedOrg, state: &AppState) -> Result<String, String> {
    let ctx = mcp_ctx(org, state);
    let harnesses = crate::domains::harnesses::ListHarnesses {
        search: None,
        include_archived: false,
    }
    .run(&ctx)
    .await
    .map_err(|e| resource_error("harnesses", e))?;

    let summary: Vec<Value> = harnesses
        .into_iter()
        .map(|h| {
            json!({
                "id": h.id.to_string(),
                "name": h.name,
                "description": h.description,
                "status": h.status,
            })
        })
        .collect();

    let mut value = Value::Array(summary);
    link_builder(state).decorate_value_links(&mut value);
    serde_json::to_string(&value).map_err(|e| format!("Serialization error: {e}"))
}

async fn read_models(org: &ResolvedOrg, state: &AppState) -> Result<String, String> {
    let ctx = mcp_ctx(org, state);
    let providers = crate::domains::providers::ListProviders {}
        .run(&ctx)
        .await
        .map_err(|e| resource_error("providers", e))?;

    let summary: Vec<Value> = providers
        .into_iter()
        .map(|p| {
            json!({
                "id": p.id.to_string(),
                "name": p.name,
                "status": p.status,
            })
        })
        .collect();

    let mut value = Value::Array(summary);
    link_builder(state).decorate_value_links(&mut value);
    serde_json::to_string(&value).map_err(|e| format!("Serialization error: {e}"))
}

async fn read_agents(org: &ResolvedOrg, state: &AppState) -> Result<String, String> {
    let ctx = mcp_ctx(org, state);
    let agents = crate::domains::agents::ListAgents {
        search: None,
        include_archived: false,
        offset: Some(0),
        limit: Some(100),
    }
    .run(&ctx)
    .await
    .map_err(|e| resource_error("agents", e))?;

    let summary: Vec<Value> = agents
        .data
        .into_iter()
        .map(|a| {
            json!({
                "id": a.public_id,
                "name": a.name,
                "description": a.description,
            })
        })
        .collect();

    let mut value = Value::Array(summary);
    link_builder(state).decorate_value_links(&mut value);
    serde_json::to_string(&value).map_err(|e| format!("Serialization error: {e}"))
}
