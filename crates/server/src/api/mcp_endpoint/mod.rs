// MCP Endpoint — exposes Everruns as an MCP server (Streamable HTTP transport)
//
// Design decisions:
// - JSON-RPC 2.0 over POST /mcp (Streamable HTTP, per MCP spec)
// - Tier 1 tools: agent_run, session_send_message, session_get_status
//   → Direct service calls, first-class support for the agent conversation loop
// - Tier 2 tools: discover, execute
//   → Backed by bashkit ScriptedTool with all API operations as builtins
//   → discover: uses ScriptedTool's built-in `discover` command
//   → execute: runs bash scripts through ScriptedTool (all API ops available as commands)
// - Tier 0 tools: me, list_organizations, switch_organization
//   → Identity & org context tools for multi-org OAuth flows
//   → MCP clients can't set cookies, so org selection is via explicit tool calls
// - Auth: same as rest of API (API key or session cookie via ResolvedOrg)
// - No MCP session state — stateless request/response per JSON-RPC call
// - Multi-org: all tools accept optional `organization_id` to override the default org

mod handlers;
mod tool_registry;

use crate::auth::middleware::AuthUser;
use crate::auth::{AuthState, ResolvedOrg};
use crate::services::{
    BudgetService, CapabilityService, EventService, MessageService, SessionFileService,
    SessionService,
};
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use bashkit::ScriptingToolSet;
use everruns_core::session_sqldb::SessionSqlDbStore;
use everruns_core::{Caller, OrgRole, PlatformDefinition, validate_org_public_id};
use everruns_durable::WorkflowEventStore;
use everruns_worker::AgentRunner;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

use super::common::impl_auth_state;

mod catalog;
mod positional;

// ============================================================================
// JSON-RPC 2.0 types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcResponse {
    fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Option<Value>, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }

    fn method_not_found(id: Option<Value>) -> Self {
        Self::error(id, -32601, "Method not found")
    }

    fn invalid_params(id: Option<Value>, msg: impl Into<String>) -> Self {
        Self::error(id, -32602, msg)
    }
}

// ============================================================================
// MCP Tool definitions
// ============================================================================

const MCP_SERVER_NAME: &str = "everruns";
const MCP_SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
const MCP_PROTOCOL_VERSION_FALLBACK: &str = "2025-03-26";
const MCP_PROTOCOL_VERSION_LATEST: &str = "2025-06-18";
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] =
    &[MCP_PROTOCOL_VERSION_LATEST, MCP_PROTOCOL_VERSION_FALLBACK];

const ORG_ID_DESCRIPTION: &str = "Optional organization ID (format: org_{32-hex}). Overrides the default organization for this call. Use list_organizations to see available orgs.";

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct InitializeParams {
    #[serde(default)]
    protocol_version: Option<String>,
}

fn tool_definitions(protocol_version: &str) -> Vec<tool_registry::McpEndpointToolDefinition> {
    tool_registry::tool_definitions(protocol_version, ORG_ID_DESCRIPTION)
}

fn find_tool_definition(
    tool_name: &str,
    protocol_version: &str,
) -> Option<tool_registry::McpEndpointToolDefinition> {
    tool_registry::tool_definition(tool_name, protocol_version, ORG_ID_DESCRIPTION)
}

fn negotiate_protocol_version(requested: Option<&str>) -> &'static str {
    match requested {
        Some(MCP_PROTOCOL_VERSION_FALLBACK) => MCP_PROTOCOL_VERSION_FALLBACK,
        Some(version) if version < MCP_PROTOCOL_VERSION_LATEST => MCP_PROTOCOL_VERSION_FALLBACK,
        Some(_) => MCP_PROTOCOL_VERSION_LATEST,
        None => MCP_PROTOCOL_VERSION_FALLBACK,
    }
}

fn protocol_version_from_headers(headers: &HeaderMap) -> Result<&'static str, String> {
    match headers.get("MCP-Protocol-Version") {
        None => Ok(MCP_PROTOCOL_VERSION_FALLBACK),
        Some(value) => {
            let version = value
                .to_str()
                .map_err(|_| "Invalid MCP-Protocol-Version header".to_string())?;
            match version {
                MCP_PROTOCOL_VERSION_FALLBACK => Ok(MCP_PROTOCOL_VERSION_FALLBACK),
                MCP_PROTOCOL_VERSION_LATEST => Ok(MCP_PROTOCOL_VERSION_LATEST),
                _ => Err(format!(
                    "Unsupported MCP-Protocol-Version: {version}. Supported: {}",
                    SUPPORTED_PROTOCOL_VERSIONS.join(", ")
                )),
            }
        }
    }
}

// ============================================================================
// App State
// ============================================================================

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub session_service: Arc<SessionService>,
    pub message_service: Arc<MessageService>,
    pub event_service: Arc<EventService>,
    pub session_file_service: Arc<SessionFileService>,
    pub capability_service: Arc<CapabilityService>,
    pub budget_service: Arc<BudgetService>,
    pub runner: Arc<dyn AgentRunner>,
    pub auth: AuthState,
    pub encryption: Option<Arc<crate::storage::encryption::EncryptionService>>,
    pub fallback_base_harness_name: Option<String>,
    pub fallback_default_harness_name: Option<String>,
    pub chat_harness_name: Option<String>,
    pub chat_session_title: Option<String>,
    pub sqldb_store: Option<Arc<dyn SessionSqlDbStore>>,
    pub workflow_store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
}

impl AppState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db: Arc<StorageBackend>,
        runner: Arc<dyn AgentRunner>,
        auth: AuthState,
        platform_definition: &PlatformDefinition,
        notifications_enabled: bool,
        event_delivery: crate::event_delivery::EventDelivery,
        encryption: Option<Arc<crate::storage::encryption::EncryptionService>>,
        capability_service: Arc<CapabilityService>,
        sqldb_store: Option<Arc<dyn SessionSqlDbStore>>,
        workflow_store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
    ) -> Self {
        Self {
            session_service: Arc::new(SessionService::with_registry(
                db.clone(),
                platform_definition.capability_registry().clone(),
            )),
            message_service: Arc::new(MessageService::new(
                db.clone(),
                runner.clone(),
                notifications_enabled,
                event_delivery.clone(),
            )),
            event_service: Arc::new(EventService::new(db.clone(), event_delivery)),
            session_file_service: Arc::new(SessionFileService::new(db.clone())),
            capability_service,
            budget_service: Arc::new(BudgetService::new(db.clone())),
            db,
            runner,
            auth,
            encryption,
            fallback_base_harness_name: platform_definition
                .harness_for_role(everruns_core::BuiltInHarnessRole::Base)
                .map(|h| h.name.clone()),
            fallback_default_harness_name: platform_definition
                .harness_for_role(everruns_core::BuiltInHarnessRole::Default)
                .map(|h| h.name.clone()),
            chat_harness_name: platform_definition
                .harness_for_role(everruns_core::BuiltInHarnessRole::Chat)
                .map(|h| h.name.clone()),
            chat_session_title: platform_definition
                .harness_for_role(everruns_core::BuiltInHarnessRole::Chat)
                .map(|h| h.display_name.clone()),
            sqldb_store,
            workflow_store,
        }
    }

    pub fn with_virtual_registry(
        mut self,
        registry: Arc<crate::services::virtual_mount_registry::VirtualMountRegistry>,
    ) -> Self {
        self.session_file_service =
            Arc::new(SessionFileService::new(self.db.clone()).with_virtual_registry(registry));
        self
    }
}

impl_auth_state!(AppState);

// ============================================================================
// Routes
// ============================================================================

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/mcp", post(handle_mcp))
        .with_state(state)
}

// ============================================================================
// Main handler
// ============================================================================

async fn handle_mcp(
    auth_user: AuthUser,
    org: ResolvedOrg,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<JsonRpcRequest>,
) -> Response {
    let protocol_version = if req.method == "initialize" {
        None
    } else {
        match protocol_version_from_headers(&headers) {
            Ok(version) => Some(version),
            Err(msg) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(JsonRpcResponse::error(req.id.clone(), -32600, msg)),
                )
                    .into_response();
            }
        }
    };

    // Per JSON-RPC 2.0, a request without an `id` is a notification and the
    // server MUST NOT reply. MCP lifecycle uses this for
    // `notifications/initialized`, `notifications/cancelled`, etc. We're
    // stateless, so there's nothing to do — acknowledge with 202 Accepted and
    // no body. Short-circuit before the jsonrpc version check because the
    // spec gives no addressable id to return an error against.
    if req.id.is_none() {
        return StatusCode::ACCEPTED.into_response();
    }

    if req.jsonrpc != "2.0" {
        return Json(JsonRpcResponse::error(
            req.id,
            -32600,
            "Invalid Request: jsonrpc must be \"2.0\"",
        ))
        .into_response();
    }

    let response = match req.method.as_str() {
        "initialize" => handle_initialize(req.id, req.params),
        "tools/list" => handle_tools_list(
            req.id,
            protocol_version.unwrap_or(MCP_PROTOCOL_VERSION_FALLBACK),
        ),
        "tools/call" => {
            handle_tools_call(
                req.id.clone(),
                req.params,
                &auth_user,
                &org,
                &state,
                protocol_version.unwrap_or(MCP_PROTOCOL_VERSION_FALLBACK),
            )
            .await
        }
        "resources/list" => handle_resources_list(req.id),
        "resources/read" => handle_resources_read(req.id, req.params, &org, &state).await,
        "ping" => JsonRpcResponse::success(req.id, json!({})),
        _ => JsonRpcResponse::method_not_found(req.id),
    };

    Json(response).into_response()
}

// ============================================================================
// Protocol handlers
// ============================================================================

fn handle_initialize(id: Option<Value>, params: Value) -> JsonRpcResponse {
    let params: InitializeParams = serde_json::from_value(params).unwrap_or_default();
    let protocol_version = negotiate_protocol_version(params.protocol_version.as_deref());
    JsonRpcResponse::success(
        id,
        json!({
            "protocolVersion": protocol_version,
            "capabilities": {
                "tools": {
                    "listChanged": false
                },
                "resources": {}
            },
            "serverInfo": {
                "name": MCP_SERVER_NAME,
                "version": MCP_SERVER_VERSION
            }
        }),
    )
}

fn handle_tools_list(id: Option<Value>, protocol_version: &str) -> JsonRpcResponse {
    JsonRpcResponse::success(id, json!({ "tools": tool_definitions(protocol_version) }))
}

// ============================================================================
// Resource handlers (MCP resources capability)
// ============================================================================

/// Static resource catalog — returned by resources/list.
fn handle_resources_list(id: Option<Value>) -> JsonRpcResponse {
    JsonRpcResponse::success(
        id,
        json!({
            "resources": [
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
            ]
        }),
    )
}

/// Read a resource by URI — fetches fresh data on each call.
async fn handle_resources_read(
    id: Option<Value>,
    params: Value,
    org: &ResolvedOrg,
    state: &AppState,
) -> JsonRpcResponse {
    let uri = match params.get("uri").and_then(|v| v.as_str()) {
        Some(uri) => uri,
        None => return JsonRpcResponse::invalid_params(id, "Missing 'uri' in params"),
    };

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
    let capabilities = state
        .capability_service
        .list_all(org.org_id)
        .await
        .map_err(|e| format!("Failed to list capabilities: {e}"))?;

    let summary: Vec<Value> = capabilities
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

    serde_json::to_string(&summary).map_err(|e| format!("Serialization error: {e}"))
}

async fn read_harnesses(org: &ResolvedOrg, state: &AppState) -> Result<String, String> {
    let harnesses = state
        .db
        .list_harnesses(org.org_id, None, false)
        .await
        .map_err(|e| format!("Failed to list harnesses: {e}"))?;

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

    serde_json::to_string(&summary).map_err(|e| format!("Serialization error: {e}"))
}

async fn read_models(org: &ResolvedOrg, state: &AppState) -> Result<String, String> {
    let providers = state
        .db
        .list_llm_providers(org.org_id)
        .await
        .map_err(|e| format!("Failed to list providers: {e}"))?;

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

    serde_json::to_string(&summary).map_err(|e| format!("Serialization error: {e}"))
}

async fn read_agents(org: &ResolvedOrg, state: &AppState) -> Result<String, String> {
    let (agents, _total) = state
        .db
        .list_agents(
            org.org_id,
            None,
            false,
            crate::api::common::Pagination::new(0, 100),
        )
        .await
        .map_err(|e| format!("Failed to list agents: {e}"))?;

    let summary: Vec<Value> = agents
        .into_iter()
        .map(|a| {
            json!({
                "id": a.public_id,
                "name": a.name,
                "description": a.description,
            })
        })
        .collect();

    serde_json::to_string(&summary).map_err(|e| format!("Serialization error: {e}"))
}

async fn handle_tools_call(
    id: Option<Value>,
    params: Value,
    auth_user: &AuthUser,
    org: &ResolvedOrg,
    state: &AppState,
    _protocol_version: &str,
) -> JsonRpcResponse {
    let tool_name = match params.get("name").and_then(|v| v.as_str()) {
        Some(name) => name,
        None => return JsonRpcResponse::invalid_params(id, "Missing 'name' in params"),
    };

    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or(Value::Object(Default::default()));

    let Some(tool_def) = find_tool_definition(tool_name, _protocol_version) else {
        return JsonRpcResponse::success(
            id,
            json!({
                "content": [{ "type": "text", "text": format!("Unknown tool: {tool_name}") }],
                "isError": true
            }),
        );
    };

    let result = tokio::time::timeout(
        std::time::Duration::from_millis(tool_def.timeout_ms()),
        async {
            match tool_name {
                // Tier 0: identity & org context
                "me" => tool_me(auth_user, org, state).await,
                "list_organizations" => tool_list_organizations(auth_user, state).await,
                "switch_organization" => {
                    tool_switch_organization(&arguments, auth_user, org, state).await
                }
                // Tier 1: agent conversation (org-scoped — accept organization_id override)
                "agent_run" => {
                    match resolve_org_override(&arguments, auth_user, org, state).await {
                        Ok(org) => tool_agent_run(&arguments, &org, state).await,
                        Err(e) => Err(e),
                    }
                }
                "session_send_message" => {
                    match resolve_org_override(&arguments, auth_user, org, state).await {
                        Ok(org) => tool_session_send_message(&arguments, &org, state).await,
                        Err(e) => Err(e),
                    }
                }
                "session_get_status" => {
                    match resolve_org_override(&arguments, auth_user, org, state).await {
                        Ok(org) => tool_session_get_status(&arguments, &org, state).await,
                        Err(e) => Err(e),
                    }
                }
                // Tier 2: catalog & scripting (org-scoped)
                "discover" => match resolve_org_override(&arguments, auth_user, org, state).await {
                    Ok(org) => tool_discover(&arguments, &org, state).await,
                    Err(e) => Err(e),
                },
                "execute" => match resolve_org_override(&arguments, auth_user, org, state).await {
                    Ok(org) => tool_execute(&arguments, &org, state).await,
                    Err(e) => Err(e),
                },
                _ => Err(format!("Unknown tool: {tool_name}")),
            }
        },
    )
    .await
    .unwrap_or_else(|_| Err(format!("Tool timed out after {}ms", tool_def.timeout_ms())));

    match result {
        Ok(content) => {
            let structured_content = if _protocol_version == MCP_PROTOCOL_VERSION_LATEST
                && tool_def.has_output_schema()
            {
                serde_json::from_str::<Value>(&content).ok()
            } else {
                None
            };

            let mut result = json!({
                "content": [{ "type": "text", "text": content }]
            });
            if let Some(structured_content) = structured_content {
                result["structuredContent"] = structured_content;
            }

            JsonRpcResponse::success(id, result)
        }
        Err(msg) => JsonRpcResponse::success(
            id,
            json!({
                "content": [{ "type": "text", "text": msg }],
                "isError": true
            }),
        ),
    }
}

// ============================================================================
// Org override helper
// ============================================================================

/// If the tool arguments contain `organization_id`, validate the user's membership
/// and return a `ResolvedOrg` targeting that org. Otherwise return the default.
///
/// This is the core mechanism for multi-org MCP support: since MCP clients
/// can't set cookies, they pass `organization_id` per-tool-call instead.
async fn resolve_org_override(
    args: &Value,
    auth_user: &AuthUser,
    default_org: &ResolvedOrg,
    state: &AppState,
) -> Result<ResolvedOrg, String> {
    let org_public_id = match args.get("organization_id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => return Ok(default_org.clone()),
    };

    resolve_org_by_id(org_public_id, auth_user, state).await
}

/// Resolve and validate an organization by public ID for the given user.
async fn resolve_org_by_id(
    org_public_id: &str,
    auth_user: &AuthUser,
    state: &AppState,
) -> Result<ResolvedOrg, String> {
    if !validate_org_public_id(org_public_id) {
        return Err(format!("Invalid organization_id format: {org_public_id}"));
    }

    // Query DB for fresh membership (JWT orgs may be stale)
    let user_orgs = state
        .db
        .list_user_organizations(auth_user.id)
        .await
        .map_err(|e| format!("Failed to list user organizations: {e}"))?;

    let org_row = user_orgs
        .iter()
        .find(|o| o.public_id == org_public_id)
        .ok_or_else(|| {
            format!("Organization not found or you are not a member: {org_public_id}")
        })?;

    let role = org_row.role.parse::<OrgRole>().unwrap_or(OrgRole::Member);

    Ok(ResolvedOrg {
        org_id: org_row.org_id,
        public_id: org_row.public_id.clone(),
        name: org_row.name.clone(),
        user_id: Some(auth_user.id),
        role,
        is_platform_user: auth_user.is_platform_user,
    })
}

// ============================================================================
// Tier 0: me
// ============================================================================

async fn tool_me(
    auth_user: &AuthUser,
    org: &ResolvedOrg,
    state: &AppState,
) -> Result<String, String> {
    // Get fresh org memberships from DB
    let user_orgs = state
        .db
        .list_user_organizations(auth_user.id)
        .await
        .map_err(|e| format!("Failed to list user organizations: {e}"))?;

    let orgs: Vec<Value> = user_orgs
        .iter()
        .map(|o| {
            let is_current = o.public_id == org.public_id;
            json!({
                "id": o.public_id,
                "name": o.name,
                "role": o.role,
                "current": is_current,
            })
        })
        .collect();

    Ok(serde_json::to_string_pretty(&json!({
        "user": {
            "id": auth_user.id.to_string(),
            "email": auth_user.email,
            "name": auth_user.name,
        },
        "current_organization": {
            "id": org.public_id,
            "name": org.name,
            "role": org.role.to_string(),
        },
        "organizations": orgs,
    }))
    .unwrap())
}

// ============================================================================
// Tier 0: list_organizations
// ============================================================================

async fn tool_list_organizations(auth_user: &AuthUser, state: &AppState) -> Result<String, String> {
    let user_orgs = state
        .db
        .list_user_organizations(auth_user.id)
        .await
        .map_err(|e| format!("Failed to list user organizations: {e}"))?;

    let orgs: Vec<Value> = user_orgs
        .iter()
        .map(|o| {
            json!({
                "id": o.public_id,
                "name": o.name,
                "role": o.role,
            })
        })
        .collect();

    Ok(serde_json::to_string_pretty(&json!({
        "organizations": orgs,
        "count": orgs.len(),
    }))
    .unwrap())
}

// ============================================================================
// Tier 0: switch_organization
// ============================================================================

/// Switch the active organization for subsequent MCP tool calls.
///
/// Since MCP is stateless (no session/cookie), this tool validates the org
/// and returns instructions. The MCP client should pass the org ID to
/// subsequent calls via `organization_id`, or the platform can set the
/// `everruns_org` cookie on the response if the transport supports it.
async fn tool_switch_organization(
    args: &Value,
    auth_user: &AuthUser,
    _current_org: &ResolvedOrg,
    state: &AppState,
) -> Result<String, String> {
    let org_public_id = args
        .get("organization_id")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: organization_id")?;

    let resolved = resolve_org_by_id(org_public_id, auth_user, state).await?;

    Ok(serde_json::to_string_pretty(&json!({
        "switched": true,
        "organization_id": resolved.public_id,
        "name": resolved.name,
        "role": resolved.role.to_string(),
        "hint": "Pass this organization_id on subsequent tool calls to operate in this org's context."
    }))
    .unwrap())
}

// ============================================================================
// Tier 1: agent_run
// ============================================================================

async fn tool_agent_run(
    args: &Value,
    org: &ResolvedOrg,
    state: &AppState,
) -> Result<String, String> {
    let message_text = args
        .get("message")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: message")?;

    let ctx = catalog_context(org, state);

    // Create session via catalog handler
    let session_params = json!({
        "agent_id": args.get("agent_id"),
        "harness_id": args.get("harness_id"),
        "title": args.get("title"),
        "model_id": args.get("model_id"),
    });
    let session_json = handlers::create_session(&session_params, &ctx).await?;
    let session: Value =
        serde_json::from_str(&session_json).map_err(|e| format!("Internal error: {e}"))?;
    let session_id = session["id"]
        .as_str()
        .ok_or("Internal error: no session ID in response")?;

    // Create budget if budget_limit is specified
    let budget_id = if let Some(budget_limit) = args.get("budget_limit").and_then(|v| v.as_f64()) {
        if budget_limit <= 0.0 {
            return Err("budget_limit must be positive".to_string());
        }
        let budget_soft_limit = args.get("budget_soft_limit").and_then(|v| v.as_f64());
        if budget_soft_limit.is_some_and(|s| s <= 0.0 || s > budget_limit) {
            return Err(
                "budget_soft_limit must be greater than 0 and at most budget_limit".to_string(),
            );
        }
        let budget_params = json!({
            "org_id": org.org_id,
            "subject_type": "session",
            "subject_id": session_id,
            "currency": args.get("budget_currency").and_then(|v| v.as_str()).unwrap_or("usd"),
            "limit": budget_limit,
            "soft_limit": budget_soft_limit,
        });
        let budget_json = handlers::create_budget(&budget_params, &ctx)
            .await
            .map_err(|e| format!("Session created but budget creation failed: {e}"))?;
        let budget: Value =
            serde_json::from_str(&budget_json).map_err(|e| format!("Internal error: {e}"))?;
        // Wrap raw UUID as typed BudgetId for consistent API output
        budget["id"].as_str().map(|raw| {
            if let Ok(uuid) = raw.parse::<uuid::Uuid>() {
                everruns_core::typed_id::BudgetId::from_uuid(uuid).to_string()
            } else {
                raw.to_string()
            }
        })
    } else {
        None
    };

    // Send first message via catalog handler
    let msg_params = json!({
        "session_id": session_id,
        "message": {
            "role": "user",
            "content": [{"type": "text", "text": message_text}]
        }
    });
    let msg_json = handlers::create_message(&msg_params, &ctx).await?;
    let msg: Value = serde_json::from_str(&msg_json).map_err(|e| format!("Internal error: {e}"))?;

    let mut result = json!({
        "session_id": session_id,
        "message_id": msg["id"],
        "status": session["status"],
        "hint": "Use session_get_status to poll for the agent's response, or connect to SSE at /api/v1/sessions/{session_id}/sse"
    });
    if let Some(bid) = budget_id {
        result["budget_id"] = json!(bid);
    }

    Ok(serde_json::to_string_pretty(&result).unwrap())
}

// ============================================================================
// Tier 1: session_send_message
// ============================================================================

async fn tool_session_send_message(
    args: &Value,
    org: &ResolvedOrg,
    state: &AppState,
) -> Result<String, String> {
    let session_id = args
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: session_id")?;

    let message_text = args
        .get("message")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: message")?;

    let ctx = catalog_context(org, state);

    let msg_params = json!({
        "session_id": session_id,
        "message": {
            "role": "user",
            "content": [{"type": "text", "text": message_text}]
        }
    });
    let msg_json = handlers::create_message(&msg_params, &ctx).await?;
    let msg: Value = serde_json::from_str(&msg_json).map_err(|e| format!("Internal error: {e}"))?;

    // Fetch session to get current status (create_message returns message, not session)
    let session_params = json!({ "session_id": session_id });
    let session_json = handlers::get_session(&session_params, &ctx).await?;
    let session: Value =
        serde_json::from_str(&session_json).map_err(|e| format!("Internal error: {e}"))?;

    Ok(serde_json::to_string_pretty(&json!({
        "message_id": msg["id"],
        "session_status": session["status"],
        "hint": "Use session_get_status to poll for completion"
    }))
    .unwrap())
}

// ============================================================================
// Tier 1: session_get_status
// ============================================================================

async fn tool_session_get_status(
    args: &Value,
    org: &ResolvedOrg,
    state: &AppState,
) -> Result<String, String> {
    let session_id = args
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: session_id")?;

    let ctx = catalog_context(org, state);

    // Get session via handler
    let session_params = json!({ "session_id": session_id });
    let session_json = handlers::get_session(&session_params, &ctx).await?;
    let session: Value =
        serde_json::from_str(&session_json).map_err(|e| format!("Internal error: {e}"))?;

    // Get recent events via handler
    let event_types: Vec<String> = args
        .get("event_types")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    // Parse since_event_id as typed EventId and extract UUID for the handler
    let since_id_uuid = args
        .get("since_event_id")
        .and_then(|v| v.as_str())
        .map(|s| {
            s.parse::<everruns_core::typed_id::EventId>()
                .map(|id| id.uuid().to_string())
                .or_else(|_| s.parse::<uuid::Uuid>().map(|u| u.to_string()))
                .map_err(|_| format!("Invalid since_event_id: {s}"))
        })
        .transpose()?;

    let event_params = json!({
        "session_id": session_id,
        "since_id": since_id_uuid,
        "types": if event_types.is_empty() { None } else { Some(event_types.join(",")) },
        "limit": 50,
    });
    let events_json = handlers::list_events(&event_params, &ctx).await?;
    let events_data: Value =
        serde_json::from_str(&events_json).map_err(|e| format!("Internal error: {e}"))?;
    let events = events_data["data"].as_array();

    // Extract latest output from events
    let empty = vec![];
    let events_arr = events.unwrap_or(&empty);
    let latest_output = events_arr
        .iter()
        .rev()
        .find(|e| e.get("event_type").and_then(|v| v.as_str()) == Some("output.message.completed"))
        .and_then(|e| e.get("data"))
        .and_then(|d| d.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())
        .and_then(|parts| {
            parts
                .iter()
                .find(|p| p.get("type").and_then(|t| t.as_str()) == Some("text"))
                .and_then(|p| p.get("text").and_then(|t| t.as_str()))
        });

    let last_event_id = events_arr
        .last()
        .and_then(|e| e.get("id").and_then(|v| v.as_str()));

    Ok(serde_json::to_string_pretty(&json!({
        "session_id": session_id,
        "status": session["status"],
        "agent_id": session["agent_id"],
        "title": session["title"],
        "latest_output": latest_output,
        "last_event_id": last_event_id,
        "event_count": events_arr.len(),
        "events": events_arr.iter().map(|e| json!({
            "id": e["id"],
            "type": e["event_type"],
            "ts": e["ts"],
        })).collect::<Vec<_>>()
    }))
    .unwrap())
}

// ============================================================================
// Tier 2: discover — delegates to ScriptedTool's built-in discover command
// ============================================================================

async fn tool_discover(
    args: &Value,
    org: &ResolvedOrg,
    state: &AppState,
) -> Result<String, String> {
    let show_all = args.get("all").and_then(|v| v.as_bool()).unwrap_or(false);

    let command = if show_all {
        "discover --categories".to_string()
    } else {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
        if query.is_empty() {
            return Err(
                "Provide a 'query' to search or set 'all: true' to list everything.".into(),
            );
        }
        format!("discover --search '{}'", query.replace('\'', "'\\''"))
    };

    let toolset = build_toolset(org, state);
    execute_script(&toolset, &command, 10_000).await
}

// ============================================================================
// Tier 2: execute — delegates to ScriptedTool
// ============================================================================

async fn tool_execute(args: &Value, org: &ResolvedOrg, state: &AppState) -> Result<String, String> {
    let command = args
        .get("commands")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: commands")?;

    let timeout_ms = args
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(30000)
        .min(60000);

    // EVE-323: rewrite `<cmd> <id>` → `<cmd> --<positional> <id>` so LLM
    // callers don't hit bashkit's "expected --flag" rejection.
    let rewritten = positional::rewrite(command, positional::positional_map());

    let toolset = build_toolset(org, state);
    execute_script(&toolset, &rewritten, timeout_ms).await
}

// ============================================================================
// ScriptingToolSet helpers
// ============================================================================

/// Build a CatalogContext for the given org.
fn catalog_context(org: &ResolvedOrg, state: &AppState) -> catalog::CatalogContext {
    catalog::CatalogContext {
        state: state.clone(),
        caller: Caller::from(org),
        org_id: org.org_id,
        user_id: org.user_id,
    }
}

/// Build a ScriptingToolSet for the given org context.
/// All catalog operations are direct service calls — no HTTP.
fn build_toolset(org: &ResolvedOrg, state: &AppState) -> ScriptingToolSet {
    catalog::build_toolset(catalog_context(org, state))
}

/// Execute a script through a ScriptingToolSet and return formatted output.
async fn execute_script(
    toolset: &ScriptingToolSet,
    command: &str,
    timeout_ms: u64,
) -> Result<String, String> {
    let tools = toolset.tools();
    let tool = tools
        .first()
        .ok_or_else(|| "No tools registered in toolset".to_string())?;
    let request = bashkit::ToolRequest::new(command);

    let result = tokio::time::timeout(
        std::time::Duration::from_millis(timeout_ms),
        tool.execute(request),
    )
    .await;

    match result {
        Ok(response) => {
            if response.exit_code == 0 {
                Ok(response.stdout)
            } else {
                let combined = if response.stderr.is_empty() {
                    response.stdout
                } else if response.stdout.is_empty() {
                    response.stderr
                } else {
                    format!("{}\n{}", response.stdout, response.stderr)
                };
                let trimmed = combined.trim();
                if trimmed.is_empty() {
                    Err(format!(
                        "Command failed with exit code {}",
                        response.exit_code
                    ))
                } else {
                    Err(trimmed.to_string())
                }
            }
        }
        Err(_) => Err(format!("Command timed out after {timeout_ms}ms")),
    }
}
