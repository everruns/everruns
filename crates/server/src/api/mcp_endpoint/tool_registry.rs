// MCP endpoint tool registry for Everruns' own MCP server.
//
// Decisions:
// - Emit only standard MCP tool fields.
// - Shape tool discovery/results to the negotiated protocol version:
//   `2025-06-18` gets `title`, `outputSchema`, and structured output support;
//   `2025-03-26` omits those newer fields.
// - Keep execution-only data such as timeout internal to the registry.

use everruns_core::McpToolAnnotations;
use serde::Serialize;
use serde_json::{Value, json};

const MCP_PROTOCOL_VERSION_LATEST: &str = "2025-06-18";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpEndpointToolDefinition {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
    #[serde(skip_serializing_if = "Option::is_none", rename = "outputSchema")]
    pub output_schema: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<McpToolAnnotations>,
    #[serde(skip_serializing)]
    timeout_ms: u64,
}

impl McpEndpointToolDefinition {
    pub fn timeout_ms(&self) -> u64 {
        self.timeout_ms
    }

    pub fn has_output_schema(&self) -> bool {
        self.output_schema.is_some()
    }
}

pub fn tool_definitions(
    protocol_version: &str,
    org_id_description: &str,
) -> Vec<McpEndpointToolDefinition> {
    let mut tools = vec![
        me_tool(protocol_version),
        list_organizations_tool(protocol_version),
        switch_organization_tool(protocol_version, org_id_description),
        agent_run_tool(protocol_version, org_id_description),
        session_send_message_tool(protocol_version, org_id_description),
        session_get_status_tool(protocol_version, org_id_description),
        discover_tool(protocol_version, org_id_description),
        query_tool(protocol_version, org_id_description),
        execute_tool(protocol_version, org_id_description),
    ];
    // Entity cards (specs/mcp-cards.md) require `text/html` embedded
    // resources and tool annotations introduced in MCP 2025-06-18. Older
    // clients negotiate the fallback protocol and don't see card tools.
    if protocol_version == MCP_PROTOCOL_VERSION_LATEST {
        tools.push(agent_get_card_tool(protocol_version, org_id_description));
    }
    tools
}

pub fn tool_definition(
    tool_name: &str,
    protocol_version: &str,
    org_id_description: &str,
) -> Option<McpEndpointToolDefinition> {
    tool_definitions(protocol_version, org_id_description)
        .into_iter()
        .find(|tool| tool.name == tool_name)
}

fn me_tool(protocol_version: &str) -> McpEndpointToolDefinition {
    tool(
        protocol_version,
        "me",
        "Current User",
        "Get the current authenticated user's profile and active organization context. Returns user ID, email, name, and the organization currently used for all operations.",
        object_schema(vec![], vec![]),
        Some(me_output_schema()),
        Some(read_only_annotations()),
        5_000,
    )
}

fn list_organizations_tool(protocol_version: &str) -> McpEndpointToolDefinition {
    tool(
        protocol_version,
        "list_organizations",
        "List Organizations",
        "List all organizations the authenticated user belongs to, with their role in each. Use this to discover available orgs before switching with switch_organization or passing organization_id to other tools.",
        object_schema(vec![], vec![]),
        Some(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "organizations": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "id": { "type": "string" },
                            "name": { "type": "string" },
                            "role": { "type": "string" }
                        },
                        "required": ["id", "name", "role"]
                    }
                },
                "count": { "type": "integer" }
            },
            "required": ["organizations", "count"]
        })),
        Some(read_only_annotations()),
        5_000,
    )
}

fn switch_organization_tool(
    protocol_version: &str,
    org_id_description: &str,
) -> McpEndpointToolDefinition {
    tool(
        protocol_version,
        "switch_organization",
        "Switch Organization",
        "Validate and select an organization. Returns the validated organization details. Pass the returned organization_id to subsequent tool calls to operate in that org's context. You can also pass organization_id directly to individual tools without calling this first.",
        object_schema(
            vec![(
                "organization_id",
                json!({
                    "type": "string",
                    "description": org_id_description,
                    "pattern": "^org_[0-9a-f]{32}$"
                }),
            )],
            vec!["organization_id"],
        ),
        Some(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "switched": { "type": "boolean" },
                "organization_id": { "type": "string" },
                "name": { "type": "string" },
                "role": { "type": "string" },
                "hint": { "type": "string" }
            },
            "required": ["switched", "organization_id", "name", "role", "hint"]
        })),
        Some(read_only_annotations()),
        5_000,
    )
}

fn agent_run_tool(protocol_version: &str, org_id_description: &str) -> McpEndpointToolDefinition {
    tool(
        protocol_version,
        "agent_run",
        "Run Agent",
        "Create a new session and send the first message to an agent. Returns the session ID and message ID. Use session_get_status to poll for the agent's response, or connect to the SSE stream at /api/v1/sessions/{session_id}/sse for real-time events.",
        object_schema(
            vec![
                id_property(
                    "agent_id",
                    "Agent ID (format: agent_{32-hex}). The agent to run.",
                ),
                id_property(
                    "harness_id",
                    "Optional harness ID (format: harness_{32-hex}). Used when no agent_id is provided.",
                ),
                (
                    "message",
                    json!({
                        "type": "string",
                        "description": "The initial user message to send to the agent.",
                        "minLength": 1
                    }),
                ),
                (
                    "title",
                    json!({
                        "type": "string",
                        "description": "Optional session title."
                    }),
                ),
                id_property(
                    "model_id",
                    "Optional model override (format: model_{32-hex}).",
                ),
                (
                    "budget_limit",
                    json!({
                        "type": "number",
                        "description": "Optional budget limit. Creates a session budget that stops the agent at this amount."
                    }),
                ),
                (
                    "budget_currency",
                    json!({
                        "type": "string",
                        "description": "Budget currency (default: usd)."
                    }),
                ),
                (
                    "budget_soft_limit",
                    json!({
                        "type": "number",
                        "description": "Optional soft limit. Must be less than or equal to budget_limit."
                    }),
                ),
                (
                    "organization_id",
                    json!({
                        "type": "string",
                        "description": org_id_description,
                        "pattern": "^org_[0-9a-f]{32}$"
                    }),
                ),
            ],
            vec!["message"],
        ),
        Some(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "session_id": { "type": "string" },
                "message_id": { "type": "string" },
                "status": { "type": "string" },
                "hint": { "type": "string" },
                "budget_id": { "type": "string" }
            },
            "required": ["session_id", "message_id", "status", "hint"],
            "additionalProperties": false
        })),
        Some(McpToolAnnotations {
            read_only_hint: Some(false),
            destructive_hint: Some(false),
            idempotent_hint: Some(false),
            open_world_hint: Some(true),
        }),
        15_000,
    )
}

fn session_send_message_tool(
    protocol_version: &str,
    org_id_description: &str,
) -> McpEndpointToolDefinition {
    tool(
        protocol_version,
        "session_send_message",
        "Send Session Message",
        "Send a follow-up message to an existing session. The agent will process the message and generate a response. Use session_get_status to poll for completion, or connect to the SSE stream.",
        object_schema(
            vec![
                id_property("session_id", "Session ID (format: session_{32-hex})."),
                (
                    "message",
                    json!({
                        "type": "string",
                        "description": "The user message to send.",
                        "minLength": 1
                    }),
                ),
                (
                    "organization_id",
                    json!({
                        "type": "string",
                        "description": org_id_description,
                        "pattern": "^org_[0-9a-f]{32}$"
                    }),
                ),
            ],
            vec!["session_id", "message"],
        ),
        Some(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "message_id": { "type": "string" },
                "session_status": { "type": "string" },
                "hint": { "type": "string" }
            },
            "required": ["message_id", "session_status", "hint"]
        })),
        Some(McpToolAnnotations {
            read_only_hint: Some(false),
            destructive_hint: Some(false),
            idempotent_hint: Some(false),
            open_world_hint: Some(true),
        }),
        15_000,
    )
}

fn session_get_status_tool(
    protocol_version: &str,
    org_id_description: &str,
) -> McpEndpointToolDefinition {
    tool(
        protocol_version,
        "session_get_status",
        "Get Session Status",
        "Get the current status of a session and its recent events. Returns the session status (started/active/idle), the latest agent message if available, and recent events. Use this to poll for agent responses after sending a message.",
        object_schema(
            vec![
                id_property("session_id", "Session ID (format: session_{32-hex})."),
                (
                    "since_event_id",
                    json!({
                        "type": "string",
                        "description": "Only return events after this event ID (for incremental polling)."
                    }),
                ),
                (
                    "event_types",
                    json!({
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Filter to specific event types."
                    }),
                ),
                (
                    "organization_id",
                    json!({
                        "type": "string",
                        "description": org_id_description,
                        "pattern": "^org_[0-9a-f]{32}$"
                    }),
                ),
            ],
            vec!["session_id"],
        ),
        Some(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "session_id": { "type": "string" },
                "status": { "type": ["string", "null"] },
                "agent_id": { "type": ["string", "null"] },
                "title": { "type": ["string", "null"] },
                "latest_output": { "type": ["string", "null"] },
                "last_event_id": { "type": ["string", "null"] },
                "event_count": { "type": "integer" },
                "events": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "id": { "type": ["string", "null"] },
                            "type": { "type": ["string", "null"] },
                            "ts": { "type": ["string", "null"] }
                        },
                        "required": ["id", "type", "ts"]
                    }
                }
            },
            "required": ["session_id", "status", "agent_id", "title", "latest_output", "last_event_id", "event_count", "events"]
        })),
        Some(read_only_annotations()),
        10_000,
    )
}

fn discover_tool(protocol_version: &str, org_id_description: &str) -> McpEndpointToolDefinition {
    tool(
        protocol_version,
        "discover",
        "Discover Operations",
        "Search the Everruns API catalog to find available operations. Returns matching operations with input/output schemas and jq-oriented output shape hints. Use all: true to list every operation grouped by category. Read-only operations are available as bash builtins in query; the full catalog is available in execute.",
        object_schema(
            vec![
                (
                    "query",
                    json!({
                        "type": "string",
                        "description": "Search query to find API operations."
                    }),
                ),
                (
                    "all",
                    json!({
                        "type": "boolean",
                        "description": "List all available operations grouped by category. When true, query is ignored."
                    }),
                ),
                (
                    "include_schemas",
                    json!({
                        "type": "boolean",
                        "description": "Include input_schema and output_schema in results. Defaults to true for search and false for all."
                    }),
                ),
                (
                    "organization_id",
                    json!({
                        "type": "string",
                        "description": org_id_description,
                        "pattern": "^org_[0-9a-f]{32}$"
                    }),
                ),
            ],
            vec![],
        ),
        Some(discover_output_schema()),
        Some(read_only_annotations()),
        10_000,
    )
}

fn query_tool(protocol_version: &str, org_id_description: &str) -> McpEndpointToolDefinition {
    tool(
        protocol_version,
        "query",
        "Query Commands",
        "Execute a bash script in an environment where only read-only Everruns API operations are available as built-in commands. Supports pipes, variables, loops, conditionals, jq, and direct access to safe builtins. Mutating commands are intentionally unavailable.",
        object_schema(
            vec![
                (
                    "commands",
                    json!({
                        "type": "string",
                        "description": "Bash script to execute. Only read-only API operations are available as built-in commands.",
                        "minLength": 1
                    }),
                ),
                (
                    "timeout_ms",
                    json!({
                        "type": "integer",
                        "description": "Execution timeout in milliseconds (default: 30000, max: 60000).",
                        "minimum": 1,
                        "maximum": 60000
                    }),
                ),
                (
                    "organization_id",
                    json!({
                        "type": "string",
                        "description": org_id_description,
                        "pattern": "^org_[0-9a-f]{32}$"
                    }),
                ),
            ],
            vec!["commands"],
        ),
        None,
        Some(read_only_annotations()),
        65_000,
    )
}

fn execute_tool(protocol_version: &str, org_id_description: &str) -> McpEndpointToolDefinition {
    tool(
        protocol_version,
        "execute",
        "Execute Commands",
        "Execute a bash script in an environment where every Everruns API operation is a built-in command, including operations with side effects. Supports pipes, variables, loops, conditionals, jq, and direct access to the full builtin set. Prefer query for read-only inspection; use execute when you need create/update/delete or other mutating operations.",
        object_schema(
            vec![
                (
                    "commands",
                    json!({
                        "type": "string",
                        "description": "Bash script to execute. API operations are available as built-in commands.",
                        "minLength": 1
                    }),
                ),
                (
                    "timeout_ms",
                    json!({
                        "type": "integer",
                        "description": "Execution timeout in milliseconds (default: 30000, max: 60000).",
                        "minimum": 1,
                        "maximum": 60000
                    }),
                ),
                (
                    "organization_id",
                    json!({
                        "type": "string",
                        "description": org_id_description,
                        "pattern": "^org_[0-9a-f]{32}$"
                    }),
                ),
            ],
            vec!["commands"],
        ),
        None,
        Some(McpToolAnnotations {
            read_only_hint: Some(false),
            destructive_hint: Some(true),
            idempotent_hint: Some(false),
            open_world_hint: Some(true),
        }),
        65_000,
    )
}

fn agent_get_card_tool(
    protocol_version: &str,
    org_id_description: &str,
) -> McpEndpointToolDefinition {
    tool(
        protocol_version,
        "agent_get_card",
        "Get Agent Card",
        "Render an MCP-Apps card for a single agent: a sandboxed text/html resource with name, status, description, tags, token usage, and session count. Returns an embedded resource at ui://everruns/agent/{agent_id}/card plus a short text summary. The HTML is host-rendered in a sandboxed iframe; future iterations will add interactive buttons (run, archive) routed back through tools/call. See specs/mcp-cards.md for the standard.",
        object_schema(
            vec![
                (
                    "agent_id",
                    json!({
                        "type": "string",
                        "description": "Agent ID (format: agent_{32-hex}) or unique agent name within the organization.",
                        "minLength": 1
                    }),
                ),
                (
                    "organization_id",
                    json!({
                        "type": "string",
                        "description": org_id_description,
                        "pattern": "^org_[0-9a-f]{32}$"
                    }),
                ),
            ],
            vec!["agent_id"],
        ),
        // No JSON outputSchema — card tools return a content array
        // (embedded resource + summary text), not structured JSON.
        None,
        Some(read_only_annotations()),
        10_000,
    )
}

#[allow(clippy::too_many_arguments)]
fn tool(
    protocol_version: &str,
    name: &str,
    title: &str,
    description: &str,
    input_schema: Value,
    output_schema: Option<Value>,
    annotations: Option<McpToolAnnotations>,
    timeout_ms: u64,
) -> McpEndpointToolDefinition {
    let supports_2025_06 = protocol_version == MCP_PROTOCOL_VERSION_LATEST;
    McpEndpointToolDefinition {
        name: name.to_string(),
        title: supports_2025_06.then(|| title.to_string()),
        description: description.to_string(),
        input_schema,
        output_schema: supports_2025_06.then_some(output_schema).flatten(),
        annotations,
        timeout_ms,
    }
}

fn read_only_annotations() -> McpToolAnnotations {
    McpToolAnnotations {
        read_only_hint: Some(true),
        destructive_hint: Some(false),
        idempotent_hint: Some(true),
        open_world_hint: Some(false),
    }
}

fn object_schema(properties: Vec<(&str, Value)>, required: Vec<&str>) -> Value {
    let props = properties
        .into_iter()
        .map(|(name, schema)| (name.to_string(), schema))
        .collect::<serde_json::Map<String, Value>>();
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": props,
        "required": required
    })
}

fn id_property<'a>(name: &'a str, description: &str) -> (&'a str, Value) {
    let pattern = match name {
        "agent_id" => "^agent_[0-9a-f]{32}$",
        "harness_id" => "^harness_[0-9a-f]{32}$",
        "model_id" => "^model_[0-9a-f]{32}$",
        "session_id" => "^session_[0-9a-f]{32}$",
        _ => "^[a-z]+_[0-9a-f]{32}$",
    };
    (
        name,
        json!({
            "type": "string",
            "description": description,
            "pattern": pattern
        }),
    )
}

fn me_output_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "user": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "id": { "type": "string" },
                    "email": { "type": ["string", "null"] },
                    "name": { "type": ["string", "null"] }
                },
                "required": ["id", "email", "name"]
            },
            "current_organization": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "id": { "type": "string" },
                    "name": { "type": "string" },
                    "role": { "type": "string" }
                },
                "required": ["id", "name", "role"]
            },
            "organizations": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "id": { "type": "string" },
                        "name": { "type": "string" },
                        "role": { "type": "string" },
                        "current": { "type": "boolean" }
                    },
                    "required": ["id", "name", "role", "current"]
                }
            }
        },
        "required": ["user", "current_organization", "organizations"]
    })
}

fn discover_output_schema() -> Value {
    let operation_schema = json!({
        "type": "object",
        "additionalProperties": true,
        "properties": {
            "name": { "type": "string" },
            "category": { "type": "string" },
            "description": { "type": "string" },
            "method": { "type": "string" },
            "path": { "type": "string" },
            "read_only": { "type": "boolean" },
            "positional_arg": { "type": "string" },
            "input_schema": { "type": "object", "additionalProperties": true },
            "output_schema": { "type": "object", "additionalProperties": true },
            "output_shape": {
                "type": "string",
                "enum": ["array", "paginated", "unknown"]
            }
        },
        "required": ["name", "category", "description", "method", "path", "read_only", "output_shape"]
    });

    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "count": { "type": "integer" },
            "include_schemas": { "type": "boolean" },
            "operations": {
                "type": "array",
                "items": operation_schema
            },
            "categories": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "category": { "type": "string" },
                        "count": { "type": "integer" },
                        "operations": {
                            "type": "array",
                            "items": operation_schema
                        }
                    },
                    "required": ["category", "count", "operations"]
                }
            },
            "shape_hints": {
                "type": "object",
                "additionalProperties": { "type": "string" }
            }
        },
        "required": ["count", "include_schemas", "shape_hints"]
    })
}
