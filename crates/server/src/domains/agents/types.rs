// Agents domain types — canonical definitions for request/response shapes.
//
// Storage row types are re-exported from `storage::models` so domain code
// has a single import path.

use everruns_core::typed_id::{AgentId, ModelId};
use everruns_core::{
    AgentCapabilityConfig, AgentStatus, InitialFile, ScopedMcpServers, ToolDefinition,
};
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
    /// Tags for organizing and filtering agents.
    #[serde(default)]
    #[schema(example = json!(["support", "customer-facing"]))]
    pub tags: Vec<String>,
    /// Capabilities to enable for this agent with per-agent configuration.
    /// Each capability has a `ref` (capability ID) and optional `config`.
    #[serde(default)]
    #[schema(example = json!([{"ref": "current_time", "config": {}}, {"ref": "web_fetch", "config": {}}]))]
    pub capabilities: Vec<AgentCapabilityConfig>,
    /// Starter files copied into each new session for this agent.
    #[serde(default)]
    pub initial_files: Vec<InitialFile>,
    /// Client-side tools for this agent.
    /// These tools are sent to the LLM but executed by the client, not the server.
    #[serde(default, deserialize_with = "deserialize_client_side_tools")]
    pub tools: Vec<ToolDefinition>,
    /// Remote MCP servers scoped to this agent.
    #[serde(default, rename = "mcpServers", alias = "mcp_servers")]
    pub mcp_servers: ScopedMcpServers,
    /// Network access list controlling which hosts/URLs this agent's sessions can reach.
    /// If set, merged with harness and session layers (allowed: intersect, blocked: union).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_access: Option<everruns_core::network_access::NetworkAccessList>,
    /// Maximum number of LLM iterations per turn for this agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_iterations: Option<usize>,
}

/// Request to update an agent. Only provided fields will be updated.
#[derive(Debug, Clone, Deserialize, ToSchema)]
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
    /// Tags for organizing and filtering agents.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!(["updated-tag"]))]
    pub tags: Option<Vec<String>>,
    /// Capabilities to enable for this agent with per-agent configuration.
    /// Replaces existing capabilities. Each has a `ref` (capability ID) and optional `config`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!([{"ref": "current_time", "config": {}}, {"ref": "web_fetch", "config": {}}]))]
    pub capabilities: Option<Vec<AgentCapabilityConfig>>,
    /// Starter files copied into each new session for this agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_files: Option<Vec<InitialFile>>,
    /// The status of the agent. Set to "archived" to soft-delete.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<AgentStatus>,
    /// Client-side tools for this agent.
    /// Replaces existing tools if provided.
    #[serde(
        default,
        deserialize_with = "deserialize_optional_client_side_tools",
        skip_serializing_if = "Option::is_none"
    )]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_access: Option<everruns_core::network_access::NetworkAccessList>,
    /// Maximum number of LLM iterations per turn for this agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_iterations: Option<usize>,
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

fn deserialize_client_side_tools<'de, D>(deserializer: D) -> Result<Vec<ToolDefinition>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let tools = Vec::<ToolDefinition>::deserialize(deserializer)?;
    if tools
        .iter()
        .any(|tool| !matches!(tool, ToolDefinition::ClientSide(_)))
    {
        return Err(serde::de::Error::custom(
            "tools must contain only client_side definitions",
        ));
    }
    Ok(tools)
}

fn deserialize_optional_client_side_tools<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<ToolDefinition>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let tools = Option::<Vec<ToolDefinition>>::deserialize(deserializer)?;
    if let Some(tools) = &tools
        && tools
            .iter()
            .any(|tool| !matches!(tool, ToolDefinition::ClientSide(_)))
    {
        return Err(serde::de::Error::custom(
            "tools must contain only client_side definitions",
        ));
    }
    Ok(tools)
}

#[cfg(test)]
mod tests {
    use super::{CreateAgentRequest, UpdateAgentRequest};

    #[test]
    fn create_agent_request_rejects_builtin_tools() {
        let json = r#"{
            "name": "test-agent",
            "system_prompt": "You are helpful",
            "tools": [
                {
                    "type": "builtin",
                    "name": "read_file",
                    "description": "Read file",
                    "parameters": {"type": "object"}
                }
            ]
        }"#;

        let err = serde_json::from_str::<CreateAgentRequest>(json).unwrap_err();
        assert!(
            err.to_string()
                .contains("tools must contain only client_side definitions")
        );
    }

    #[test]
    fn update_agent_request_rejects_builtin_tools() {
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

        let err = serde_json::from_str::<UpdateAgentRequest>(json).unwrap_err();
        assert!(
            err.to_string()
                .contains("tools must contain only client_side definitions")
        );
    }
}
