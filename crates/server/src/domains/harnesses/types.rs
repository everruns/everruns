// Harnesses domain types — canonical definitions for request/response shapes.
//
// Storage row types are re-exported from `storage::models` so domain code
// has a single import path.

use everruns_core::typed_id::{HarnessId, ModelId};
use everruns_core::{
    AgentCapabilityConfig, HarnessStatus, InitialFile, ScopedMcpServers, ToolDefinition,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

pub use crate::storage::models::{CreateHarnessRow, HarnessRow, UpdateHarness};

/// Request to create a new harness
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateHarnessRequest {
    /// Name, unique per org. Lowercase alphanumeric and hyphens.
    #[schema(example = "deep-research")]
    pub name: String,
    /// Human-readable display name shown in UI.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Deep Research")]
    pub display_name: Option<String>,
    /// Description of what the harness does.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Research harness with planning and web capabilities")]
    pub description: Option<String>,
    /// The system prompt defining the harness's base behavior.
    #[schema(example = "You are a research assistant with deep analytical capabilities.")]
    pub system_prompt: String,
    /// Optional parent harness to inherit from.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "harness_01933b5a000070008000000000000602")]
    pub parent_harness_id: Option<HarnessId>,
    /// Default LLM model ID for this harness. Lowest priority in model chain.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "model_01933b5a00007000800000000000001")]
    pub default_model_id: Option<ModelId>,
    /// Tags for organizing harnesses.
    #[serde(default)]
    #[schema(example = json!(["research", "planning"]))]
    pub tags: Vec<String>,
    /// Capabilities to enable with per-harness configuration.
    #[serde(default)]
    #[schema(example = json!([{"ref": "current_time", "config": {}}, {"ref": "web_fetch", "config": {}}]))]
    pub capabilities: Vec<AgentCapabilityConfig>,
    /// Starter files copied into each new session for this harness.
    #[serde(default)]
    pub initial_files: Vec<InitialFile>,
    /// Remote MCP servers scoped to this harness.
    #[serde(default, rename = "mcpServers", alias = "mcp_servers")]
    pub mcp_servers: ScopedMcpServers,
    /// Network access list controlling which hosts/URLs sessions can reach.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_access: Option<everruns_core::network_access::NetworkAccessList>,
}

/// Request to update a harness. Only provided fields will be updated.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateHarnessRequest {
    /// Name, unique per org.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "updated-research")]
    pub name: Option<String>,
    /// Human-readable display name.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Updated Research Harness")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, nullable = true)]
    pub parent_harness_id: Option<Option<HarnessId>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub default_model_id: Option<ModelId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Vec<AgentCapabilityConfig>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_files: Option<Vec<InitialFile>>,
    #[serde(
        default,
        rename = "mcpServers",
        alias = "mcp_servers",
        skip_serializing_if = "Option::is_none"
    )]
    pub mcp_servers: Option<ScopedMcpServers>,
    /// Network access list. Send `{}` (empty object) to clear. Omit to leave unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_access: Option<everruns_core::network_access::NetworkAccessList>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<HarnessStatus>,
}

/// Request to preview harness shape with capabilities applied
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct PreviewHarnessRequest {
    #[schema(example = "You are a research assistant.")]
    pub system_prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "harness_01933b5a000070008000000000000602")]
    pub parent_harness_id: Option<HarnessId>,
    #[serde(default)]
    pub capabilities: Vec<AgentCapabilityConfig>,
    #[serde(default, rename = "mcpServers", alias = "mcp_servers")]
    pub mcp_servers: ScopedMcpServers,
}

/// Preview response showing merged prompt and tools
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct HarnessPreviewResponse {
    pub system_prompt: String,
    #[schema(value_type = Vec<Object>)]
    pub tools: Vec<ToolDefinition>,
}

/// Query parameters for checking harness name availability.
#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct CheckNameQuery {
    /// The harness name to check.
    pub name: String,
    /// Optional harness ID to exclude (for edit forms where the current harness's own name is valid).
    pub exclude_id: Option<String>,
}

/// Response for name availability check.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CheckNameResponse {
    /// Whether the name is available for use.
    pub available: bool,
}
