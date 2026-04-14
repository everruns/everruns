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

use crate::auth::middleware::AuthUser;
use crate::auth::{AuthState, ResolvedOrg};
use crate::services::{
    AgentService, BudgetService, CapabilityService, EventService, McpServerService, MessageService,
    SessionService, SkillService,
};
use crate::storage::StorageBackend;
use axum::{Json, Router, extract::State, routing::post};
use bashkit::{ScriptedTool, Tool as _};
use everruns_core::{Caller, OrgRole, PlatformDefinition, validate_org_public_id};
use everruns_worker::AgentRunner;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

use super::common::impl_auth_state;

mod catalog;

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
const MCP_PROTOCOL_VERSION: &str = "2025-03-26";

const ORG_ID_DESCRIPTION: &str = "Optional organization ID (format: org_{32-hex}). Overrides the default organization for this call. Use list_organizations to see available orgs.";

fn tool_definitions() -> Value {
    json!([
        // ── Tier 0: identity & org context ──────────────────────────
        {
            "name": "me",
            "description": "Get the current authenticated user's profile and active organization context. Returns user ID, email, name, and the organization currently used for all operations.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        },
        {
            "name": "list_organizations",
            "description": "List all organizations the authenticated user belongs to, with their role in each. Use this to discover available orgs before switching with switch_organization or passing organization_id to other tools.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        },
        {
            "name": "switch_organization",
            "description": "Validate and select an organization. Returns the validated organization details. Pass the returned organization_id to subsequent tool calls to operate in that org's context. You can also pass organization_id directly to individual tools without calling this first.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "organization_id": {
                        "type": "string",
                        "description": "Organization ID to switch to (format: org_{32-hex}). Must be an org the user belongs to."
                    }
                },
                "required": ["organization_id"]
            }
        },
        // ── Tier 1: agent conversation loop ─────────────────────────
        {
            "name": "agent_run",
            "description": "Create a new session and send the first message to an agent. Returns the session ID and message ID. Use session_get_status to poll for the agent's response, or connect to the SSE stream at /api/v1/sessions/{session_id}/sse for real-time events.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "agent_id": {
                        "type": "string",
                        "description": "Agent ID (format: agent_{32-hex}). The agent to run."
                    },
                    "message": {
                        "type": "string",
                        "description": "The initial user message to send to the agent."
                    },
                    "title": {
                        "type": "string",
                        "description": "Optional session title."
                    },
                    "model_id": {
                        "type": "string",
                        "description": "Optional model override (format: model_{32-hex})."
                    },
                    "budget_limit": {
                        "type": "number",
                        "description": "Optional budget limit. Creates a session budget that stops the agent at this amount. Currency defaults to 'usd'."
                    },
                    "budget_currency": {
                        "type": "string",
                        "description": "Budget currency (default: 'usd'). Options: usd, tokens, credits, or custom."
                    },
                    "budget_soft_limit": {
                        "type": "number",
                        "description": "Optional soft limit — pauses the session at this amount before the hard stop. Must be less than budget_limit."
                    },
                    "organization_id": {
                        "type": "string",
                        "description": ORG_ID_DESCRIPTION
                    }
                },
                "required": ["message"]
            }
        },
        {
            "name": "session_send_message",
            "description": "Send a follow-up message to an existing session. The agent will process the message and generate a response. Use session_get_status to poll for completion, or connect to the SSE stream.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Session ID (format: session_{32-hex})."
                    },
                    "message": {
                        "type": "string",
                        "description": "The user message to send."
                    },
                    "organization_id": {
                        "type": "string",
                        "description": ORG_ID_DESCRIPTION
                    }
                },
                "required": ["session_id", "message"]
            }
        },
        {
            "name": "session_get_status",
            "description": "Get the current status of a session and its recent events. Returns the session status (started/active/idle), the latest agent message if available, and recent events. Use this to poll for agent responses after sending a message.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Session ID (format: session_{32-hex})."
                    },
                    "since_event_id": {
                        "type": "string",
                        "description": "Only return events after this event ID (for incremental polling)."
                    },
                    "event_types": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Filter to specific event types. Useful types: turn.completed, output.message.completed, tool.completed, session.idled"
                    },
                    "organization_id": {
                        "type": "string",
                        "description": ORG_ID_DESCRIPTION
                    }
                },
                "required": ["session_id"]
            }
        },
        // ── Tier 2: catalog & scripting ─────────────────────────────
        {
            "name": "discover",
            "description": concat!(
                "Search the Everruns API catalog to find available operations. ",
                "Returns matching operations with description and parameters.\n\n",
                "Available resource types: agents, sessions, harnesses, capabilities, models, ",
                "providers, mcp servers, skills, budgets, schedules, files, events, messages, ",
                "images, organizations, users, databases, storage.\n\n",
                "Example queries: 'create agent', 'list sessions', 'capabilities', 'mcp'\n\n",
                "Use `all: true` to list every operation grouped by category.\n\n",
                "The discovered operations are available as bash builtins in the 'execute' tool."
            ),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query to find API operations (e.g., 'create agent', 'sessions', 'mcp'). Supports natural language — tokens are matched against names, descriptions, and categories."
                    },
                    "all": {
                        "type": "boolean",
                        "description": "List all available operations grouped by category. When true, query is ignored."
                    }
                }
            }
        },
        {
            "name": "execute",
            "description": concat!(
                "Execute a bash script in an environment where every Everruns API operation is a built-in command.\n\n",
                "## Available commands (~50 builtins across 20 categories)\n",
                "agents: list_agents, create_agent, get_agent, update_agent, delete_agent, ...\n",
                "sessions: list_sessions, create_session, get_session, ...\n",
                "events: list_events, subscribe_events\n",
                "models: list_models, get_model\n",
                "capabilities, budgets, harnesses, mcp_servers, files, messages, schedules, skills, and more.\n\n",
                "Run `discover --categories` to list all categories, or `discover --search <query>` to find a specific command.\n",
                "Run `<command> --help` for usage details on any command.\n\n",
                "## Bash features available\n",
                "Pipes, jq (built-in), variables, loops, conditionals, subshells.\n\n",
                "## Examples\n",
                "# List all agent names\n",
                "list_agents | jq '.data[].name'\n\n",
                "# Create a session and capture its ID\n",
                "SID=$(create_session --agent_id agent_abc123 --title 'Test' | jq -r .id)\n\n",
                "# Iterate over agents\n",
                "list_agents | jq -r '.data[].id' | while read id; do get_agent --id \"$id\"; done\n\n",
                "# Search for commands related to events\n",
                "discover --search events"
            ),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Bash script to execute. API operations are available as built-in commands."
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "description": "Execution timeout in milliseconds (default: 30000, max: 60000)."
                    },
                    "organization_id": {
                        "type": "string",
                        "description": ORG_ID_DESCRIPTION
                    }
                },
                "required": ["command"]
            }
        }
    ])
}

// ============================================================================
// App State
// ============================================================================

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub agent_service: Arc<AgentService>,
    pub session_service: Arc<SessionService>,
    pub message_service: Arc<MessageService>,
    pub event_service: Arc<EventService>,
    pub capability_service: Arc<CapabilityService>,
    pub mcp_server_service: Arc<McpServerService>,
    pub skill_service: Arc<SkillService>,
    pub budget_service: Arc<BudgetService>,
    pub runner: Arc<dyn AgentRunner>,
    pub auth: AuthState,
    pub fallback_base_harness_name: Option<String>,
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
    ) -> Self {
        Self {
            agent_service: Arc::new(AgentService::new(db.clone())),
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
            capability_service,
            mcp_server_service: Arc::new(McpServerService::new(db.clone(), encryption)),
            skill_service: Arc::new(SkillService::new(db.clone())),
            budget_service: Arc::new(BudgetService::new(db.clone())),
            db,
            runner,
            auth,
            fallback_base_harness_name: platform_definition
                .harness_for_role(everruns_core::BuiltInHarnessRole::Base)
                .map(|h| h.name.clone()),
        }
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
    Json(req): Json<JsonRpcRequest>,
) -> Json<JsonRpcResponse> {
    if req.jsonrpc != "2.0" {
        return Json(JsonRpcResponse::error(
            req.id,
            -32600,
            "Invalid Request: jsonrpc must be \"2.0\"",
        ));
    }

    let response = match req.method.as_str() {
        "initialize" => handle_initialize(req.id),
        "tools/list" => handle_tools_list(req.id),
        "tools/call" => {
            handle_tools_call(req.id.clone(), req.params, &auth_user, &org, &state).await
        }
        "resources/list" => handle_resources_list(req.id),
        "resources/read" => handle_resources_read(req.id, req.params, &org, &state).await,
        "ping" => JsonRpcResponse::success(req.id, json!({})),
        _ => JsonRpcResponse::method_not_found(req.id),
    };

    Json(response)
}

// ============================================================================
// Protocol handlers
// ============================================================================

fn handle_initialize(id: Option<Value>) -> JsonRpcResponse {
    JsonRpcResponse::success(
        id,
        json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {
                "tools": {},
                "resources": {}
            },
            "serverInfo": {
                "name": MCP_SERVER_NAME,
                "version": MCP_SERVER_VERSION
            }
        }),
    )
}

fn handle_tools_list(id: Option<Value>) -> JsonRpcResponse {
    JsonRpcResponse::success(id, json!({ "tools": tool_definitions() }))
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
) -> JsonRpcResponse {
    let tool_name = match params.get("name").and_then(|v| v.as_str()) {
        Some(name) => name,
        None => return JsonRpcResponse::invalid_params(id, "Missing 'name' in params"),
    };

    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or(Value::Object(Default::default()));

    let result = match tool_name {
        // Tier 0: identity & org context
        "me" => tool_me(auth_user, org, state).await,
        "list_organizations" => tool_list_organizations(auth_user, state).await,
        "switch_organization" => tool_switch_organization(&arguments, auth_user, org, state).await,
        // Tier 1: agent conversation (org-scoped — accept organization_id override)
        "agent_run" => match resolve_org_override(&arguments, auth_user, org, state).await {
            Ok(org) => tool_agent_run(&arguments, &org, state).await,
            Err(e) => Err(e),
        },
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
    };

    match result {
        Ok(content) => JsonRpcResponse::success(
            id,
            json!({
                "content": [{ "type": "text", "text": content }]
            }),
        ),
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
        budget["id"].as_str().map(String::from)
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

    Ok(serde_json::to_string_pretty(&json!({
        "message_id": msg["id"],
        "session_status": msg["status"],
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

    let event_params = json!({
        "session_id": session_id,
        "since_id": args.get("since_event_id"),
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
        format!("discover --search {query}")
    };

    let tool = build_scripted_tool(org, state);
    execute_script(&tool, &command, 10_000).await
}

// ============================================================================
// Tier 2: execute — delegates to ScriptedTool
// ============================================================================

async fn tool_execute(args: &Value, org: &ResolvedOrg, state: &AppState) -> Result<String, String> {
    let command = args
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: command")?;

    let timeout_ms = args
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(30000)
        .min(60000);

    let tool = build_scripted_tool(org, state);
    execute_script(&tool, command, timeout_ms).await
}

// ============================================================================
// ScriptedTool helpers
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

/// Build a ScriptedTool for the given org context.
/// All catalog operations are direct service calls — no HTTP.
fn build_scripted_tool(org: &ResolvedOrg, state: &AppState) -> ScriptedTool {
    catalog::build_scripted_tool(catalog_context(org, state))
}

/// Execute a script through a ScriptedTool and return formatted output.
async fn execute_script(
    tool: &ScriptedTool,
    command: &str,
    timeout_ms: u64,
) -> Result<String, String> {
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
