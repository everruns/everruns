// MCP resources capability (spec 2025-11-25): `resources/list`,
// `resources/templates/list` and `resources/read`.
//
// JSON catalogs (`everruns://...`) are read fresh on every call and scoped to
// the resolved org. `ui://everruns/app/...` templates are MCP Apps views
// (`apps`), static per server build. The two URI templates read one agent or
// session through the same policy-gated commands as the REST API.
//
// Decision: the catalog is one page, so `cursor` is accepted and ignored and
// no `nextCursor` is returned. The capability stays `resources: {}`: no
// subscriptions and no list-changed notifications, since the list is static.

use super::{AppState, JsonRpcResponse, ResolvedOrg, apps, link_builder, mcp_ctx, resource_error};
use crate::domains::common::{Command, CommandError, CommandErrorKind};
use serde_json::{Value, json};

/// JSON-RPC code for an unknown resource URI (spec 2025-11-25).
const RESOURCE_NOT_FOUND: i32 = -32002;

const AGENT_URI_PREFIX: &str = "everruns://agents/";
const SESSION_URI_PREFIX: &str = "everruns://sessions/";

/// Static resource catalog — returned by resources/list.
pub(super) fn handle_resources_list(id: Option<Value>) -> JsonRpcResponse {
    // Catalogs are context for the model; MCP Apps views are for the host.
    let for_assistant = json!({ "audience": ["assistant"] });
    let mut resources = json!([
        {
            "uri": "everruns://capabilities",
            "name": "capabilities",
            "title": "Capabilities",
            "description": "Available capabilities (tools, sandboxes, integrations)",
            "mimeType": "application/json",
            "annotations": for_assistant
        },
        {
            "uri": "everruns://harnesses",
            "name": "harnesses",
            "title": "Harnesses",
            "description": "Available harnesses (base environments for sessions)",
            "mimeType": "application/json",
            "annotations": for_assistant
        },
        {
            "uri": "everruns://models",
            "name": "models",
            "title": "LLM Models",
            "description": "Available LLM models and providers",
            "mimeType": "application/json",
            "annotations": for_assistant
        },
        {
            "uri": "everruns://agents",
            "name": "agents",
            "title": "Agents",
            "description": "Agent summaries (id, name, description)",
            "mimeType": "application/json",
            "annotations": for_assistant
        }
    ]);
    if let Some(list) = resources.as_array_mut() {
        list.extend(apps::resource_list_entries());
    }
    JsonRpcResponse::success(id, json!({ "resources": resources }))
}

/// Resource templates — returned by resources/templates/list.
pub(super) fn handle_resource_templates_list(id: Option<Value>) -> JsonRpcResponse {
    JsonRpcResponse::success(
        id,
        json!({ "resourceTemplates": [
            {
                "uriTemplate": "everruns://agents/{agent_id}",
                "name": "agent",
                "title": "Agent",
                "description": "One agent's configuration, by id (agent_...) or name",
                "mimeType": "application/json",
                "annotations": { "audience": ["assistant"] }
            },
            {
                "uriTemplate": "everruns://sessions/{session_id}",
                "name": "session",
                "title": "Session",
                "description": "One session's status, agent, harness and model, by id (session_...)",
                "mimeType": "application/json",
                "annotations": { "audience": ["assistant"] }
            }
        ]}),
    )
}

fn resource_not_found(id: Option<Value>, uri: &str) -> JsonRpcResponse {
    let mut response = JsonRpcResponse::error(id, RESOURCE_NOT_FOUND, "Resource not found");
    if let Some(error) = response.error.as_mut() {
        error.data = Some(json!({ "uri": uri }));
    }
    response
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
        _ => match template_read(uri, org, state).await {
            Some(Ok(text)) => Ok(text),
            Some(Err(TemplateReadError::NotFound)) | None => {
                return resource_not_found(id, uri);
            }
            Some(Err(TemplateReadError::Failed(msg))) => Err(msg),
        },
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

enum TemplateReadError {
    NotFound,
    Failed(String),
}

impl From<CommandError> for TemplateReadError {
    fn from(e: CommandError) -> Self {
        match e.kind {
            CommandErrorKind::NotFound(_) => Self::NotFound,
            _ => Self::Failed(resource_error("resource", e)),
        }
    }
}

/// Read a URI filled from one of the templates. `None` when the URI matches
/// no template.
async fn template_read(
    uri: &str,
    org: &ResolvedOrg,
    state: &AppState,
) -> Option<Result<String, TemplateReadError>> {
    let ctx = mcp_ctx(org, state);
    let mut value = if let Some(agent_id) = template_param(uri, AGENT_URI_PREFIX) {
        let agent = crate::domains::agents::GetAgent {
            id: agent_id.to_string(),
        }
        .run(&ctx)
        .await;
        match agent.map(|a| serde_json::to_value(a)) {
            Ok(Ok(value)) => value,
            Ok(Err(e)) => return Some(Err(TemplateReadError::Failed(e.to_string()))),
            Err(e) => return Some(Err(e.into())),
        }
    } else if let Some(session_id) = template_param(uri, SESSION_URI_PREFIX) {
        let session = crate::domains::sessions::GetSession {
            session_id: session_id.to_string(),
        }
        .run(&ctx)
        .await;
        match session.map(|s| serde_json::to_value(s)) {
            Ok(Ok(value)) => value,
            Ok(Err(e)) => return Some(Err(TemplateReadError::Failed(e.to_string()))),
            Err(e) => return Some(Err(e.into())),
        }
    } else {
        return None;
    };
    link_builder(state).decorate_value_links(&mut value);
    Some(
        serde_json::to_string(&value)
            .map_err(|e| TemplateReadError::Failed(format!("Serialization error: {e}"))),
    )
}

/// The single path segment after `prefix`, or `None` when the URI is not
/// `prefix` followed by one non-empty segment.
fn template_param<'a>(uri: &'a str, prefix: &str) -> Option<&'a str> {
    uri.strip_prefix(prefix)
        .filter(|rest| !rest.is_empty() && !rest.contains(['/', '?', '#']))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_param_takes_exactly_one_segment() {
        assert_eq!(
            template_param("everruns://agents/agent_01", AGENT_URI_PREFIX),
            Some("agent_01")
        );
        assert_eq!(template_param("everruns://agents/", AGENT_URI_PREFIX), None);
        assert_eq!(
            template_param("everruns://agents/a/b", AGENT_URI_PREFIX),
            None
        );
        assert_eq!(
            template_param("everruns://agents/a?x=1", AGENT_URI_PREFIX),
            None
        );
        assert_eq!(template_param("everruns://agents", AGENT_URI_PREFIX), None);
    }

    #[test]
    fn unknown_uri_is_resource_not_found_with_the_uri() {
        let response = resource_not_found(Some(json!(7)), "everruns://nope");
        let error = response.error.expect("error");
        assert_eq!(error.code, -32002);
        assert_eq!(error.data, Some(json!({ "uri": "everruns://nope" })));
    }

    #[test]
    fn catalog_entries_carry_title_and_assistant_audience() {
        let response = handle_resources_list(None);
        let resources = response.result.expect("result")["resources"].clone();
        let agents = resources
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["uri"] == "everruns://agents")
            .unwrap()
            .clone();
        assert_eq!(agents["title"], "Agents");
        assert_eq!(agents["annotations"]["audience"], json!(["assistant"]));
    }

    #[test]
    fn templates_list_agent_and_session() {
        let response = handle_resource_templates_list(None);
        let templates = response.result.expect("result")["resourceTemplates"].clone();
        let uris: Vec<_> = templates
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["uriTemplate"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            uris,
            [
                "everruns://agents/{agent_id}",
                "everruns://sessions/{session_id}"
            ]
        );
    }
}
