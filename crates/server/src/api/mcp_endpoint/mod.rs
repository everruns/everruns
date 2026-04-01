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
use everruns_core::typed_id::{AgentId, EventId, HarnessId, SessionId};
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
            "description": "Create a new session and send the first message to an agent. Returns the session ID and message ID. Use session_get_status to poll for the agent's response, or connect to the SSE stream at /v1/sessions/{session_id}/sse for real-time events.",
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
                "Returns matching API operations with their description, parameters, and usage.\n\n",
                "Example queries: 'list agents', 'create session', 'events', 'mcp servers', 'models'\n\n",
                "The discovered operations are available as bash builtins in the 'execute' tool."
            ),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query to find API operations (e.g., 'list agents', 'session events', 'mcp')."
                    }
                },
                "required": ["query"]
            }
        },
        {
            "name": "execute",
            "description": concat!(
                "Execute a bash script with all Everruns API operations available as built-in commands.\n\n",
                "Available commands include: list_agents, create_agent, get_agent, list_sessions, ",
                "create_session, list_events, list_models, and many more.\n\n",
                "Use 'discover' to find available commands, or run `discover --categories` inside the script.\n\n",
                "Example:\n",
                "  list_agents | jq '.data[] | .name'\n",
                "  get_agent --id agent_abc123\n",
                "  create_session --agent_id agent_abc123 --title 'Test' | jq .id\n\n",
                "Scripts can use pipes, jq, loops, conditionals, and other bash features. ",
                "Results are returned as {stdout, stderr, exit_code, success}."
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
        "tools/call" => handle_tools_call(req.id.clone(), req.params, &org, &state).await,
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
        "agent_run" => tool_agent_run(&arguments, org, state).await,
        "session_send_message" => tool_session_send_message(&arguments, org, state).await,
        "session_get_status" => tool_session_get_status(&arguments, org, state).await,
        "discover" => tool_discover(&arguments, org, state).await,
        "execute" => tool_execute(&arguments, org, state).await,
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

    Ok(serde_json::to_string_pretty(&json!({
        "session_id": session.id.to_string(),
        "message_id": message.id.to_string(),
        "status": session.status.to_string(),
        "hint": "Use session_get_status to poll for the agent's response, or connect to SSE at /v1/sessions/{session_id}/sse"
    }))
    .unwrap())
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
// Tier 2: discover — delegates to ScriptedTool's built-in `discover` command
// ============================================================================

async fn tool_discover(
    args: &Value,
    org: &ResolvedOrg,
    state: &AppState,
) -> Result<String, String> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: query")?;

    let tool = build_scripted_tool(org, state);
    let script = format!("discover --search {}", shell_escape(query));
    execute_script(&tool, &script, 10_000).await
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

/// Build a ScriptedTool for the given org context.
fn build_scripted_tool(org: &ResolvedOrg, state: &AppState) -> ScriptedTool {
    let api_key = format!("org-{}", org.public_id);
    catalog::build_scripted_tool(&state.api_base_url, &api_key)
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
            let output = serde_json::to_string_pretty(&json!({
                "stdout": response.stdout,
                "stderr": response.stderr,
                "exit_code": response.exit_code,
                "success": response.exit_code == 0
            }))
            .unwrap();

            if response.exit_code == 0 {
                Ok(output)
            } else {
                Err(output)
            }
        }
        Err(_) => Err(format!("Command timed out after {timeout_ms}ms")),
    }
}

/// Simple shell escaping for a single argument.
fn shell_escape(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
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
