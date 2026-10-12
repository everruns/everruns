// Agent and agent capability rows (configuration for the agentic loop).

use crate::domains::agents::record::{Agent, AgentStatus};
use crate::kernel_imports::contracts::typed_id::{AgentId, HarnessId, ModelId, VirtualUserId};
use chrono::{DateTime, Utc};
use everruns_core::conversation::Communication;
use everruns_core::{InitialFile, TokenUsage};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct AgentRow {
    pub id: AgentId,
    pub public_id: String,
    pub org_id: i64,
    pub name: String,
    #[sqlx(default)]
    pub display_name: Option<String>,
    pub description: Option<String>,
    /// Markdown intro for fresh Platform Chat threads. Wins over the harness
    /// intro. Hidden once the user inputs.
    #[sqlx(default)]
    pub intro_markdown: Option<String>,
    /// One-line description in simplified Markdown. Wins over the harness value.
    #[sqlx(default)]
    pub short_description: Option<String>,
    /// Conversation starters (JSONB in DB). Win over harness starters when
    /// non-empty.
    #[sqlx(default)]
    pub starters: serde_json::Value,
    pub system_prompt: String,
    pub default_model_id: Option<ModelId>,
    pub harness_id: HarnessId,
    /// Whether `harness_id` was explicitly selected or materialized from the
    /// organization default for backward-compatible API responses.
    #[sqlx(default)]
    pub harness_source: String,
    /// Lazily-created identity principal subject for this agent (EVE-758).
    /// NULL until the agent first acts unattended (e.g. an agent trigger fire),
    /// at which point an `virtual_users` row is created and linked so the
    /// agent owns its unattended sessions as itself. Storage-only: intentionally
    /// not surfaced on the public `crate::domains::agents::record::Agent` API.
    #[sqlx(default)]
    pub virtual_user_id: Option<VirtualUserId>,
    #[sqlx(default)]
    pub forked_from_agent_id: Option<AgentId>,
    #[sqlx(default)]
    pub root_agent_id: Option<AgentId>,
    pub tags: Vec<String>,
    pub status: String,
    /// Incident switch: when true no endpoint on this agent accepts traffic,
    /// without rewriting the per-endpoint status it must restore to (EVE-1007).
    pub exposures_suspended: bool,
    /// Platform-supplied agent (mirrors `HarnessRow::is_built_in`). Its
    /// definition is immutable through the API and excluded from the per-org
    /// agent limit; bindings around it stay editable.
    #[sqlx(default)]
    pub is_built_in: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
    /// Starter files copied into new sessions (JSONB in DB)
    #[sqlx(default)]
    pub initial_files: serde_json::Value,
    /// Client-side tools (JSONB in DB)
    #[sqlx(default)]
    pub tools: serde_json::Value,
    /// Scoped MCP server configs (JSONB in DB)
    #[sqlx(default)]
    pub mcp_servers: serde_json::Value,
    /// Network access list (JSONB in DB, nullable)
    #[sqlx(default)]
    pub network_access: Option<serde_json::Value>,
    /// Maximum iterations per turn
    #[sqlx(default)]
    pub max_iterations: Option<i32>,
    /// Request-level parallel tool calling preference (EVE-598)
    #[sqlx(default)]
    pub parallel_tool_calls: Option<bool>,
    /// How the agent talks: `direct` or `explicit`.
    #[sqlx(default)]
    pub communication: String,
    #[sqlx(default)]
    pub environments: Option<serde_json::Value>,
    /// Current avatar (`agent_avatars.id`), `None` when the agent has none.
    #[sqlx(default)]
    pub avatar_id: Option<Uuid>,
    /// Cumulative input tokens across all sessions
    #[sqlx(default)]
    pub total_input_tokens: i64,
    /// Cumulative output tokens across all sessions
    #[sqlx(default)]
    pub total_output_tokens: i64,
    /// Cumulative cache read tokens across all sessions
    #[sqlx(default)]
    pub total_cache_read_tokens: i64,
    /// Cumulative cache creation tokens across all sessions
    #[sqlx(default)]
    pub total_cache_creation_tokens: i64,
    /// Cumulative provider-reported actual cost in USD across all sessions
    #[sqlx(default)]
    pub total_actual_cost_usd: f64,
    /// Cumulative price-table estimated cost in USD across all sessions
    #[sqlx(default)]
    pub total_estimated_cost_usd: f64,
    /// Cumulative best-effort cost in USD across all sessions (actual if present, else estimated)
    #[sqlx(default)]
    pub total_cost_usd: f64,
}

#[derive(Debug, Clone)]
pub struct CreateAgentRow {
    pub public_id: String,
    pub name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    /// Markdown intro for fresh Platform Chat threads (agent wins).
    pub intro_markdown: Option<String>,
    /// One-line description in simplified Markdown (agent wins).
    pub short_description: Option<String>,
    /// Conversation starters (JSONB in DB, agent wins when non-empty).
    pub starters: serde_json::Value,
    pub system_prompt: String,
    pub default_model_id: Option<ModelId>,
    pub harness_id: HarnessId,
    pub tags: Vec<String>,
    /// Starter files copied into new sessions (JSONB in DB)
    pub initial_files: serde_json::Value,
    /// Client-side tools (JSONB in DB)
    pub tools: serde_json::Value,
    /// Scoped MCP server configs (JSONB in DB)
    pub mcp_servers: serde_json::Value,
    /// Network access list (JSONB in DB)
    pub network_access: Option<serde_json::Value>,
    /// Maximum iterations per turn
    pub max_iterations: Option<i32>,
    /// Request-level parallel tool calling preference (EVE-598)
    pub parallel_tool_calls: Option<bool>,
    /// How the agent talks.
    pub communication: Communication,
    pub environments: Option<serde_json::Value>,
    /// Platform-supplied agent. Only org bootstrap sets this; every API-facing
    /// creation path leaves it false.
    pub is_built_in: bool,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateAgent {
    pub virtual_user_id: Option<Option<VirtualUserId>>,
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    /// Markdown intro for fresh Platform Chat threads (agent wins).
    pub intro_markdown: Option<Option<String>>,
    /// One-line description in simplified Markdown (agent wins).
    pub short_description: Option<Option<String>>,
    /// Conversation starters (JSONB); None = leave unchanged.
    pub starters: Option<serde_json::Value>,
    pub system_prompt: Option<String>,
    pub default_model_id: Option<ModelId>,
    pub harness_id: Option<HarnessId>,
    pub harness_source: Option<String>,
    pub forked_from_agent_id: Option<AgentId>,
    pub root_agent_id: Option<AgentId>,
    pub tags: Option<Vec<String>>,
    pub status: Option<String>,
    /// Agent-level exposure incident switch (EVE-1007).
    pub exposures_suspended: Option<bool>,
    pub initial_files: Option<serde_json::Value>,
    pub tools: Option<serde_json::Value>,
    pub mcp_servers: Option<serde_json::Value>,
    pub network_access: Option<Option<serde_json::Value>>,
    /// None = don't change, Some(None) = set to NULL, Some(Some(v)) = set to v
    pub max_iterations: Option<Option<i32>>,
    /// Request-level parallel tool calling preference (EVE-598).
    /// None = don't change, Some(None) = set to NULL, Some(Some(v)) = set to v
    pub parallel_tool_calls: Option<Option<bool>>,
    /// How the agent talks. None = don't change.
    pub communication: Option<Communication>,
    pub environments: Option<Option<serde_json::Value>>,
}

#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct AgentCapabilityRow {
    pub id: Uuid,
    pub agent_id: AgentId,
    pub capability_id: String,
    pub position: i32,
    /// Per-agent capability configuration (JSON)
    pub config: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateAgentCapabilityRow {
    pub agent_id: AgentId,
    pub capability_id: String,
    pub position: i32,
    /// Per-agent capability configuration (JSON)
    #[allow(dead_code)]
    pub config: serde_json::Value,
}

/// The one mapping from an agent row to the `Agent` record. Capabilities are
/// loaded separately (`agent_capabilities`) and passed in.
pub fn row_to_agent(row: AgentRow, capabilities: Vec<everruns_contracts::CapabilityRef>) -> Agent {
    let usage = if row.total_input_tokens > 0 || row.total_output_tokens > 0 {
        // Actual and estimated cost totals are tracked separately; the aggregate
        // carries each so consumers can prefer actual and reconcile drift.
        Some(
            TokenUsage::with_cache(
                row.total_input_tokens as u32,
                row.total_output_tokens as u32,
                if row.total_cache_read_tokens > 0 {
                    Some(row.total_cache_read_tokens as u32)
                } else {
                    None
                },
                if row.total_cache_creation_tokens > 0 {
                    Some(row.total_cache_creation_tokens as u32)
                } else {
                    None
                },
            )
            .with_cost(
                (row.total_actual_cost_usd > 0.0).then_some(row.total_actual_cost_usd),
                (row.total_estimated_cost_usd > 0.0).then_some(row.total_estimated_cost_usd),
            )
            .with_effective_cost((row.total_cost_usd > 0.0).then_some(row.total_cost_usd)),
        )
    } else {
        None
    };

    let public_id: AgentId = row
        .public_id
        .parse()
        .unwrap_or_else(|_| AgentId::from_uuid(row.id.uuid()));

    Agent {
        service_virtual_user_id: row.virtual_user_id,
        public_id,
        internal_id: row.id.uuid(),
        name: row.name,
        display_name: row.display_name,
        description: row.description,
        intro_markdown: row.intro_markdown,
        short_description: row.short_description,
        starters: serde_json::from_value::<
            Vec<crate::domains::harnesses::record::ConversationStarter>,
        >(row.starters)
        .unwrap_or_default(),
        avatar: row
            .avatar_id
            .map(crate::domains::agents::record::AgentAvatar::from_uuid),
        system_prompt: row.system_prompt,
        default_model_id: row.default_model_id,
        harness_id: row.harness_id,
        forked_from_agent_id: row.forked_from_agent_id,
        root_agent_id: row.root_agent_id,
        tags: row.tags,
        is_built_in: row.is_built_in,
        capabilities,
        sandbox_policy: row
            .environments
            .and_then(|value| serde_json::from_value(value).ok()),
        initial_files: serde_json::from_value::<Vec<InitialFile>>(row.initial_files)
            .unwrap_or_default(),
        mcp_servers: serde_json::from_value(row.mcp_servers).unwrap_or_default(),
        network_access: row
            .network_access
            .and_then(|v| serde_json::from_value(v).ok()),
        max_iterations: crate::max_iterations::from_db(row.max_iterations),
        parallel_tool_calls: row.parallel_tool_calls,
        communication: Communication::from_str_opt(&row.communication).unwrap_or_default(),
        tools: serde_json::from_value(row.tools).unwrap_or_default(),
        status: AgentStatus::from(row.status.as_str()),
        // `exposed` is derived from the endpoint rows, which this row-level
        // mapping cannot see. Callers that surface it use
        // `with_derived_exposure`.
        exposures_suspended: row.exposures_suspended,
        exposed: false,
        created_at: row.created_at,
        updated_at: row.updated_at,
        archived_at: row.archived_at,
        deleted_at: row.deleted_at,
        usage,
    }
}
