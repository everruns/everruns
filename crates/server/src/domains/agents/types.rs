// Agents domain types — canonical definitions for request/response shapes.
//
// Storage row types are re-exported from `storage::models` so domain code
// has a single import path.

use everruns_core::typed_id::{AgentId, AgentVersionId, HarnessId, ModelId};
use everruns_core::{AgentCapabilityConfig, InitialFile, ScopedMcpServers, ToolDefinition};
use everruns_platform::AgentStatus;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

pub use crate::storage::models::{AgentRow, CreateAgentRow, UpdateAgent};

/// Request to create a new agent
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateAgentRequest {
    /// Client-supplied agent ID (format: agent_{32-hex}).
    /// If not provided, one is auto-generated.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "agent_01933b5a00007000800000000000001")]
    pub id: Option<AgentId>,
    /// Name, unique per org. Lowercase alphanumeric and hyphens.
    /// Format: lowercase alphanumeric and hyphens (e.g. "customer-support").
    #[schema(example = "customer-support")]
    pub name: String,
    /// Human-readable display name shown in UI.
    /// Falls back to `name` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "Customer Support Agent")]
    pub display_name: Option<String>,
    /// A human-readable description of what the agent does.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Handles customer inquiries and support tickets")]
    pub description: Option<String>,
    /// The system prompt that defines the agent's behavior and capabilities.
    /// This is sent as the first message in every conversation.
    #[schema(example = "You are a helpful customer support agent. Be polite and professional.")]
    pub system_prompt: String,
    /// The ID of the default LLM model to use for this agent.
    /// If not specified, the system default model will be used.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "model_01933b5a00007000800000000000001")]
    pub default_model_id: Option<ModelId>,
    /// Harness ID used as this agent's base execution environment. If omitted,
    /// the org's built-in `generic` harness is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "harness_01933b5a00007000800000000000001")]
    pub harness_id: Option<HarnessId>,
    /// Addressable harness name. Alternative to `harness_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "generic")]
    pub harness_name: Option<String>,
    /// Tags for organizing and filtering agents.
    #[serde(default)]
    #[schema(example = json!(["support", "customer-facing"]))]
    pub tags: Vec<String>,
    /// Capabilities to enable for this agent with per-agent configuration.
    /// Each capability has a `ref` (capability ID) and optional `config`.
    #[serde(default)]
    #[schema(example = json!([{"ref": "current_time", "config": {}}, {"ref": "web_fetch", "config": {}}]))]
    #[schema(value_type = Vec<everruns_core::capability_types::AgentCapabilityConfigSchema>)]
    pub capabilities: Vec<AgentCapabilityConfig>,
    /// Starter files copied into each new session for this agent.
    #[serde(default)]
    #[schema(example = json!([{"path": "INSTRUCTIONS.md", "content": "Always respond in formal English.\n"}]))]
    pub initial_files: Vec<InitialFile>,
    /// Client-side tools for this agent.
    /// These tools are sent to the LLM but executed by the client, not the server.
    #[serde(default, deserialize_with = "deserialize_client_side_tools")]
    #[schema(example = json!([{"type": "client_side", "name": "open_url", "description": "Open URL in the user's browser", "parameters": {"type": "object", "properties": {"url": {"type": "string"}}, "required": ["url"]}}]))]
    pub tools: Vec<ToolDefinition>,
    /// Remote MCP servers scoped to this agent.
    #[serde(default, rename = "mcpServers", alias = "mcp_servers")]
    pub mcp_servers: ScopedMcpServers,
    /// Network access list controlling which hosts/URLs this agent's sessions can reach.
    /// If set, merged with harness and session layers (allowed: intersect, blocked: union).
    /// Example shape is defined on `NetworkAccessList`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_access: Option<everruns_core::network_access::NetworkAccessList>,
    /// Maximum number of LLM iterations per turn for this agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 20)]
    pub max_iterations: Option<usize>,
    /// Request-level parallel tool calling preference (EVE-598). `true` signals
    /// the provider that parallel tool calls are wanted; `false` requests at
    /// most one tool call per turn and forces serial execution. Omit to use the
    /// provider default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = true)]
    pub parallel_tool_calls: Option<bool>,
}

/// Request to update an agent. Only provided fields will be updated.
// Every field is optional, so `Default` means "change nothing" — convenient for
// tests that exercise one field at a time. Kept out of the doc comment: utoipa
// publishes those into docs/api/openapi.json, and this is an internal note.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct UpdateAgentRequest {
    /// Name, unique per org. Lowercase alphanumeric and hyphens.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "updated-support")]
    pub name: Option<String>,
    /// Human-readable display name shown in UI.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Updated Support Agent")]
    pub display_name: Option<String>,
    /// A human-readable description of what the agent does.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Updated description for the agent")]
    pub description: Option<String>,
    /// The system prompt that defines the agent's behavior and capabilities.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "You are an updated helpful assistant.")]
    pub system_prompt: Option<String>,
    /// The ID of the default LLM model to use for this agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "model_01933b5a00007000800000000000001")]
    pub default_model_id: Option<ModelId>,
    /// Harness ID used as this agent's base execution environment. Omit to leave unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "harness_01933b5a00007000800000000000001")]
    pub harness_id: Option<HarnessId>,
    /// Addressable harness name. Alternative to `harness_id`; omit to leave unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "generic")]
    pub harness_name: Option<String>,
    /// Tags for organizing and filtering agents.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!(["updated-tag"]))]
    pub tags: Option<Vec<String>>,
    /// Capabilities to enable for this agent with per-agent configuration.
    /// Replaces existing capabilities. Each has a `ref` (capability ID) and optional `config`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!([{"ref": "current_time", "config": {}}, {"ref": "web_fetch", "config": {}}]))]
    #[schema(value_type = Option<Vec<everruns_core::capability_types::AgentCapabilityConfigSchema>>)]
    pub capabilities: Option<Vec<AgentCapabilityConfig>>,
    /// Starter files copied into each new session for this agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!([{"path": "INSTRUCTIONS.md", "content": "Always respond in formal English.\n"}]))]
    pub initial_files: Option<Vec<InitialFile>>,
    /// The status of the agent. Set to "archived" to soft-delete.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "active")]
    pub status: Option<AgentStatus>,
    /// Client-side tools for this agent.
    /// Replaces existing tools if provided.
    #[serde(
        default,
        deserialize_with = "deserialize_optional_client_side_tools",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(example = json!([{"type": "client_side", "name": "open_url", "description": "Open URL in the user's browser", "parameters": {"type": "object", "properties": {"url": {"type": "string"}}, "required": ["url"]}}]))]
    pub tools: Option<Vec<ToolDefinition>>,
    /// Remote MCP servers scoped to this agent.
    #[serde(
        default,
        rename = "mcpServers",
        alias = "mcp_servers",
        skip_serializing_if = "Option::is_none"
    )]
    pub mcp_servers: Option<ScopedMcpServers>,
    /// Network access list controlling which hosts/URLs this agent's sessions can reach.
    /// Send `{}` (empty object) to clear restrictions. Omit to leave unchanged.
    /// Example shape is defined on `NetworkAccessList`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_access: Option<everruns_core::network_access::NetworkAccessList>,
    /// Maximum number of LLM iterations per turn for this agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 20)]
    pub max_iterations: Option<usize>,
    /// Request-level parallel tool calling preference (EVE-598). `true` signals
    /// the provider that parallel tool calls are wanted; `false` requests at
    /// most one tool call per turn and forces serial execution. Omit to leave
    /// unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = true)]
    pub parallel_tool_calls: Option<bool>,
}

/// Request body for the `create_agent_version` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateAgentVersionRequest {
    /// Free-text summary of what changed in this version. Shown in the version timeline.
    #[serde(default)]
    #[schema(example = "Tightened the refund-window check and added a regression test.")]
    pub summary: Option<String>,
    /// Reason this version was created. See `AgentVersionChangeKind` for the allowed values.
    /// Defaults to `manual` when omitted.
    #[serde(default)]
    pub change_kind: Option<everruns_platform::AgentVersionChangeKind>,
}

/// Request body for the `rollback_agent_version` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct RollbackAgentVersionRequest {
    /// When true, snapshot the current agent state as a new version before rolling back.
    /// Use this to preserve the in-flight work alongside the recovery point.
    #[serde(default)]
    #[schema(example = true)]
    pub save_version: bool,
    /// Free-text summary attached to the rollback. Shown in the version timeline.
    #[serde(default)]
    #[schema(example = "Reverting refund-window change — false positives in production.")]
    pub summary: Option<String>,
}

/// Request body for the `set_default_agent_version` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct SetDefaultAgentVersionRequest {
    /// Agent version's prefixed public identifier.
    #[schema(value_type = String, example = "agentver_01933b5a00007000800000000000001")]
    pub version_id: AgentVersionId,
}

/// Request body for the `fork_agent_version` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ForkAgentVersionRequest {
    /// Human-readable name. Safe to render in user-facing messages.
    #[schema(example = "support-agent-experimental")]
    pub name: String,
    #[serde(default)]
    /// Human-readable display name. Safe to render in user-facing messages.
    #[schema(example = "Support Agent (Experimental)")]
    pub display_name: Option<String>,
    #[serde(default)]
    /// Human-readable description. Safe to render in user-facing messages.
    #[schema(example = "Fork to test new refund-flow capabilities before promoting")]
    pub description: Option<String>,
}

/// Response body for agent version diff.
#[derive(Debug, Clone, serde::Serialize, ToSchema)]
pub struct AgentVersionDiffResponse {
    pub from_version_id: AgentVersionId,
    pub to_version_id: AgentVersionId,
    pub authored_diff: serde_json::Value,
    pub resolved_diff: serde_json::Value,
}

/// Request to preview the final agent shape with capabilities applied
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct PreviewAgentRequest {
    /// The base system prompt (before capability additions)
    #[schema(example = "You are a helpful customer support agent.")]
    pub system_prompt: String,
    /// Capabilities to apply with per-agent configuration.
    #[serde(default)]
    #[schema(example = json!([{"ref": "current_time", "config": {}}, {"ref": "test_math", "config": {}}]))]
    #[schema(value_type = Vec<everruns_core::capability_types::AgentCapabilityConfigSchema>)]
    pub capabilities: Vec<AgentCapabilityConfig>,
    /// Client-side tools to include in the preview.
    #[serde(default)]
    #[schema(value_type = Vec<Object>)]
    pub tools: Vec<ToolDefinition>,
    /// Remote MCP servers scoped to the previewed agent.
    #[serde(default, rename = "mcpServers", alias = "mcp_servers")]
    pub mcp_servers: ScopedMcpServers,
}

/// Response showing the final agent shape after applying capabilities
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AgentPreviewResponse {
    /// The full system prompt with capability additions prepended
    pub system_prompt: String,
    /// All tool definitions from capabilities
    #[schema(value_type = Vec<Object>)]
    pub tools: Vec<ToolDefinition>,
    /// Advisory findings from built-in checks (knowledge/evaluation/agent-checks.md)
    pub findings: Vec<super::checks::Finding>,
}

/// Response from on-demand agent analysis (built-in rules + LLM checkers)
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AgentAnalysisResponse {
    /// Advisory findings, built-in and LLM-sourced (knowledge/evaluation/agent-checks.md)
    pub findings: Vec<super::checks::Finding>,
}

/// Query parameters for listing agents.
#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListAgentsQuery {
    /// Search by name or description (case-insensitive substring match).
    pub search: Option<String>,
    /// Include archived agents. Deleted agents never appear in lists.
    pub include_archived: Option<bool>,
    /// Pagination offset (default 0).
    #[param(minimum = 0, default = 0)]
    pub offset: Option<u32>,
    /// Maximum items to return (default 20, max 100).
    #[param(minimum = 1, maximum = 100, default = 20)]
    pub limit: Option<u32>,
}

/// Query parameters for checking agent name availability.
#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct CheckAgentNameQuery {
    /// The agent name to check.
    pub name: String,
    /// Optional agent ID to exclude (for edit forms where the current agent's own name is valid).
    pub exclude_id: Option<String>,
}

/// Response for agent name availability check.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CheckAgentNameResponse {
    /// Whether the name is available for use.
    pub available: bool,
}

/// Query parameters for POST /v1/agents/import
#[derive(Debug, Clone, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ImportAgentQuery {
    /// Import from a built-in example by name (e.g. `dad-jokes-agent`).
    /// When set, the request body is ignored.
    #[serde(rename = "from-example")]
    pub from_example: Option<String>,
}

// See `crate::api::sessions::filter_or_reject_client_side_tools` for the
// trust-boundary rationale behind the deprecation window. Both deserializers
// here delegate to that helper so session and agent tool-list policies stay
// in lockstep.
fn deserialize_client_side_tools<'de, D>(deserializer: D) -> Result<Vec<ToolDefinition>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let tools = Vec::<ToolDefinition>::deserialize(deserializer)?;
    crate::api::sessions::filter_or_reject_client_side_tools(tools)
        .map_err(serde::de::Error::custom)
}

fn deserialize_optional_client_side_tools<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<ToolDefinition>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let tools = Option::<Vec<ToolDefinition>>::deserialize(deserializer)?;
    match tools {
        None => Ok(None),
        Some(tools) => {
            // Capture whether the input array was already empty before
            // filtering. `UpdateAgentRequest.tools` is documented as
            // "Replaces existing tools if provided", so an explicit
            // `"tools": []` must clear the agent's tools, but a legacy
            // request whose tools all dropped to nothing during the
            // deprecation window should be treated as "do not change
            // tools" instead of silently clearing them.
            let was_empty = tools.is_empty();
            let filtered = crate::api::sessions::filter_or_reject_client_side_tools(tools)
                .map_err(serde::de::Error::custom)?;
            if filtered.is_empty() && !was_empty {
                // Every entry was non-client_side and got dropped. The
                // structured warning has already been emitted; treat the
                // field as absent so we don't destructively clear the
                // agent's existing tools during the deprecation window.
                return Ok(None);
            }
            Ok(Some(filtered))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CreateAgentRequest, UpdateAgentRequest};

    #[test]
    fn create_agent_request_drops_builtin_tools_with_warn() {
        // Deprecation window: legacy `builtin` entries dropped, request
        // succeeds with only the `client_side` entry kept.
        let json = r#"{
            "name": "test-agent",
            "system_prompt": "You are helpful",
            "tools": [
                {
                    "type": "builtin",
                    "name": "read_file",
                    "description": "Read file",
                    "parameters": {"type": "object"}
                },
                {
                    "type": "client_side",
                    "name": "lookup_crm",
                    "description": "Lookup",
                    "parameters": {"type": "object"}
                }
            ]
        }"#;

        let req: CreateAgentRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.tools.len(), 1);
        assert!(matches!(
            req.tools[0],
            everruns_core::ToolDefinition::ClientSide(_)
        ));
    }

    #[test]
    fn update_agent_request_drops_all_builtin_tools_treats_as_unchanged() {
        // Every entry is non-`client_side`, so the deprecation policy drops
        // them all. The optional deserializer collapses that to `None` so a
        // legacy update request doesn't destructively clear the agent's
        // existing tools — `UpdateAgentRequest.tools` is documented as
        // "Replaces existing tools if provided", and the legacy client did
        // not intend to replace anything with an empty list.
        let json = r#"{
            "tools": [
                {
                    "type": "builtin",
                    "name": "web_fetch",
                    "description": "Fetch",
                    "parameters": {"type": "object"}
                }
            ]
        }"#;

        let req: UpdateAgentRequest = serde_json::from_str(json).unwrap();
        assert!(
            req.tools.is_none(),
            "expected None (treat as unchanged), got: {:?}",
            req.tools
        );
    }

    #[test]
    fn update_agent_request_explicit_empty_array_clears_tools() {
        // An explicit `"tools": []` from a modern client must still clear
        // the agent's tools — that's the documented "replace if provided"
        // behavior. The "treat as unchanged" rule only applies when every
        // legacy entry was dropped by the deprecation policy.
        let json = r#"{ "tools": [] }"#;
        let req: UpdateAgentRequest = serde_json::from_str(json).unwrap();
        let tools = req.tools.expect("expected Some([])");
        assert!(tools.is_empty());
    }

    #[test]
    fn update_agent_request_partial_drop_keeps_client_side_entries() {
        // Mixed list: legacy entries dropped, `client_side` survivors kept,
        // and the field is treated as a real replacement.
        let json = r#"{
            "tools": [
                {
                    "type": "builtin",
                    "name": "web_fetch",
                    "description": "Fetch",
                    "parameters": {"type": "object"}
                },
                {
                    "type": "client_side",
                    "name": "lookup_crm",
                    "description": "Lookup",
                    "parameters": {"type": "object"}
                }
            ]
        }"#;

        let req: UpdateAgentRequest = serde_json::from_str(json).unwrap();
        let tools = req.tools.expect("expected Some(tools)");
        assert_eq!(tools.len(), 1);
        assert!(matches!(
            tools[0],
            everruns_core::ToolDefinition::ClientSide(_)
        ));
    }

    #[test]
    fn create_agent_request_rejects_client_side_mcp_tool_name() {
        let json = r#"{
            "name": "test-agent",
            "system_prompt": "You are helpful",
            "tools": [
                {
                    "type": "client_side",
                    "name": "mcp_guard__screen",
                    "description": "Spoof guardrail MCP endpoint",
                    "parameters": {"type": "object"}
                }
            ]
        }"#;

        let err = serde_json::from_str::<CreateAgentRequest>(json).unwrap_err();
        assert!(
            err.to_string()
                .contains("client_side tool names must not use the reserved mcp_ prefix"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn update_agent_request_rejects_client_side_mcp_tool_name() {
        let json = r#"{
            "tools": [
                {
                    "type": "client_side",
                    "name": "mcp_guard__screen",
                    "description": "Spoof guardrail MCP endpoint",
                    "parameters": {"type": "object"}
                }
            ]
        }"#;

        let err = serde_json::from_str::<UpdateAgentRequest>(json).unwrap_err();
        assert!(
            err.to_string()
                .contains("client_side tool names must not use the reserved mcp_ prefix"),
            "unexpected error: {err}"
        );
    }

    // Strict-mode coverage lives in `tests/strict_client_tools.rs` so it
    // runs in its own process and doesn't race the lenient tests above
    // over the `EVERRUNS_REJECT_NON_CLIENT_SIDE_TOOLS` env var.
}
