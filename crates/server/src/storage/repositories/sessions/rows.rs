// Session rows (instance of the agentic loop).
//
// Why `CreateSessionRow` has a `Default`:
//
// The struct has thirty fields and most callers care about three or four.
// Spelling all thirty at every construction site made adding a column a
// mechanical edit across dozens of files that had no opinion about it —
// `sessions.trigger_id` (EVE-1138) touched ninety-six of them — and pushed
// several files that are already on the size debt list further up it.
//
// Every field defaults to its inert value: no agent, no ingress, no workspace,
// no blueprint, no capabilities. The one field with no meaningful zero is
// `owner_principal_id`; it gets a freshly generated principal id rather than
// the nil uuid, so a caller that forgets to set an owner produces a session
// owned by a principal that does not exist — which fails a foreign key — in
// preference to one that silently joins whatever nil-owned rows exist.
//
// A new column therefore defaults to "absent" without anyone opting in. That
// is the right default for an attribution or ingress pointer, which is what
// these columns are; a column that must be set deliberately should be added
// to the *request* types instead, where the compiler still demands it.
//
// `CreateSessionRequest` carries a `Default` for the same
// reason, and it is sound there for a stronger one: every field already has
// `#[serde(default)]`, so `Default` is exactly what a `{}` body deserializes
// to. That rationale lives here rather than on the struct because utoipa
// publishes a doc comment as the schema `description`, and Rust's `Default`
// means nothing to an API caller reading the OpenAPI spec.

use crate::kernel_imports::contracts::typed_id::{
    AgentId, HarnessId, ModelId, PrincipalId, SessionId, VirtualUserId,
};
use crate::storage::UpdateField;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow)]
pub struct SessionRow {
    pub id: SessionId,
    pub org_id: i64,
    /// Workspace this session is attached to (owns the virtual filesystem).
    /// `#[sqlx(default)]` so projections that don't select it (e.g. stats) still
    /// decode; all session-detail/list queries select it explicitly.
    #[sqlx(default)]
    pub workspace_id: Uuid,
    #[sqlx(default)]
    pub app_id: Option<Uuid>,
    /// Endpoint whose ingress created this session (EVE-1004). `app_id` says
    /// which bundle; this says which door. NULL for user, API, and
    /// platform-created sessions, and for app-channel sessions predating the
    /// routing tag the backfill reads.
    #[sqlx(default)]
    pub channel_id: Option<Uuid>,
    /// Agent trigger whose ingress created this session (EVE-1138). The
    /// structural successor to the `app_channel:` tag for budget attribution:
    /// migration 138 deleted the endpoint rows a webhook trigger's budgets had
    /// been keyed on, leaving the tag as the only identifier until 153.
    #[sqlx(default)]
    pub trigger_id: Option<Uuid>,
    #[sqlx(default)]
    pub harness_id: Option<HarnessId>,
    pub agent_id: Option<AgentId>,
    /// Revision of the agent's history the session started on.
    #[sqlx(default)]
    pub agent_revision: Option<i64>,
    #[sqlx(default)]
    pub virtual_user_id: Option<VirtualUserId>,
    #[sqlx(default)]
    pub playground_user_id: Option<VirtualUserId>,
    pub owner_principal_id: PrincipalId,
    #[sqlx(default)]
    pub resolved_owner_user_id: Option<Uuid>,
    pub title: Option<String>,
    #[sqlx(default)]
    pub goal: Option<String>,
    #[sqlx(default)]
    pub locale: Option<String>,
    pub tags: Vec<String>,
    pub model_id: Option<ModelId>,
    /// Session-level capabilities (JSONB in DB)
    #[sqlx(default)]
    pub capabilities: serde_json::Value,
    /// Client-side tools (JSONB in DB)
    #[sqlx(default)]
    pub tools: serde_json::Value,
    /// Scoped MCP server configs (JSONB in DB)
    #[sqlx(default)]
    pub mcp_servers: serde_json::Value,
    /// Session-level system prompt override
    #[sqlx(default)]
    pub system_prompt: Option<String>,
    /// Session-level initial files (JSONB in DB)
    #[sqlx(default)]
    pub initial_files: serde_json::Value,
    /// Session-level client hints (JSONB in DB, nullable)
    #[sqlx(default)]
    pub hints: Option<serde_json::Value>,
    /// Network access list (JSONB in DB, nullable)
    #[sqlx(default)]
    pub network_access: Option<serde_json::Value>,
    /// Maximum iterations per turn
    #[sqlx(default)]
    pub max_iterations: Option<i32>,
    /// Request-level parallel tool calling preference (EVE-598)
    #[sqlx(default)]
    pub parallel_tool_calls: Option<bool>,
    pub status: String,
    /// How the session was started (EVE-852). Stored as the closed-set string
    /// backing `crate::records::SessionSource`.
    #[sqlx(default)]
    pub source: String,
    /// Denormalized outcome of the most recent terminal turn: `completed`,
    /// `failed`, `cancelled`, or `None` when no turn has finished yet.
    #[sqlx(default)]
    pub last_turn_status: Option<String>,
    #[sqlx(default)]
    pub last_turn_at: Option<DateTime<Utc>>,
    /// Generated one-sentence description of what the run did (EVE-867).
    /// `None` whenever no summary exists, which is the resting state for chat
    /// threads and for deployments with no utility LLM.
    #[sqlx(default)]
    pub run_summary: Option<String>,
    /// `events.sequence` of the terminal turn `run_summary` describes. Fences a
    /// late out-of-band write for an older turn.
    #[sqlx(default)]
    pub run_summary_turn_sequence: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    /// Cumulative input tokens for all LLM calls in this session
    #[sqlx(default)]
    pub total_input_tokens: i64,
    /// Cumulative output tokens for all LLM calls in this session
    #[sqlx(default)]
    pub total_output_tokens: i64,
    /// Cumulative cache read tokens for all LLM calls in this session
    #[sqlx(default)]
    pub total_cache_read_tokens: i64,
    /// Cumulative cache creation tokens for all LLM calls in this session
    #[sqlx(default)]
    pub total_cache_creation_tokens: i64,
    /// Denormalized count of turn.completed, turn.failed, and turn.cancelled events
    #[sqlx(default)]
    pub turn_count: i64,
    /// Denormalized count of tool.completed events
    #[sqlx(default)]
    pub tool_call_count: i64,
    /// Live events in the session (EVE-868), derived from `event_sequences` (191).
    /// Backs the Events tab badge; `#[sqlx(default)]` because most SELECTs don't project it.
    #[sqlx(default)]
    pub event_count: i64,
    /// Denormalized count of `session_tasks` rows owned by this session
    /// (EVE-868). Backs the Work tab badge.
    #[sqlx(default)]
    pub task_count: i64,
    /// Denormalized count of non-directory files in this session's workspace
    /// (EVE-868). Lives on `workspaces`, so session-detail SELECTs project it
    /// through a join and everything else leaves it at zero.
    #[sqlx(default)]
    pub workspace_file_count: i64,
    /// Cumulative provider-reported actual cost in USD for this session
    #[sqlx(default)]
    pub total_actual_cost_usd: f64,
    /// Cumulative price-table estimated cost in USD for this session
    #[sqlx(default)]
    pub total_estimated_cost_usd: f64,
    /// Cumulative best-effort cost in USD for this session (actual where present,
    /// else estimated)
    #[sqlx(default)]
    pub total_cost_usd: f64,
    // -- Subagent nesting fields --
    #[sqlx(default)]
    pub parent_session_id: Option<SessionId>,
    /// Root of this session's delegation tree (EVE-680). A top-level session is
    /// its own root; a subagent child inherits its parent's root. Denormalized
    /// so a whole tree is one indexed query. Set by the storage layer at
    /// creation; `#[sqlx(default)]` because most SELECTs don't project it.
    #[sqlx(default)]
    pub root_session_id: Option<SessionId>,
    // -- Fork lineage fields (knowledge/runtime-resources/forking-sessions.md) --
    #[sqlx(default)]
    pub forked_from_session_id: Option<SessionId>,
    #[sqlx(default)]
    pub forked_from_sequence: Option<i32>,
    // -- Blueprint fields --
    #[sqlx(default)]
    pub blueprint_id: Option<String>,
    #[sqlx(default)]
    pub blueprint_config: Option<serde_json::Value>,
    /// When the session was archived; `None` means active. See migration 124.
    #[sqlx(default)]
    pub archived_at: Option<DateTime<Utc>>,
}

/// One bucket of a facet rail dimension.
#[derive(Debug, Clone, FromRow)]
pub struct SessionFacetBucket {
    pub value: String,
    pub count: i64,
}

/// Masthead metrics and facet-rail counts for the sessions surface.
#[derive(Debug, Clone, Default)]
pub struct SessionFacetsRow {
    pub total: i64,
    pub by_activity: Vec<SessionFacetBucket>,
    pub by_source: Vec<SessionFacetBucket>,
    /// `value` holds the agent's public id; `NULL` agents are omitted.
    pub by_agent: Vec<SessionFacetBucket>,
    pub active_now: i64,
    pub failed_today: i64,
    pub p95_duration_ms: i64,
    pub tokens_today: i64,
}

#[derive(Debug, Clone, Default, FromRow)]
pub struct SessionMastheadRow {
    pub active_now: i64,
    pub failed_today: i64,
    pub p95_duration_ms: i64,
    pub tokens_today: i64,
}

#[derive(Debug, Clone, Default, FromRow)]
pub struct SessionAggregateStatsRow {
    pub session_count: i64,
    pub active_session_count: i64,
    pub idle_session_count: i64,
    pub started_session_count: i64,
    pub waiting_for_tool_results_session_count: i64,
    pub execution_count: i64,
    pub total_session_duration_ms: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cache_read_tokens: i64,
    pub total_cache_creation_tokens: i64,
    pub total_actual_cost_usd: f64,
    pub total_estimated_cost_usd: f64,
    pub total_cost_usd: f64,
    pub first_session_at: Option<DateTime<Utc>>,
    pub last_session_at: Option<DateTime<Utc>>,
    pub last_execution_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateSession {
    pub harness_id: Option<HarnessId>,
    pub title: Option<String>,
    pub goal: Option<String>,
    pub virtual_user_id: UpdateField<VirtualUserId>,
    pub owner_principal_id: Option<PrincipalId>,
    pub resolved_owner_user_id: UpdateField<Uuid>,
    pub locale: Option<String>,
    pub tags: Option<Vec<String>>,
    pub model_id: Option<ModelId>,
    pub status: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub tools: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct CreateSessionRow {
    pub org_id: i64,
    /// How this session was started. Set by the creating ingress path, never
    /// taken from untrusted client input except for client-declarable
    /// variants (see `SessionSource::is_client_declarable`).
    pub source: crate::records::SessionSource,
    pub app_id: Option<Uuid>,
    /// The two ingress pointers (EVE-1004, EVE-1138): the endpoint, set by the
    /// app-channel paths that all know theirs, and the trigger, set by the
    /// trigger invocation path. Both `None` for every other way in.
    pub channel_id: Option<Uuid>,
    pub trigger_id: Option<Uuid>,
    pub harness_id: Option<HarnessId>,
    pub agent_id: Option<AgentId>,
    /// Revision of the agent's history the session starts on.
    pub agent_revision: Option<i64>,
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
            source: crate::records::SessionSource::Api,
            app_id: None,
            channel_id: None,
            trigger_id: None,
            harness_id: None,
            agent_id: None,
            agent_revision: None,
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
    pub side_chats_only: bool,
    pub playground_user_id: Option<VirtualUserId>,
    pub archived_only: bool,
    pub agent_id: Option<AgentId>,
    pub search: Option<String>,
    /// Empty means "any source".
    pub sources: Vec<crate::records::SessionSource>,
    /// Empty means "any activity".
    pub activities: Vec<crate::records::SessionActivity>,
    /// Restrict to sessions whose resolved human owner is this user (`mine`).
    pub owner_user_id: Option<Uuid>,
    pub created_after: Option<DateTime<Utc>>,
    pub created_before: Option<DateTime<Utc>>,
    /// Widen the result set to archived sessions too. Default `false`: archive
    /// is a "put it away" bit, so hiding it is the point.
    pub include_archived: bool,
    pub order: SessionListOrder,
}
