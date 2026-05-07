// Capability domain types — re-exports from everruns-core.
//
// CapabilityInfo is the public DTO returned from the API.
// The service layer (CapabilityService) owns the registry logic.

pub use everruns_core::CapabilityInfo;

use chrono::{DateTime, Utc};
use everruns_core::{DeclarativeCapabilityDefinition, DeclarativeCapabilityId};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

pub use crate::storage::models::{
    CreateDeclarativeCapabilityRow, DeclarativeCapabilityRow, UpdateDeclarativeCapability,
};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DeclarativeCapability {
    /// Public resource ID for this persisted declarative capability.
    #[serde(rename = "id")]
    #[schema(value_type = String, example = "cap_01933b5a000070008000000000000001")]
    pub public_id: DeclarativeCapabilityId,
    #[serde(skip, default = "Uuid::nil")]
    pub internal_id: Uuid,
    /// Runtime capability reference. Agents and harnesses may use this or the plain unique name.
    #[schema(example = "declarative:research_pack")]
    pub capability_id: String,
    /// Stable unique name used in capability refs. Lowercase letters, numbers, and underscores.
    #[schema(example = "research_pack")]
    pub name: String,
    /// Human-facing label shown in the UI. Defaults to `name` when omitted.
    #[schema(example = "Research Pack")]
    pub display_name: Option<String>,
    /// Short summary shown in pickers, search results, and API listings.
    #[schema(example = "Adds research instructions, starter files, and MCP tools.")]
    pub description: String,
    /// Lifecycle state for the resource: active, disabled, archived, or deleted.
    #[schema(example = "active")]
    pub status: String,
    /// Declarative capability payload: system prompt, skills, starter files, MCP servers, and metadata.
    #[schema(value_type = Object, example = json!({
        "name": "research_pack",
        "display_name": "Research Pack",
        "description": "Adds research instructions, starter files, and MCP tools.",
        "system_prompt": "Use the research workflow.",
        "skills": ["web-research"],
        "files": [{"path": "/README.md", "content": "Research workflow", "encoding": "utf-8"}],
        "mcp_servers": {}
    }))]
    pub definition: DeclarativeCapabilityDefinition,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateDeclarativeCapabilityRequest {
    /// Definition for the new declarative capability. `name` must be unique per org and becomes the canonical `declarative:<name>` capability ref.
    #[schema(value_type = Object, example = json!({
        "name": "research_pack",
        "display_name": "Research Pack",
        "description": "Adds research instructions, starter files, and MCP tools.",
        "system_prompt": "Use the research workflow.",
        "skills": ["web-research"],
        "files": [{"path": "/README.md", "content": "Research workflow", "encoding": "utf-8"}],
        "mcp_servers": {}
    }))]
    pub definition: DeclarativeCapabilityDefinition,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateDeclarativeCapabilityRequest {
    /// Replacement declarative definition. Changing `name` updates the canonical capability ref after uniqueness validation.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Object, example = json!({
        "name": "research_pack",
        "display_name": "Research Pack",
        "description": "Adds research instructions, starter files, and MCP tools.",
        "system_prompt": "Use the updated research workflow.",
        "skills": ["web-research"],
        "files": [],
        "mcp_servers": {}
    }))]
    pub definition: Option<DeclarativeCapabilityDefinition>,
    /// Optional lifecycle state update. Use `disabled` to hide from runtime selection without archiving.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "disabled")]
    pub status: Option<String>,
}
