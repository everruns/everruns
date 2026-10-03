//! `CreateSessionRow` and its `Default`.
//!
//! The struct has thirty fields and most callers care about three or four.
//! Spelling all thirty at every construction site made adding a column a
//! mechanical edit across dozens of files that had no opinion about it —
//! `sessions.trigger_id` (EVE-1138) touched ninety-six of them — and pushed
//! several files that are already on the size debt list further up it.
//!
//! The struct lives here rather than in `models.rs`, which is on that list,
//! and is re-exported from there so callers are unaffected.
//!
//! Every field defaults to its inert value: no agent, no ingress, no workspace,
//! no blueprint, no capabilities. The one field with no meaningful zero is
//! `owner_principal_id`; it gets a freshly generated principal id rather than
//! the nil uuid, so a caller that forgets to set an owner produces a session
//! owned by a principal that does not exist — which fails a foreign key — in
//! preference to one that silently joins whatever nil-owned rows exist.
//!
//! A new column therefore defaults to "absent" without anyone opting in. That
//! is the right default for an attribution or ingress pointer, which is what
//! these columns are; a column that must be set deliberately should be added
//! to the *request* types instead, where the compiler still demands it.
//!
//! `CreateSessionRequest` in `api/sessions.rs` carries a `Default` for the same
//! reason, and it is sound there for a stronger one: every field already has
//! `#[serde(default)]`, so `Default` is exactly what a `{}` body deserializes
//! to. That rationale lives here rather than on the struct because utoipa
//! publishes a doc comment as the schema `description`, and Rust's `Default`
//! means nothing to an API caller reading the OpenAPI spec.

use crate::kernel_imports::{
    contracts::typed_id::AgentId, contracts::typed_id::HarnessId, contracts::typed_id::ModelId,
    contracts::typed_id::PrincipalId, contracts::typed_id::VirtualUserId,
};
use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct CreateSessionRow {
    pub org_id: i64,
    /// How this session was started. Set by the creating ingress path, never
    /// taken from untrusted client input except for client-declarable
    /// variants (see `SessionSource::is_client_declarable`).
    pub source: everruns_platform::SessionSource,
    pub app_id: Option<Uuid>,
    /// The two ingress pointers (EVE-1004, EVE-1138): the endpoint, set by the
    /// app-channel paths that all know theirs, and the trigger, set by the
    /// trigger invocation path. Both `None` for every other way in.
    pub endpoint_id: Option<Uuid>,
    pub trigger_id: Option<Uuid>,
    pub harness_id: Option<HarnessId>,
    pub agent_id: Option<AgentId>,
    pub agent_version_id: Option<everruns_contracts::typed_id::AgentVersionId>,
    pub agent_config_hash: Option<String>,
    pub virtual_user_id: Option<VirtualUserId>,
    pub playground_user_id: Option<VirtualUserId>,
    pub owner_principal_id: PrincipalId,
    pub resolved_owner_user_id: Option<Uuid>,
    pub title: Option<String>,
    pub locale: Option<String>,
    pub tags: Vec<String>,
    pub model_id: Option<ModelId>,
    /// Session-level capabilities (additive to agent capabilities)
    pub capabilities: serde_json::Value,
    /// Client-side tools (additive to agent tools, JSONB in DB)
    pub tools: serde_json::Value,
    /// Scoped MCP server configs (JSONB in DB)
    pub mcp_servers: serde_json::Value,
    /// Session-level system prompt override (prepended to agent prompt)
    pub system_prompt: Option<String>,
    /// Session-level initial files (JSONB in DB, additive to agent files)
    pub initial_files: serde_json::Value,
    /// Session-level client hints (JSONB in DB)
    pub hints: Option<serde_json::Value>,
    /// Network access list (JSONB in DB)
    pub network_access: Option<serde_json::Value>,
    /// Maximum iterations per turn
    pub max_iterations: Option<i32>,
    /// Request-level parallel tool calling preference (EVE-598)
    pub parallel_tool_calls: Option<bool>,
    /// Blueprint ID for blueprint-backed sessions.
    pub blueprint_id: Option<String>,
    /// Validated blueprint config (JSONB in DB).
    pub blueprint_config: Option<serde_json::Value>,
    /// Parent session ID for governed subagent depth tracking.
    pub parent_session_id: Option<everruns_contracts::typed_id::SessionId>,
    /// Explicit internal-only budget/delegation root for detached peers.
    pub budget_root_session_id: Option<everruns_contracts::typed_id::SessionId>,
    /// Internal id of an existing workspace to attach this session to. When
    /// `None`, `create_session` auto-creates a default 1:1 workspace whose id
    /// equals the new session id (the equality invariant). When `Some`, the
    /// session attaches to that workspace and no new workspace is created.
    pub workspace_id: Option<Uuid>,
}

impl Default for CreateSessionRow {
    fn default() -> Self {
        Self {
            org_id: 0,
            source: everruns_platform::SessionSource::Api,
            app_id: None,
            endpoint_id: None,
            trigger_id: None,
            harness_id: None,
            agent_id: None,
            agent_version_id: None,
            agent_config_hash: None,
            virtual_user_id: None,
            playground_user_id: None,
            owner_principal_id: PrincipalId::new(),
            resolved_owner_user_id: None,
            title: None,
            locale: None,
            tags: Vec::new(),
            model_id: None,
            capabilities: serde_json::Value::Array(Vec::new()),
            tools: serde_json::Value::Array(Vec::new()),
            mcp_servers: serde_json::Value::Object(serde_json::Map::new()),
            system_prompt: None,
            initial_files: serde_json::Value::Array(Vec::new()),
            hints: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            blueprint_id: None,
            blueprint_config: None,
            parent_session_id: None,
            budget_root_session_id: None,
            workspace_id: None,
        }
    }
}

/// Ordering for the sessions list. The chat thread list wants last activity;
/// the operational list wants creation order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SessionListOrder {
    #[default]
    CreatedAt,
    LastActivity,
}

/// Filter predicate shared by the sessions list and its facet aggregates
/// (EVE-852). Both read the same struct so a count can never describe a
/// different population than the page it annotates.
#[derive(Debug, Clone, Default)]
pub struct SessionListFilters {
    pub playground_user_id: Option<VirtualUserId>,
    pub archived_only: bool,
    pub agent_id: Option<AgentId>,
    pub search: Option<String>,
    /// Empty means "any source".
    pub sources: Vec<everruns_platform::SessionSource>,
    /// Empty means "any activity".
    pub activities: Vec<everruns_platform::SessionActivity>,
    /// Restrict to sessions whose resolved human owner is this user (`mine`).
    pub owner_user_id: Option<Uuid>,
    pub created_after: Option<DateTime<Utc>>,
    pub created_before: Option<DateTime<Utc>>,
    /// Widen the result set to archived sessions too. Default `false`: archive
    /// is a "put it away" bit, so hiding it is the point.
    pub include_archived: bool,
    pub order: SessionListOrder,
}
