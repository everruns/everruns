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
// - Auth: same as rest of API (API key or session cookie via ResolvedOrg)
// - No MCP session state — stateless request/response per JSON-RPC call

use crate::auth::{AuthState, ResolvedOrg};
use crate::services::{EventService, MessageService, SessionService};
use crate::storage::StorageBackend;
use axum::{Json, Router, extract::State, routing::post};
use bashkit::{ScriptedTool, Tool as ScriptedToolTrait};
use everruns_core::typed_id::{AgentId, BudgetId, EventId, HarnessId, SessionId};
use everruns_core::{Caller, PlatformDefinition};
use everruns_worker::AgentRunner;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

use super::common::impl_auth_state;
use super::messages::{CreateMessageRequest, InputMessage, MessageRole};
use super::sessions::CreateSessionRequest;

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

fn tool_definitions() -> Value {
    json!([
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
                    }
                },
                "required": ["session_id"]
            }
        },
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
    pub session_service: Arc<SessionService>,
    pub message_service: Arc<MessageService>,
    pub event_service: Arc<EventService>,
    pub runner: Arc<dyn AgentRunner>,
    pub auth: AuthState,
    pub api_base_url: String,
    pub fallback_base_harness_name: Option<String>,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        runner: Arc<dyn AgentRunner>,
        auth: AuthState,
        platform_definition: &PlatformDefinition,
        notifications_enabled: bool,
        api_base_url: String,
        event_delivery: crate::event_delivery::EventDelivery,
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
            db,
            runner,
            auth,
            api_base_url,
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
    org: ResolvedOrg,
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
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
        "tools/call" => handle_tools_call(req.id.clone(), req.params, &org, &state, &headers).await,
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
                "tools": {}
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

async fn handle_tools_call(
    id: Option<Value>,
    params: Value,
    org: &ResolvedOrg,
    state: &AppState,
    headers: &axum::http::HeaderMap,
) -> JsonRpcResponse {
    let tool_name = match params.get("name").and_then(|v| v.as_str()) {
        Some(name) => name,
        None => return JsonRpcResponse::invalid_params(id, "Missing 'name' in params"),
    };

    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or(Value::Object(Default::default()));

    // Extract Bearer token from request to pass to scripted tool HTTP callbacks.
    // Parse case-insensitively per RFC 7235, matching extract_auth_user behavior.
    let bearer_token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| {
            let (scheme, token) = s.trim().split_once(char::is_whitespace)?;
            if scheme.eq_ignore_ascii_case("Bearer") {
                let token = token.trim();
                if !token.is_empty() {
                    return Some(token);
                }
            }
            None
        })
        .unwrap_or("")
        .to_string();

    let result = match tool_name {
        "agent_run" => tool_agent_run(&arguments, org, state).await,
        "session_send_message" => tool_session_send_message(&arguments, org, state).await,
        "session_get_status" => tool_session_get_status(&arguments, org, state).await,
        "discover" => tool_discover(&arguments).await,
        "execute" => tool_execute(&arguments, org, state, &bearer_token).await,
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

    let agent_id: Option<AgentId> = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .map(|s| s.parse())
        .transpose()
        .map_err(|e| format!("Invalid agent_id: {e}"))?;

    let title = args.get("title").and_then(|v| v.as_str()).map(String::from);

    let model_id = args
        .get("model_id")
        .and_then(|v| v.as_str())
        .map(|s| s.parse())
        .transpose()
        .map_err(|e| format!("Invalid model_id: {e}"))?;

    // Resolve harness
    let harness_id = resolve_base_harness(
        &state.db,
        org.org_id,
        state.fallback_base_harness_name.as_deref(),
    )
    .await
    .map_err(|e| format!("Failed to resolve harness: {e}"))?;

    // Resolve agent internal ID
    let (agent_internal_id, agent_public_id) = if let Some(ref aid) = agent_id {
        let agent_row = state
            .db
            .get_agent_by_public_id(org.org_id, &aid.to_string())
            .await
            .map_err(|e| format!("Failed to resolve agent: {e}"))?
            .ok_or("Agent not found")?;

        let public_id: AgentId = agent_row
            .public_id
            .parse()
            .unwrap_or_else(|_| AgentId::from_uuid(agent_row.id.uuid()));
        (Some(agent_row.id.uuid()), Some(public_id))
    } else {
        (None, None)
    };

    let caller = Caller::from(org);

    // Create session
    let session_req = CreateSessionRequest {
        harness_id: Some(harness_id),
        harness_name: None,
        agent_id,
        agent_identity_id: None,
        title,
        locale: None,
        tags: vec![],
        model_id,
        capabilities: vec![],
        tools: vec![],
        system_prompt: None,
        initial_files: vec![],
        hints: None,
        network_access: None,
        max_iterations: None,
    };

    let session = state
        .session_service
        .create(
            &caller,
            harness_id.uuid(),
            agent_internal_id,
            agent_public_id,
            session_req,
        )
        .await
        .map_err(|e| format!("Failed to create session: {e}"))?;

    // Create budget if budget_limit is specified
    let budget_id = if let Some(budget_limit) = args.get("budget_limit").and_then(|v| v.as_f64()) {
        if budget_limit <= 0.0 {
            return Err("budget_limit must be positive".to_string());
        }
        let budget_currency = args
            .get("budget_currency")
            .and_then(|v| v.as_str())
            .unwrap_or("usd");
        let budget_soft_limit = args.get("budget_soft_limit").and_then(|v| v.as_f64());
        if budget_soft_limit.is_some_and(|s| s <= 0.0 || s > budget_limit) {
            return Err(
                "budget_soft_limit must be greater than 0 and at most budget_limit".to_string(),
            );
        }

        let input = crate::storage::models::CreateBudgetRow {
            org_id: org.org_id,
            subject_type: "session".to_string(),
            subject_id: session.id.to_string(),
            currency: budget_currency.to_string(),
            limit: budget_limit,
            soft_limit: budget_soft_limit,
            period: None,
            metadata: None,
        };
        let row = state
            .db
            .create_budget(input)
            .await
            .map_err(|e| format!("Session created but budget creation failed: {e}"))?;
        Some(BudgetId::from_uuid(row.id).to_string())
    } else {
        None
    };

    // Send first message
    let msg_req = CreateMessageRequest {
        message: InputMessage {
            role: MessageRole::User,
            content: vec![everruns_core::InputContentPart::text(message_text)],
        },
        controls: None,
        metadata: None,
        tags: None,
        external_actor: None,
    };

    let message = state
        .message_service
        .create(
            crate::services::CreateMessageContext {
                org_id: org.org_id,
                user_id: org.user_id,
                harness_id: session.harness_id.uuid(),
                agent_id: session.agent_id.map(|a| a.uuid()),
                session_id: session.id.uuid(),
                event_metadata: None,
            },
            msg_req,
        )
        .await
        .map_err(|e| format!("Failed to send message: {e}"))?;

    let mut result = json!({
        "session_id": session.id.to_string(),
        "message_id": message.id.to_string(),
        "status": session.status.to_string(),
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
    let session_id: SessionId = args
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: session_id")?
        .parse()
        .map_err(|e| format!("Invalid session_id: {e}"))?;

    let message_text = args
        .get("message")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: message")?;

    let caller = Caller::from(org);

    // Verify session exists
    let session = state
        .session_service
        .get(&caller, session_id.uuid(), None)
        .await
        .map_err(|e| format!("Failed to get session: {e}"))?
        .ok_or("Session not found")?;

    let msg_req = CreateMessageRequest {
        message: InputMessage {
            role: MessageRole::User,
            content: vec![everruns_core::InputContentPart::text(message_text)],
        },
        controls: None,
        metadata: None,
        tags: None,
        external_actor: None,
    };

    let message = state
        .message_service
        .create(
            crate::services::CreateMessageContext {
                org_id: org.org_id,
                user_id: org.user_id,
                harness_id: session.harness_id.uuid(),
                agent_id: session.agent_id.map(|a| a.uuid()),
                session_id: session_id.uuid(),
                event_metadata: None,
            },
            msg_req,
        )
        .await
        .map_err(|e| format!("Failed to send message: {e}"))?;

    Ok(serde_json::to_string_pretty(&json!({
        "message_id": message.id.to_string(),
        "session_status": session.status.to_string(),
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
    let session_id: SessionId = args
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: session_id")?
        .parse()
        .map_err(|e| format!("Invalid session_id: {e}"))?;

    let since_event_id: Option<EventId> = args
        .get("since_event_id")
        .and_then(|v| v.as_str())
        .map(|s| s.parse())
        .transpose()
        .map_err(|e| format!("Invalid since_event_id: {e}"))?;

    let event_types: Vec<String> = args
        .get("event_types")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let caller = Caller::from(org);

    // Get session
    let session = state
        .session_service
        .get(&caller, session_id.uuid(), None)
        .await
        .map_err(|e| format!("Failed to get session: {e}"))?
        .ok_or("Session not found")?;

    // Get recent events
    let since_id = since_event_id.map(|id| id.uuid());

    let events = state
        .event_service
        .list(
            session_id.uuid(),
            None,         // since_sequence
            since_id,     // since_id (UUID)
            &event_types, // filter_types
            &[],          // exclude_types
            None,         // before_sequence
            Some(50),     // limit
        )
        .await
        .map_err(|e| format!("Failed to list events: {e}"))?;

    // Extract the latest agent message text from events if available
    let latest_output = events
        .iter()
        .rev()
        .find(|e| e.event_type == "output.message.completed")
        .and_then(|e| {
            if let everruns_core::EventData::OutputMessageCompleted(data) = &e.data {
                data.message.text().map(String::from)
            } else {
                None
            }
        });

    let last_event_id = events.last().map(|e| e.id.to_string());

    Ok(serde_json::to_string_pretty(&json!({
        "session_id": session.id.to_string(),
        "status": session.status.to_string(),
        "agent_id": session.agent_id.map(|a| a.to_string()),
        "title": session.title,
        "latest_output": latest_output,
        "last_event_id": last_event_id,
        "event_count": events.len(),
        "events": events.iter().map(|e| json!({
            "id": e.id.to_string(),
            "type": e.event_type,
            "ts": e.ts.to_rfc3339(),
        })).collect::<Vec<_>>()
    }))
    .unwrap())
}

// ============================================================================
// Tier 2: discover — searches catalog directly via catalog::discover_all/search
// ============================================================================

async fn tool_discover(args: &Value) -> Result<String, String> {
    let show_all = args.get("all").and_then(|v| v.as_bool()).unwrap_or(false);

    if show_all {
        return Ok(catalog::discover_all());
    }

    let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");

    if query.is_empty() {
        return Err("Provide a 'query' to search or set 'all: true' to list everything.".into());
    }

    Ok(catalog::discover_search(query))
}

// ============================================================================
// Tier 2: execute — delegates to ScriptedTool
// ============================================================================

async fn tool_execute(
    args: &Value,
    org: &ResolvedOrg,
    state: &AppState,
    bearer_token: &str,
) -> Result<String, String> {
    let command = args
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: command")?;

    // Normalize `discover --all` to bashkit's `discover --categories`.
    let command = if command.trim() == "discover --all" {
        "discover --categories"
    } else {
        command
    };

    let timeout_ms = args
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(30000)
        .min(60000);

    let tool = build_scripted_tool(org, state, bearer_token);
    execute_script(&tool, command, timeout_ms).await
}

// ============================================================================
// ScriptedTool helpers
// ============================================================================

/// Build a ScriptedTool for the given org context.
///
/// Uses the caller's Bearer token for internal API calls so that MCP OAuth
/// tokens (and API keys) are forwarded to the REST API.
fn build_scripted_tool(org: &ResolvedOrg, state: &AppState, bearer_token: &str) -> ScriptedTool {
    let auth_token = if bearer_token.is_empty() {
        format!("org-{}", org.public_id)
    } else {
        bearer_token.to_string()
    };
    catalog::build_scripted_tool(&state.api_base_url, &auth_token)
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

// ============================================================================
// Helpers
// ============================================================================

async fn resolve_base_harness(
    db: &StorageBackend,
    org_id: i64,
    fallback_name: Option<&str>,
) -> anyhow::Result<HarnessId> {
    let settings = db.get_organization_settings(org_id).await?;
    if let Some(harness_id) = settings.and_then(|row| row.base_harness_id) {
        return Ok(harness_id);
    }

    if let Some(name) = fallback_name {
        let harnesses = db.list_harnesses(org_id, Some(name), false).await?;
        if let Some(h) = harnesses.first() {
            return Ok(h.id);
        }
    }

    anyhow::bail!("No base harness configured for this organization")
}
