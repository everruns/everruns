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

use super::supports_rich_tool_shape;

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
        agent_run_tool(protocol_version, org_id_description),
        session_send_message_tool(protocol_version, org_id_description),
        session_get_status_tool(protocol_version, org_id_description),
        discover_tool(protocol_version, org_id_description),
        query_tool(protocol_version, org_id_description),
        execute_tool(protocol_version, org_id_description),
    ];
    // Entity cards (knowledge/ui/mcp-cards.md) require `text/html` embedded
    // resources and tool annotations introduced in MCP 2025-06-18 (and carried
    // forward in 2026-07-28). Older clients negotiate the fallback protocol and
    // don't see card tools.
    if supports_rich_tool_shape(protocol_version) {
        tools.push(agent_get_card_tool(protocol_version, org_id_description));
    }
    // Credential-collecting tools answer with a URL mode elicitation, which is
    // delivered as an MRTR `input_required` result. MRTR exists only in
    // 2026-07-28, and without it the tool could not do its job without asking
    // the client for the credential itself — exactly what it must never do. So
    // 2025-era clients do not see them at all.
    if supports_url_elicitation(protocol_version) {
        tools.push(session_set_secret_tool(
            protocol_version,
            org_id_description,
        ));
        tools.push(connect_tool(protocol_version, org_id_description));
    }
    tools
}

/// Whether the negotiated protocol can carry a URL mode elicitation (MRTR,
/// 2026-07-28).
pub(super) fn supports_url_elicitation(protocol_version: &str) -> bool {
    protocol_version == super::MCP_PROTOCOL_VERSION_LATEST
}

fn session_set_secret_tool(
    protocol_version: &str,
    org_id_description: &str,
) -> McpEndpointToolDefinition {
    tool(
        protocol_version,
        "session_set_secret",
        "Set Session Secret",
        "Store an encrypted secret (API key, token, password) on a session. This tool never \
         accepts the value: pass only the secret's name, and Everruns responds with a URL for \
         the user to open, where a form served by Everruns collects the value directly. The \
         value therefore never passes through this MCP client or the model. Call it again \
         after the user reports finishing to confirm the secret is stored.",
        with_organization_id(
            object_schema(
                vec![
                    id_property("session_id", "Session to store the secret on"),
                    (
                        "name",
                        json!({
                            "type": "string",
                            "description": "Secret name, e.g. OPENAI_API_KEY. Names are case-sensitive.",
                            "minLength": 1,
                            "maxLength": 255
                        }),
                    ),
                ],
                vec!["session_id", "name"],
            ),
            org_id_description,
        ),
        Some(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "name": { "type": "string" },
                "session_id": { "type": "string" },
                "stored": { "type": "boolean" },
                "message": { "type": "string" }
            },
            "required": ["name", "session_id", "stored"]
        })),
        None,
        15_000,
    )
}

fn connect_tool(protocol_version: &str, org_id_description: &str) -> McpEndpointToolDefinition {
    tool(
        protocol_version,
        "connect",
        "Connect a Provider",
        "Authorize Everruns to act on the user's behalf in a third-party provider (the \
         connection an agent needs when a tool reports connection_required). This tool never \
         accepts credentials: Everruns responds with a URL for the user to open, and the \
         authorization happens between the user and the provider. Call it again after the \
         user reports finishing to confirm the connection exists.",
        with_organization_id(
            object_schema(
                vec![(
                    "provider",
                    json!({
                        "type": "string",
                        "description": "Provider to connect, e.g. github. Use discover to see which providers an agent's capabilities require.",
                        "minLength": 1,
                        "maxLength": 255
                    }),
                )],
                vec!["provider"],
            ),
            org_id_description,
        ),
        Some(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "provider": { "type": "string" },
                "connected": { "type": "boolean" },
                "message": { "type": "string" }
            },
            "required": ["provider", "connected"]
        })),
        None,
        15_000,
    )
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
        "Get the current authenticated user's profile and default organization context. MCP calls are stateless; to work in another organization, call list_organizations and pass that org's id as organization_id on each org-scoped tool call.",
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
        "List all organizations the authenticated user belongs to, with their role in each. MCP has no current-organization switch; use this to find an org id, then pass it as organization_id on each org-scoped tool call. When organization_id is omitted, tools use the default organization from me.",
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
        everruns_platform::capabilities::PLATFORM_DISCOVER_DESCRIPTION,
        with_organization_id(
            everruns_platform::capabilities::discover_input_schema(),
            org_id_description,
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
        everruns_platform::capabilities::PLATFORM_QUERY_DESCRIPTION,
        with_organization_id(
            everruns_platform::capabilities::query_input_schema(),
            org_id_description,
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
        everruns_platform::capabilities::PLATFORM_EXECUTE_DESCRIPTION,
        with_organization_id(
            everruns_platform::capabilities::execute_input_schema(),
            org_id_description,
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
        "Render an MCP-Apps card for a single agent: a sandboxed text/html resource with name, status, description, tags, token usage, and session count. Returns an embedded resource at ui://everruns/agent/{agent_id}/card plus a short text summary. The HTML is host-rendered in a sandboxed iframe; future iterations will add interactive buttons (run, archive) routed back through tools/call. See knowledge/ui/mcp-cards.md for the standard.",
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
    let supports_2025_06 = supports_rich_tool_shape(protocol_version);
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

fn with_organization_id(mut schema: Value, description: &str) -> Value {
    schema["properties"]["organization_id"] = json!({
        "type": "string",
        "description": description,
        "pattern": "^org_[0-9a-f]{32}$"
    });
    schema
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
            "read_only": { "type": "boolean" },
            "positional_arg": { "type": "string" },
            "bash_usage": { "type": "string" },
            "output_fields": {
                "type": "array",
                "items": { "type": "string" }
            },
            "schemas_omitted": { "type": "string" },
            "input_schema": { "type": "object", "additionalProperties": true },
            "output_schema": { "type": "object", "additionalProperties": true },
            "output_shape": {
                "type": "string",
                "enum": ["array", "paginated", "unknown"]
            }
        },
        "required": ["name", "category", "description", "read_only", "output_shape"]
    });

    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "count": { "type": "integer" },
            "include_schemas": { "type": "boolean" },
            "result_scope": { "type": "string", "enum": ["operation_catalog"] },
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
            },
            "script_guidance": {
                "type": "array",
                "items": { "type": "string" }
            },
            "refine_hint": { "type": "string" },
            "resource_absence_warning": { "type": "string" }
        },
        "required": ["count", "include_schemas", "result_scope", "shape_hints", "script_guidance"]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn without_organization_id(mut schema: Value) -> Value {
        schema["properties"]
            .as_object_mut()
            .expect("object schema properties")
            .remove("organization_id");
        schema
    }

    #[test]
    fn platform_and_mcp_command_tool_contracts_match() {
        let org_description = "organization";
        let cases = [
            (
                discover_tool("2026-07-28", org_description),
                everruns_platform::capabilities::PLATFORM_DISCOVER_DESCRIPTION,
                everruns_platform::capabilities::discover_input_schema(),
            ),
            (
                query_tool("2026-07-28", org_description),
                everruns_platform::capabilities::PLATFORM_QUERY_DESCRIPTION,
                everruns_platform::capabilities::query_input_schema(),
            ),
            (
                execute_tool("2026-07-28", org_description),
                everruns_platform::capabilities::PLATFORM_EXECUTE_DESCRIPTION,
                everruns_platform::capabilities::execute_input_schema(),
            ),
        ];

        for (mcp, description, platform_schema) in cases {
            assert_eq!(mcp.description, description);
            assert_eq!(without_organization_id(mcp.input_schema), platform_schema);
        }
    }

    #[test]
    fn credential_tools_exist_only_where_elicitation_can_be_delivered() {
        let names = |version: &str| {
            tool_definitions(version, "org")
                .into_iter()
                .map(|tool| tool.name)
                .collect::<Vec<_>>()
        };
        let latest = names("2026-07-28");
        assert!(latest.contains(&"session_set_secret".to_string()));
        assert!(latest.contains(&"connect".to_string()));
        // MRTR — and therefore URL mode elicitation — does not exist before
        // 2026-07-28, so the tools are not offered at all rather than falling
        // back to asking the client for the value.
        for version in ["2025-06-18", "2025-03-26"] {
            let names = names(version);
            assert!(!names.contains(&"session_set_secret".to_string()));
            assert!(!names.contains(&"connect".to_string()));
        }
    }

    #[test]
    fn credential_tools_have_no_parameter_that_could_carry_a_secret() {
        // The whole point of URL mode elicitation: there must be no way for a
        // client (or a model) to pass the value in-band.
        for tool in ["session_set_secret", "connect"] {
            let definition = tool_definition(tool, "2026-07-28", "org").expect("tool present");
            let properties = definition.input_schema["properties"]
                .as_object()
                .expect("object schema");
            for forbidden in [
                "value",
                "secret",
                "token",
                "password",
                "api_key",
                "credential",
            ] {
                assert!(
                    !properties.contains_key(forbidden),
                    "{tool} must not accept '{forbidden}'"
                );
            }
        }
    }
}
