// Harnesses domain types — canonical definitions for request/response shapes.
//
// Storage row types are re-exported from `storage::models` so domain code
// has a single import path.

use std::collections::HashMap;

use crate::kernel_imports::{
    AgentCapabilityConfig, InitialFile, ScopedMcpServers,
    everruns_provider::tool_types::ToolDefinition,
};
use everruns_platform::HarnessStatus;
use everruns_provider::typed_id::{HarnessId, ModelId};
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
    /// Base system prompt defining the harness's behavior. Optional: omit (or
    /// send an empty string) to contribute no base prompt, in which case the
    /// effective prompt comes from the parent harness, agent, session, and
    /// capability layers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "You are a research assistant with deep analytical capabilities.")]
    pub system_prompt: Option<String>,
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
    #[schema(value_type = Vec<everruns_platform::CapabilityRefSchema>)]
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
    /// Arbitrary key-value metadata injected into LLM requests for observability.
    #[serde(default)]
    pub embedder_metadata: HashMap<String, String>,
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
    /// Human-readable description. Safe to render in user-facing messages.
    #[schema(example = "Research harness with web tools")]
    pub description: Option<String>,
    /// New system prompt the harness contributes to sessions; omit to leave unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "You are a research assistant. Cite sources verbatim.")]
    pub system_prompt: Option<String>,
    /// New parent harness for inheritance. Outer `None` leaves unchanged; inner `None` removes inheritance (becomes a root harness).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, nullable = true)]
    pub parent_harness_id: Option<Option<HarnessId>>,
    /// New default model selected when sessions inherit from this harness; omit to leave unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "model_01933b5a00007000800000000000001")]
    pub default_model_id: Option<ModelId>,
    /// Replace the tag list entirely; omit to leave unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!(["research", "web-tools"]))]
    pub tags: Option<Vec<String>>,
    /// Replace the capability list entirely; omit to leave unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!([{"ref": "current_time", "config": {}}, {"ref": "web_fetch", "config": {}}]))]
    #[schema(value_type = Option<Vec<everruns_platform::CapabilityRefSchema>>)]
    pub capabilities: Option<Vec<AgentCapabilityConfig>>,
    /// Replace the initial-files list entirely; omit to leave unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!([{"path": "INSTRUCTIONS.md", "content": "Cite sources verbatim.\n"}]))]
    pub initial_files: Option<Vec<InitialFile>>,
    /// Replace the scoped MCP server set entirely; omit to leave unchanged.
    #[serde(
        default,
        rename = "mcpServers",
        alias = "mcp_servers",
        skip_serializing_if = "Option::is_none"
    )]
    pub mcp_servers: Option<ScopedMcpServers>,
    /// Network access list. Send `{}` (empty object) to clear. Omit to leave unchanged.
    /// Example shape is defined on `NetworkAccessList`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_access: Option<everruns_core::network_access::NetworkAccessList>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Current lifecycle status. Example shape is defined on `HarnessStatus`.
    pub status: Option<HarnessStatus>,
    /// Replace the embedder metadata map entirely; omit to leave unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedder_metadata: Option<HashMap<String, String>>,
}

/// Request to preview harness shape with capabilities applied
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct PreviewHarnessRequest {
    /// System prompt to render as the base prompt for the preview. Optional:
    /// omit to preview a harness that contributes no base prompt of its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "You are a research assistant.")]
    pub system_prompt: Option<String>,
    /// Parent harness to extend. When set, its prompt, capabilities, and MCP servers are
    /// merged with the fields in this request before rendering.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "harness_01933b5a000070008000000000000602")]
    pub parent_harness_id: Option<HarnessId>,
    /// Capability configurations to layer onto the preview. Empty list means none.
    #[serde(default)]
    #[schema(example = json!([{"ref": "web.search", "config": {}}, {"ref": "filesystem.read", "config": {"root": "/workspace"}}]))]
    #[schema(value_type = Vec<everruns_platform::CapabilityRefSchema>)]
    pub capabilities: Vec<AgentCapabilityConfig>,
    /// MCP servers scoped to this preview, keyed by scope (`shared` / per-agent / etc.).
    /// Use the camelCase key `mcpServers` (preferred) or the snake_case alias `mcp_servers`. Empty by default.
    #[serde(default, rename = "mcpServers", alias = "mcp_servers")]
    #[schema(example = json!({"shared": {"name": "filesystem", "transport": "stdio", "command": "mcp-fs", "args": ["--root", "/workspace"]}}))]
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
