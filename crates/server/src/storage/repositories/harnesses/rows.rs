// Harness and harness capability rows (base configuration for sessions).

use crate::kernel_imports::contracts::typed_id::{HarnessId, ModelId};
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct HarnessRow {
    pub id: HarnessId,
    pub org_id: i64,
    pub name: String,
    #[sqlx(default)]
    pub display_name: Option<String>,
    /// Display glyph name rendered by the UI. Set from built-in definitions.
    #[sqlx(default)]
    pub icon: Option<String>,
    pub description: Option<String>,
    /// Markdown intro for fresh Platform Chat threads (agent wins).
    #[sqlx(default)]
    pub intro_markdown: Option<String>,
    /// One-line description in simplified Markdown (agent wins).
    #[sqlx(default)]
    pub short_description: Option<String>,
    /// Conversation starters (JSONB in DB, agent wins when non-empty).
    #[sqlx(default)]
    pub starters: serde_json::Value,
    /// Base system prompt. Nullable: a harness may contribute no base prompt
    /// and rely entirely on inheritance, agent, session, and capability layers.
    pub system_prompt: Option<String>,
    pub parent_harness_id: Option<HarnessId>,
    pub default_model_id: Option<ModelId>,
    pub tags: Vec<String>,
    /// Starter files copied into new sessions (JSONB in DB)
    #[sqlx(default)]
    pub initial_files: serde_json::Value,
    /// Network access list (JSONB in DB, nullable)
    #[sqlx(default)]
    pub network_access: Option<serde_json::Value>,
    /// Scoped MCP server configs (JSONB in DB)
    #[sqlx(default)]
    pub mcp_servers: serde_json::Value,
    /// Embedder-supplied metadata for LLM observability (JSONB in DB)
    #[sqlx(default)]
    pub embedder_metadata: serde_json::Value,
    pub is_built_in: bool,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct CreateHarnessRow {
    pub name: String,
    pub display_name: Option<String>,
    /// Display glyph name rendered by the UI.
    pub icon: Option<String>,
    pub description: Option<String>,
    /// Markdown intro for fresh Platform Chat threads (agent wins).
    pub intro_markdown: Option<String>,
    /// One-line description in simplified Markdown (agent wins).
    pub short_description: Option<String>,
    /// Conversation starters (JSONB in DB, agent wins when non-empty).
    pub starters: serde_json::Value,
    /// Base system prompt; `None` means the harness contributes no base prompt.
    pub system_prompt: Option<String>,
    pub parent_harness_id: Option<HarnessId>,
    pub default_model_id: Option<ModelId>,
    pub tags: Vec<String>,
    /// Starter files copied into new sessions (JSONB in DB)
    pub initial_files: serde_json::Value,
    /// Scoped MCP server configs (JSONB in DB)
    pub mcp_servers: serde_json::Value,
    /// Network access list (JSONB in DB)
    pub network_access: Option<serde_json::Value>,
    /// Embedder-supplied metadata for LLM observability (JSONB in DB)
    pub embedder_metadata: serde_json::Value,
    pub is_built_in: bool,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateHarness {
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    /// None = leave unchanged; Some(None) = clear; Some(Some(v)) = set.
    pub intro_markdown: Option<Option<String>>,
    /// None = leave unchanged; Some(None) = clear; Some(Some(v)) = set.
    pub short_description: Option<Option<String>>,
    /// Conversation starters (JSONB); None = leave unchanged.
    pub starters: Option<serde_json::Value>,
    /// None = leave unchanged; Some(None) = clear to no base prompt;
    /// Some(Some(v)) = set to v.
    pub system_prompt: Option<Option<String>>,
    pub parent_harness_id: Option<Option<HarnessId>>,
    pub default_model_id: Option<ModelId>,
    pub tags: Option<Vec<String>>,
    pub initial_files: Option<serde_json::Value>,
    pub mcp_servers: Option<serde_json::Value>,
    pub network_access: Option<Option<serde_json::Value>>,
    pub embedder_metadata: Option<serde_json::Value>,
    pub status: Option<String>,
}

#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct HarnessCapabilityRow {
    pub id: Uuid,
    pub harness_id: HarnessId,
    pub capability_id: String,
    pub position: i32,
    pub config: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateHarnessCapabilityRow {
    pub harness_id: HarnessId,
    pub capability_id: String,
    pub position: i32,
    pub config: serde_json::Value,
}
