// Sessions domain types.
//
// Decision: request/response DTOs are defined here, not in the HTTP layer, so
// the domain never imports `api`. The `api` module re-exports them, keeping
// OpenAPI schema names and JSON shapes unchanged.

use crate::kernel_imports::{
    ScopedMcpServers, SessionSeedMode, contracts::tool_types::ToolDefinition, is_mcp_tool,
};
use crate::records::{SandboxSelection, SessionParticipantKind, SessionParticipantRole};
use crate::storage::UpdateField;
use everruns_contracts::CapabilityRef as AgentCapabilityConfig;
use everruns_contracts::typed_id::{
    AgentId, HarnessId, ModelId, SessionId, VirtualUserId, WorkspaceId,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

// `CreateSessionRequest` keeps its own file; it shares this module's imports.
#[path = "create_request.rs"]
mod create_request;
pub use create_request::CreateSessionRequest;

/// Request to fork a session. Every field is
/// optional; omitted fields inherit the parent session's value. Title defaults
/// to "{parent title} (fork)" when omitted.
#[derive(Debug, Clone, Default, Deserialize, ToSchema, serde::Serialize)]
pub struct ForkSessionRequest {
    /// Title for the fork. Defaults to "{parent title} (fork)".
    #[serde(default)]
    #[schema(example = "Branch: try the async rewrite")]
    pub title: Option<String>,
    /// Goal for the fork. Omitted inherits the parent's goal.
    #[serde(default)]
    #[schema(example = "Try the async rewrite from this state")]
    pub goal: Option<String>,
    /// Tags for the fork. Replaces (does not merge with) the parent's tags.
    #[serde(default)]
    #[schema(example = json!(["experiment"]))]
    pub tags: Option<Vec<String>>,
    /// Override the LLM model for the fork.
    #[serde(default)]
    #[schema(value_type = Option<String>, example = "model_01933b5a00007000800000000000001")]
    pub model_id: Option<ModelId>,
    /// Override the agent assigned to the fork.
    #[serde(default)]
    #[schema(value_type = Option<String>, example = "agent_01933b5a00007000800000000000001")]
    pub agent_id: Option<AgentId>,
    /// Override the locale (BCP 47).
    #[serde(default)]
    #[schema(example = "uk-UA")]
    pub locale: Option<String>,
    /// Override the session-level system prompt.
    #[serde(default)]
    pub system_prompt: Option<String>,
}

// Trust boundary (client-side tools deprecation rollout): the `tools` field
// on session/agent create/update requests is documented as carrying only
// `client_side` definitions executed by the client, not the server. Pre-#1525
// the server silently accepted other shapes; #1525 turned the invariant into
// a hard 400. To give SDK/CLI consumers a migration window, we now drop any
// non-`client_side` entries during deserialization (with a `tracing::warn!`
// for ops visibility) instead of failing the request. Operators flip
// `EVERRUNS_REJECT_NON_CLIENT_SIDE_TOOLS=1` to opt back into hard-rejection
// once their clients are confirmed migrated. The migration timeline lives in
// `knowledge/execution/client-side-tools.md`.
fn deserialize_client_side_tools<'de, D>(deserializer: D) -> Result<Vec<ToolDefinition>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let tools = Vec::<ToolDefinition>::deserialize(deserializer)?;
    filter_or_reject_client_side_tools(tools).map_err(serde::de::Error::custom)
}

/// Apply the deprecation policy to a parsed `tools` array. Rejects
/// client-side definitions using the reserved MCP prefix so user-authored
/// metadata cannot shadow worker-executable MCP guardrail endpoints. By
/// default, drops every non-`client_side` entry with a structured warning.
/// When the env var `EVERRUNS_REJECT_NON_CLIENT_SIDE_TOOLS` is set to a
/// truthy value (`1`/`true`/`yes`), returns the original hard-reject error
/// instead.
pub(crate) fn filter_or_reject_client_side_tools(
    tools: Vec<ToolDefinition>,
) -> Result<Vec<ToolDefinition>, &'static str> {
    if tools.iter().any(|tool| {
        matches!(tool, ToolDefinition::ClientSide(client_tool) if is_mcp_tool(&client_tool.name))
    }) {
        return Err("client_side tool names must not use the reserved mcp_ prefix");
    }

    let (kept, dropped): (Vec<_>, Vec<_>) = tools
        .into_iter()
        .partition(|tool| matches!(tool, ToolDefinition::ClientSide(_)));
    if dropped.is_empty() {
        return Ok(kept);
    }
    if reject_non_client_side_tools_enabled() {
        return Err("tools must contain only client_side definitions");
    }
    // Soft-warn surface for the deprecation window: log type names only, no
    // payloads, so request fields like prompts or arguments cannot leak into
    // logs from the request body. Dedupe + sort the kind labels so a request
    // with many non-`client_side` entries doesn't allocate a huge Vec or
    // amplify telemetry — the count carries the cardinality, not the labels.
    let mut dropped_kinds: Vec<&'static str> =
        dropped.iter().map(tool_definition_kind_name).collect();
    dropped_kinds.sort_unstable();
    dropped_kinds.dedup();
    tracing::warn!(
        target = "client_tools_deprecation",
        dropped_count = dropped.len(),
        dropped_kinds = ?dropped_kinds,
        "tools[] contained {} non-client_side definition(s); dropping them. \
         The server will reject these in a future release; migrate clients \
         now. See knowledge/execution/client-side-tools.md for the timeline.",
        dropped.len()
    );
    Ok(kept)
}

/// Return `true` when the operator has opted into the legacy hard-reject
/// behavior. Used to gate the deprecation window.
pub(crate) fn reject_non_client_side_tools_enabled() -> bool {
    matches!(
        std::env::var("EVERRUNS_REJECT_NON_CLIENT_SIDE_TOOLS")
            .ok()
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("1") | Some("true") | Some("yes") | Some("on")
    )
}

/// Log-safe kind label for a `ToolDefinition` variant. Returns the same
/// `snake_case` discriminator that serde uses on the wire, so dashboards can
/// pivot on `dropped_kinds` without parsing free-form text.
fn tool_definition_kind_name(tool: &ToolDefinition) -> &'static str {
    match tool {
        ToolDefinition::Builtin(_) => "builtin",
        ToolDefinition::ClientSide(_) => "client_side",
    }
}

/// Response from cancel turn endpoint
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CancelTurnResponse {
    /// Whether the cancellation was performed or was a no-op
    #[schema(example = "cancelled")]
    pub status: CancelStatus,
    /// Human-readable message
    #[schema(example = "Turn cancelled successfully")]
    pub message: String,
}

/// Status of the cancel operation
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CancelStatus {
    /// Turn was actively cancelled
    Cancelled,
    /// No turn was running, cancel was a no-op
    NoOp,
}

/// Request to update a session. Only provided fields will be updated.
#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
pub struct UpdateSessionRequest {
    /// Human-readable title for the session.
    #[serde(default)]
    #[schema(example = "Updated session title")]
    pub title: Option<String>,
    /// Updated session objective.
    #[serde(default)]
    #[schema(example = "Summarize the incident and list remediations")]
    pub goal: Option<String>,
    /// Optional resident virtual user used for unattended/background execution.
    #[serde(default, with = "crate::domains::change_history::update_field")]
    #[schema(
        value_type = Option<String>,
        example = "identity_01933b5a00007000800000000000001",
        nullable = true
    )]
    pub virtual_user_id: UpdateField<VirtualUserId>,
    /// Session locale (BCP 47, e.g. `uk-UA`).
    #[serde(default)]
    #[schema(example = "uk-UA")]
    pub locale: Option<String>,
    /// Tags for organizing and filtering sessions.
    #[serde(default)]
    #[schema(example = json!(["resolved"]))]
    pub tags: Option<Vec<String>>,
}

/// Request to add a participant to a session.
#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
pub struct AddSessionParticipantRequest {
    /// Participant kind to add.
    pub kind: SessionParticipantKind,
    /// Agent to add when `kind` is `agent`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "agent_01933b5a00007000800000000000001")]
    pub agent_id: Option<AgentId>,
    /// Participant role. Omit for ordinary members. Host assignment is managed by session creation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<SessionParticipantRole>,
}

/// Query parameters for listing sessions with pagination.
#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListSessionsQuery {
    /// Exclude the permanent Chat from side-conversation pages.
    #[serde(
        default,
        deserialize_with = "crate::domains::common::deserialize_opt_bool_lenient"
    )]
    pub side_chats_only: Option<bool>,
    /// Filter by the fixed Playground end-user identity.
    #[param(value_type = Option<String>)]
    pub playground_user_id: Option<VirtualUserId>,
    /// Return only archived sessions.
    #[serde(
        default,
        deserialize_with = "crate::domains::common::deserialize_opt_bool_lenient"
    )]
    pub archived_only: Option<bool>,
    /// Filter sessions by agent ID.
    #[param(value_type = Option<String>, example = "agent_01933b5a00007000800000000000001")]
    pub agent_id: Option<AgentId>,
    /// Search by title (case-insensitive substring match).
    pub search: Option<String>,
    /// Comma-separated session sources: `chat`, `api`, `slack`, `ag_ui`,
    /// `fcp`, `schedule`, `webhook`, `a2a`, `eval`, `subagent`, `unknown`.
    #[param(example = "chat")]
    pub source: Option<String>,
    /// Comma-separated derived statuses: `running`, `paused`, `failed`,
    /// `completed`, `idle`.
    #[param(example = "running,failed")]
    pub status: Option<String>,
    /// Restrict to sessions owned by the calling user.
    #[param(example = true)]
    pub mine: Option<bool>,
    /// Include archived sessions. Defaults to false.
    #[param(example = true)]
    pub include_archived: Option<bool>,
    /// Inclusive lower bound on creation time (RFC 3339).
    #[param(example = "2026-08-01T00:00:00Z")]
    pub created_after: Option<String>,
    /// Exclusive upper bound on creation time (RFC 3339).
    #[param(example = "2026-08-09T00:00:00Z")]
    pub created_before: Option<String>,
    /// `created_at` (default) or `last_activity`.
    #[param(example = "last_activity")]
    pub order: Option<String>,
    /// Number of items to skip (for pagination).
    #[param(minimum = 0, default = 0)]
    pub offset: Option<u32>,
    /// Maximum number of items to return (for pagination).
    #[param(minimum = 1, maximum = 100, default = 20)]
    pub limit: Option<u32>,
}

/// One bucket of a sessions facet dimension.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SessionFacetCount {
    /// The dimension value: an activity, a source, or an agent's public id.
    #[schema(example = "running")]
    pub value: String,
    pub count: u64,
}

/// Facet-rail counts and masthead metrics for the sessions surface (EVE-852).
///
/// Every count is aggregated server-side over the same filter predicate as
/// `GET /v1/sessions`, so a client never derives them by paging the list. Each
/// facet dimension is counted with the other filters applied but its own
/// selection excluded, which is what lets the rail offer multi-select.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SessionFacetsResponse {
    /// Sessions matching every applied filter.
    pub total: u64,
    /// Counts per derived activity (`running`, `paused`, `failed`,
    /// `completed`, `idle`).
    pub by_activity: Vec<SessionFacetCount>,
    /// Counts per session source.
    pub by_source: Vec<SessionFacetCount>,
    /// Counts per agent, keyed by the agent's public id. Sessions with no
    /// agent are omitted.
    pub by_agent: Vec<SessionFacetCount>,
    /// Sessions executing a turn or awaiting client tool results right now.
    pub active_now: u64,
    /// Sessions whose most recent turn failed or was cancelled today (UTC).
    pub failed_today: u64,
    /// 95th percentile session duration over the filtered set, milliseconds.
    pub p95_duration_ms: u64,
    /// Tokens consumed by sessions created today (UTC).
    pub tokens_today: u64,
}

/// Response for session statistics endpoint
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SessionStatsResponse {
    /// Total number of sessions across all statuses
    pub total: u32,
    /// Sessions with a turn currently running
    pub active: u32,
    /// Sessions waiting for next input
    pub idle: u32,
    /// Sessions just created, no turn executed yet
    pub started: u32,
    /// Sessions waiting for client-side tool results
    pub waiting_for_tool_results: u32,
}
