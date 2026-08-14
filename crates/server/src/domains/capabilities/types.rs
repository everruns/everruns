// Capability domain types — re-exports from everruns-core.
//
// CapabilityInfo is the public DTO returned from the API.
// The service layer (CapabilityService) owns the registry logic.

pub use everruns_core::CapabilityInfo;

use crate::kernel_imports::{
    DeclarativeCapabilityDefinition, everruns_provider::typed_id::DeclarativeCapabilityId,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

pub use crate::storage::models::{
    CreateDeclarativeCapabilityRow, DeclarativeCapabilityRow, UpdateDeclarativeCapability,
};

/// Persisted, org-scoped declarative capability — a YAML/JSON-defined
/// bundle of skills, files, and tool defs that an agent or harness can
/// reference by `capability_id` or name.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DeclarativeCapability {
    /// Public resource ID for this persisted declarative capability.
    #[serde(rename = "id")]
    #[schema(value_type = String, example = "cap_01933b5a000070008000000000000001")]
    pub public_id: DeclarativeCapabilityId,
    #[serde(skip, default = "Uuid::nil")]
    /// Internal database UUID. Not part of the public identifier surface.
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
    /// Timestamp when this resource was created (RFC 3339).
    pub created_at: DateTime<Utc>,
    /// Timestamp when this resource was last updated (RFC 3339).
    pub updated_at: DateTime<Utc>,
    /// Timestamp when this resource was archived, if any (RFC 3339).
    pub archived_at: Option<DateTime<Utc>>,
    /// Timestamp when this resource was soft-deleted, if any (RFC 3339).
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Request body for the `create_declarative_capability` operation.
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

/// Request body for the `dry_run_guardrails` operation: evaluate a
/// guardrails capability config against sample content without a session.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct GuardrailsDryRunRequest {
    /// The `guardrails` capability config to evaluate (same shape persisted
    /// in `AgentCapabilityConfig.config`).
    #[schema(value_type = Object, example = json!({
        "mode": "advisory",
        "checks": [
            {"id": "profanity", "stage": "output", "type": "blocklist", "words": ["darn"]}
        ]
    }))]
    pub config: serde_json::Value,
    /// Pipeline stage to evaluate.
    pub stage: everruns_core::GuardrailStage,
    /// Sample content: model output, serialized tool arguments, or tool
    /// output, depending on `stage`.
    #[schema(example = "the model said something darn surprising")]
    pub text: String,
    /// Tool name for `tool_use` stage checks (`tool_pattern` rules).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "bashkit_exec")]
    pub tool_name: Option<String>,
}

/// One triggered check from a guardrails dry run.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GuardrailsDryRunHit {
    /// Index of the check in the config's `checks` array.
    pub check_index: u32,
    /// The check's `id`, or `"<type>#<index>"` when none was set.
    #[schema(example = "profanity")]
    pub check_id: String,
    /// Stage the check ran in.
    pub stage: everruns_core::GuardrailStage,
    /// Rule type: regex, blocklist, or tool_pattern.
    #[schema(example = "blocklist")]
    pub rule_type: String,
    /// Effective action (advisory mode downgrades block to log).
    pub action: everruns_core::GuardrailAction,
    /// Stable machine-readable code, `guardrail.<rule_type>`.
    #[schema(example = "guardrail.blocklist")]
    pub reason_code: String,
    /// The check's custom replacement text, if configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replacement: Option<String>,
    /// Bounded excerpt of what matched.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "darn")]
    pub matched: Option<String>,
}

/// Response for the `dry_run_guardrails` operation.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GuardrailsDryRunResponse {
    /// All triggered checks, in config order.
    pub hits: Vec<GuardrailsDryRunHit>,
    /// Whether any hit would block (i.e. the content would be suppressed
    /// or the tool call refused when this config runs active).
    pub blocked: bool,
}

/// A read-only, adoptable guardrails preset from the gallery. Adopt by
/// dropping `config` into an agent's `guardrails` capability config.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GuardrailExample {
    /// Stable slug used to reference the preset (e.g. `secret-detection`).
    #[schema(example = "secret-detection")]
    pub name: String,
    /// Human-facing label.
    #[schema(example = "Secret & Credential Detection")]
    pub display_name: String,
    /// What the preset protects against and how to tune it.
    pub description: String,
    /// Tags for grouping/filtering in a picker.
    pub tags: Vec<String>,
    /// Distinct rule types used across the preset's checks
    /// (`regex`, `blocklist`, `tool_pattern`) — the check-type composition.
    pub check_types: Vec<String>,
    /// Distinct stages the preset's checks run in (`output`, `tool_use`,
    /// `tool_output`).
    pub stages: Vec<String>,
    /// Where the preset sends data when it runs. `none` for deterministic
    /// presets (everything runs in-process).
    #[schema(example = "none")]
    pub data_egress: String,
    /// The adoptable `GuardrailsConfig`. Same shape persisted in
    /// `AgentCapabilityConfig.config` for the `guardrails` capability.
    #[schema(value_type = Object, example = json!({
        "mode": "active",
        "checks": [
            {"id": "secret-output", "stage": "output", "type": "regex",
             "patterns": ["AKIA[0-9A-Z]{16}"], "on_fail": "block"}
        ]
    }))]
    pub config: serde_json::Value,
}

/// Response for the `list_guardrail_examples` operation.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GuardrailExamplesResponse {
    /// Adoptable guardrail presets, in display order.
    pub examples: Vec<GuardrailExample>,
}

/// Request body for the `update_declarative_capability` operation.
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
