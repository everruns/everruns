// Session CRUD HTTP routes
// Routes use ResolvedOrg: org derived from auth context (API key or cookie)
// Policy enforcement happens at the service layer via #[policy] macro.

use crate::auth::{AuthState, ResolvedOrg, rate_limit::OrgRateLimiter};
use crate::domains::common::{Command, Ctx};
use crate::domains::sessions::{
    AddSessionParticipant, ArchiveSession, CancelSession, CreateSession, DeleteSession,
    ForkSession, GetOrCreateChatSession, GetSession, GetSessionContextReport, GetSessionFacets,
    GetSessionStats, LeaveSessionParticipant, ListSessionParticipants, ListSessions, PinSession,
    SESSION_MANAGE, SESSION_VIEW, SessionFilterArgs, SessionService, UnarchiveSession,
    UnpinSession, UpdateSessionCmd,
};
use crate::kernel_imports::{
    Caller, ResourceConfigResponse, ScopedMcpServers, SessionContextReport, SessionSeedMode,
    evaluate_policies_with, everruns_provider::tool_types::ToolDefinition, is_mcp_tool,
};
use crate::services::EventService;
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_capability::CapabilityRef as AgentCapabilityConfig;
use everruns_host::HostComposition;
use everruns_platform::BuiltInHarnessRole;
use everruns_platform::{
    Session, SessionParticipant, SessionParticipantKind, SessionParticipantRole,
};
use everruns_provider::typed_id::{
    AgentId, AgentIdentityId, HarnessId, ModelId, SessionId, WorkspaceId,
};
use everruns_worker::AgentRunner;

use super::common::{
    ApiResult, ApiResultExt, ErrorResponse, PaginatedResponse, UrlBuilder, WithUrls,
    deserialize_nullable_update_field, impl_auth_state,
};
use everruns_durable::UpdateField;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

/// Request to create a session
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateSessionRequest {
    /// How this session was started. Clients may declare only `chat` (an
    /// interactive thread) or `api` (the default); every other source is
    /// server-owned so the sessions facet rail stays trustworthy.
    #[serde(default)]
    #[schema(value_type = Option<String>, example = "chat")]
    pub source: Option<everruns_platform::SessionSource>,
    /// ID of the harness for this session (format: harness_{32-hex}).
    /// If omitted, the harness is derived from the agent (when one is supplied),
    /// else the org default harness, else the built-in fallback. New orgs default
    /// that to Generic. Mutually exclusive with `harness_name`.
    #[serde(default)]
    #[schema(value_type = Option<String>, example = "harness_01933b5a00007000800000000000001")]
    pub harness_id: Option<HarnessId>,
    /// Harness name (e.g. "generic", "deep-research").
    /// Alternative to `harness_id` — looked up by name within the org.
    /// Mutually exclusive with `harness_id`.
    #[serde(default)]
    #[schema(example = "generic")]
    pub harness_name: Option<String>,
    /// ID of the agent to work in this session (optional, format: agent_{32-hex}).
    /// When supplied without a harness, the session inherits the agent's harness.
    /// Mutually exclusive with `agent_name`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "agent_01933b5a00007000800000000000001")]
    pub agent_id: Option<AgentId>,
    /// Name of the agent to work in this session (optional).
    /// Alternative to `agent_id` — looked up by name within the org.
    /// Mutually exclusive with `agent_id`.
    #[serde(default)]
    #[schema(example = "support")]
    pub agent_name: Option<String>,
    /// Optional resident agent identity used for unattended/background execution.
    #[serde(default)]
    #[schema(value_type = Option<String>, example = "identity_01933b5a00007000800000000000001")]
    pub agent_identity_id: Option<AgentIdentityId>,
    /// Human-readable title for the session.
    #[serde(default)]
    #[schema(example = "Debug login issue")]
    pub title: Option<String>,
    /// Optional objective for the session. Visible to the agent at system-prompt level.
    #[serde(default)]
    #[schema(example = "Investigate the queue latency regression and propose a fix")]
    pub goal: Option<String>,
    /// Session locale (BCP 47, e.g. `uk-UA`).
    #[serde(default)]
    #[schema(example = "uk-UA")]
    pub locale: Option<String>,
    /// Tags for organizing and filtering sessions.
    #[serde(default)]
    #[schema(example = json!(["debugging", "urgent"]))]
    pub tags: Vec<String>,
    /// The ID of the LLM model to use for this session.
    /// Overrides the agent's default model if specified.
    #[serde(default)]
    #[schema(value_type = Option<String>, example = "model_01933b5a00007000800000000000001")]
    pub model_id: Option<ModelId>,
    /// Session-level capabilities (additive to agent capabilities).
    /// Applied after agent capabilities when building RuntimeAgent.
    #[serde(default)]
    #[schema(
        value_type = Vec<everruns_platform::CapabilityRefSchema>,
        example = json!([{"ref": "current_time", "config": {}}, {"ref": "web_fetch", "config": {}}])
    )]
    pub capabilities: Vec<AgentCapabilityConfig>,
    /// Client-side tools for this session (additive to agent tools).
    /// These tools are sent to the LLM but executed by the client.
    #[serde(default, deserialize_with = "deserialize_client_side_tools")]
    #[schema(example = json!([{"type": "client_side", "name": "open_url", "description": "Open URL in the user's browser", "parameters": {"type": "object", "properties": {"url": {"type": "string"}}, "required": ["url"]}}]))]
    pub tools: Vec<ToolDefinition>,
    /// Remote MCP servers scoped to this session only.
    #[serde(default, rename = "mcpServers", alias = "mcp_servers")]
    pub mcp_servers: ScopedMcpServers,
    /// Optional session-level system prompt override.
    /// Prepended to the agent's system prompt when building RuntimeAgent.
    #[serde(default)]
    #[schema(
        example = "You are debugging a production incident. Be concise and cite log lines verbatim."
    )]
    pub system_prompt: Option<String>,
    /// Session-level initial files (additive to agent initial_files).
    /// Files with matching paths override agent/harness files; new paths are appended.
    #[serde(default)]
    #[schema(example = json!([{"path": "README.md", "content": "# Project notes\n"}]))]
    pub initial_files: Vec<everruns_core::InitialFile>,
    /// Session-level client hints — arbitrary key-value pairs that tell the
    /// server what the client can handle. These are defaults for every turn;
    /// per-message `controls.hints` override these key-by-key (shallow merge).
    #[serde(default)]
    #[schema(example = json!({"setup_connection": true, "rich_media": true}))]
    pub hints: Option<std::collections::HashMap<String, serde_json::Value>>,
    /// Network access list controlling which hosts/URLs this session can reach.
    /// Merged with harness and agent layers (allowed: intersect, blocked: union).
    /// Example shape is defined on `NetworkAccessList`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_access: Option<everruns_core::network_access::NetworkAccessList>,
    /// Maximum number of LLM iterations per turn for this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 20)]
    pub max_iterations: Option<usize>,
    /// Request-level parallel tool calling preference (EVE-598). `true` signals
    /// the provider that parallel tool calls are wanted; `false` requests at
    /// most one tool call per turn and forces serial execution. Omit to inherit
    /// the agent/harness preference or the provider default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = true)]
    pub parallel_tool_calls: Option<bool>,
    /// Internal: parent session for governed subagent depth tracking.
    /// Set by the worker when spawning a child session so nested delegation can
    /// be bounded by max_subagent_depth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(ignore)]
    pub parent_session_id: Option<SessionId>,
    /// Internal: lineage source when creating a detached peer session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(ignore)]
    pub forked_from_session_id: Option<SessionId>,
    /// Internal: org-validated budget root for detached peer sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(ignore)]
    pub budget_root_session_id: Option<SessionId>,
    /// Internal: how to seed a detached peer session from its lineage source.
    #[serde(default)]
    #[schema(ignore)]
    pub seed: SessionSeedMode,
    /// Attach this session to an existing Workspace (format: `wsp_<32-hex>`)
    /// instead of auto-creating a default per-session workspace. The workspace
    /// must exist in the caller's org and be `active`. Lets multiple sessions
    /// share one working filesystem. Omit for the default 1:1 behavior.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "wsp_01933b5a00007000800000000000001")]
    pub workspace_id: Option<WorkspaceId>,
}

/// Request to fork a session (knowledge/runtime-resources/forking-sessions.md). Every field is
/// optional; omitted fields inherit the parent session's value. Title defaults
/// to "{parent title} (fork)" when omitted.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
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
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateSessionRequest {
    /// Human-readable title for the session.
    #[serde(default)]
    #[schema(example = "Updated session title")]
    pub title: Option<String>,
    /// Updated session objective.
    #[serde(default)]
    #[schema(example = "Summarize the incident and list remediations")]
    pub goal: Option<String>,
    /// Optional resident agent identity used for unattended/background execution.
    #[serde(default, deserialize_with = "deserialize_nullable_update_field")]
    #[schema(
        value_type = Option<String>,
        example = "identity_01933b5a00007000800000000000001",
        nullable = true
    )]
    pub agent_identity_id: UpdateField<AgentIdentityId>,
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
#[derive(Debug, Clone, Deserialize, ToSchema)]
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

/// Request body for the `get_or_create_chat_session` operation.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GetOrCreateChatSessionRequest {
    /// Browser locale for seeding the global chat session (BCP 47, e.g. `uk-UA`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
}

/// Query parameters for listing sessions with pagination.
#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListSessionsQuery {
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

/// Query parameters for the sessions facet rail. Mirrors `ListSessionsQuery`
/// minus pagination, so counts and page always share a predicate.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SessionFacetsQuery {
    #[param(value_type = Option<String>, example = "agent_01933b5a00007000800000000000001")]
    pub agent_id: Option<AgentId>,
    pub search: Option<String>,
    pub source: Option<String>,
    pub status: Option<String>,
    pub mine: Option<bool>,
    pub include_archived: Option<bool>,
    pub created_after: Option<String>,
    pub created_before: Option<String>,
    pub order: Option<String>,
}

/// App state for sessions routes
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub session_service: Arc<SessionService>,
    pub event_service: EventService,
    pub runner: Arc<dyn AgentRunner>,
    pub auth: AuthState,
    pub fallback_default_harness_name: Option<String>,
    pub chat_harness_name: Option<String>,
    pub chat_session_title: Option<String>,
    pub org_rate_limiter: OrgRateLimiter,
}

impl AppState {
    pub fn new(db: Arc<StorageBackend>, runner: Arc<dyn AgentRunner>, auth: AuthState) -> Self {
        Self::with_host_composition(
            db,
            runner,
            auth,
            &crate::platform::oss_host_composition(),
            &crate::platform::oss_built_in_harnesses(),
            crate::event_delivery::EventDelivery::in_memory(),
        )
    }

    pub fn with_host_composition(
        db: Arc<StorageBackend>,
        runner: Arc<dyn AgentRunner>,
        auth: AuthState,
        host_composition: &HostComposition,
        built_in_harnesses: &[everruns_platform::BuiltInHarnessDefinition],
        event_delivery: crate::event_delivery::EventDelivery,
    ) -> Self {
        Self {
            session_service: Arc::new(SessionService::with_registry(
                db.clone(),
                (*host_composition.capability_registry()).clone(),
            )),
            event_service: EventService::new(db.clone(), event_delivery.clone()),
            db,
            runner,
            auth,
            fallback_default_harness_name: everruns_platform::harness_for_role(
                built_in_harnesses,
                BuiltInHarnessRole::Default,
            )
            .map(|h| h.name.clone()),
            chat_harness_name: everruns_platform::harness_for_role(
                built_in_harnesses,
                BuiltInHarnessRole::Chat,
            )
            .map(|h| h.name.clone()),
            chat_session_title: everruns_platform::harness_for_role(
                built_in_harnesses,
                BuiltInHarnessRole::Chat,
            )
            .map(|h| h.display_name.clone()),
            org_rate_limiter: OrgRateLimiter::default(),
        }
    }

    fn ctx(&self, org: &ResolvedOrg) -> Ctx {
        Ctx::minimal(
            Caller::from(org),
            self.db.clone(),
            None,
            self.auth.permission_resolver.clone(),
        )
        .with_feature_flags(org.feature_flags.clone())
        .with_session_service(self.session_service.clone())
        .with_event_service(Arc::new(self.event_service.clone()))
        .with_runner(self.runner.clone())
        .with_fallback_harness_name(self.fallback_default_harness_name.clone())
        .with_chat_harness_name(self.chat_harness_name.clone())
        .with_chat_session_title(self.chat_session_title.clone())
        .with_org_rate_limiter(self.org_rate_limiter.clone())
    }
}

impl_auth_state!(AppState);

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

/// Create session routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        // Config endpoint (must be before /{session_id} to avoid conflict)
        .route("/v1/sessions/config", get(session_config))
        // Global chat session (must be before /{session_id} to avoid conflict)
        .route("/v1/sessions/chat", post(get_or_create_chat_session))
        // Session facets (must be before /{session_id} to avoid conflict)
        .route("/v1/sessions/facets", get(get_session_facets))
        // Session stats (must be before /{session_id} to avoid conflict)
        .route("/v1/sessions/stats", get(get_session_stats))
        // Session CRUD
        .route("/v1/sessions", post(create_session).get(list_sessions))
        .route(
            "/v1/sessions/{session_id}/participants",
            get(list_session_participants).post(add_session_participant),
        )
        .route(
            "/v1/sessions/{session_id}/participants/{participant_id}",
            axum::routing::delete(leave_session_participant),
        )
        .route(
            "/v1/sessions/{session_id}",
            get(get_session)
                .patch(update_session)
                .delete(delete_session),
        )
        .route(
            "/v1/sessions/{session_id}/context-report",
            get(get_session_context_report),
        )
        .route(
            "/v1/sessions/{session_id}/resolved-model",
            get(get_session_resolved_model),
        )
        // Pin/unpin
        .route(
            "/v1/sessions/{session_id}/pin",
            axum::routing::put(pin_session).delete(unpin_session),
        )
        .route(
            "/v1/sessions/{session_id}/archive",
            axum::routing::put(archive_session).delete(unarchive_session),
        )
        // Cancel turn endpoint
        .route("/v1/sessions/{session_id}/cancel", post(cancel_turn))
        // Fork a session into an independent copy
        .route("/v1/sessions/{session_id}/fork", post(fork_session))
        .with_state(state)
}

/// GET /v1/sessions/config
pub async fn session_config(
    State(auth): State<AuthState>,
    org: ResolvedOrg,
) -> Json<ResourceConfigResponse> {
    let caller = Caller::from(&org);
    let policies = evaluate_policies_with(
        auth.permission_resolver.as_ref(),
        &caller,
        &[&SESSION_VIEW, &SESSION_MANAGE],
    );
    Json(ResourceConfigResponse { policies })
}

/// POST /v1/sessions - Create a new session
#[utoipa::path(
    post,
    path = "/v1/sessions",
    request_body(
        content = CreateSessionRequest,
        example = json!({
            "harness_name": "generic",
            "title": "Debug login issue",
            "tags": ["debugging", "urgent"]
        })
    ),
    responses(
        (
            status = 201,
            description = "Session created successfully",
            body = WithUrls<Session>,
            example = json!({
                "self_url": "https://app.everruns.com/api/v1/sessions/session_01933b5a00007000800000000000001",
                "view_url": "https://app.everruns.com/sessions/session_01933b5a00007000800000000000001/chat",
                "ui_link":  "https://app.everruns.com/sessions/session_01933b5a00007000800000000000001/chat",
                "id": "session_01933b5a00007000800000000000001",
                "status": "started",
                "organization_id": "org_00000000000000000000000000000001",
                "harness_id": "harness_01933b5a00007000800000000000001",
                "owner_principal_id": "principal_01933b5a000070008000000000000001",
                "title": "Debug login issue",
                "tags": ["debugging", "urgent"],
                "created_at": "2026-05-27T15:24:00Z",
                "updated_at": "2026-05-27T15:24:00Z"
            })
        ),
        (
            status = 404,
            description = "Harness, Agent, or Model not found",
            body = ErrorResponse,
            example = json!({
                "type": "https://docs.everruns.com/errors/harness_not_found",
                "title": "Not Found",
                "status": 404,
                "detail": "Harness 'generic' not found in org org_00000000000000000000000000000001.",
                "code": "harness_not_found"
            })
        ),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn create_session(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<WithUrls<Session>>), (StatusCode, Json<ErrorResponse>)> {
    let mut req = req;
    strip_internal_only_fields(&mut req);
    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    let session = CreateSession(req).run(&state.ctx(&org)).await?;

    Ok((StatusCode::CREATED, Json(urls.wrap(session))))
}

/// Strip request fields that only trusted internal dispatch paths may set.
///
/// Trusted worker dispatch sets these fields by invoking the domain command
/// directly. Values supplied at the public HTTP boundary are forgery attempts
/// and must never influence delegation ownership, lineage, or budget linkage.
fn strip_internal_only_fields(req: &mut CreateSessionRequest) {
    // THREAT[TM-TENANT-014]: Never let public callers select delegation or
    // budget ownership metadata.
    req.parent_session_id = None;
    req.forked_from_session_id = None;
    req.budget_root_session_id = None;
    req.seed = SessionSeedMode::Fresh;
}

/// POST /v1/sessions/{session_id}/fork - Fork a session into an independent copy
#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/fork",
    params(("session_id" = String, Path, description = "Session to fork")),
    request_body(
        content = ForkSessionRequest,
        example = json!({ "title": "Branch: try the async rewrite", "tags": ["experiment"] })
    ),
    responses(
        (status = 201, description = "Fork created successfully", body = WithUrls<Session>),
        (status = 404, description = "Parent session, agent, or harness not found", body = ErrorResponse),
        (status = 409, description = "Parent session is mid-turn and cannot be forked", body = ErrorResponse),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn fork_session(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    body: Option<Json<ForkSessionRequest>>,
) -> Result<(StatusCode, Json<WithUrls<Session>>), (StatusCode, Json<ErrorResponse>)> {
    let ctx = state.ctx(&org);

    // Authorize before consuming the org's session-create budget. Forking
    // creates a new session and deep-copies parent state, so successful forks
    // share the same per-org velocity limit as ordinary session creation.
    if let Some(policy) = ForkSession::policy() {
        policy
            .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
            .map_err(|e| crate::domains::common::CommandError::forbidden(e.message))?;
    }

    // Per-org session-create throttle is enforced inside `ForkSession::execute`
    // (shared across REST and MCP dispatch), so no separate pre-check here.

    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    let session = ForkSession {
        session_id,
        overrides: body.map(|Json(b)| b).unwrap_or_default(),
    }
    .run(&ctx)
    .await?;

    Ok((StatusCode::CREATED, Json(urls.wrap(session))))
}

/// POST /v1/sessions/chat - Get or create global chat session
///
/// Returns the user's singleton global chat session. Creates one if it doesn't exist.
/// Uses the Platform Chat harness and tags for per-user singleton management.
#[utoipa::path(
    post,
    path = "/v1/sessions/chat",
    request_body = Option<GetOrCreateChatSessionRequest>,
    responses(
        (status = 200, description = "Chat session returned", body = WithUrls<Session>),
        (status = 401, description = "Authentication required"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn get_or_create_chat_session(
    org: ResolvedOrg,
    State(state): State<AppState>,
    payload: Option<Json<GetOrCreateChatSessionRequest>>,
) -> ApiResult<WithUrls<Session>> {
    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    let session = GetOrCreateChatSession {
        locale: payload.and_then(|Json(body)| body.locale),
    }
    .run(&state.ctx(&org))
    .await?;

    Ok(Json(urls.wrap(session)))
}

/// GET /v1/sessions - List sessions in organization
#[utoipa::path(
    get,
    path = "/v1/sessions",
    params(ListSessionsQuery),
    responses(
        (status = 200, description = "Paginated list of sessions", body = PaginatedResponse<WithUrls<Session>>),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn list_sessions(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListSessionsQuery>,
) -> ApiResult<PaginatedResponse<WithUrls<Session>>> {
    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    let page = ListSessions {
        filters: SessionFilterArgs {
            agent_id: query.agent_id,
            search: query.search,
            source: query.source,
            status: query.status,
            mine: query.mine,
            include_archived: query.include_archived,
            created_after: query.created_after,
            created_before: query.created_before,
            order: query.order,
        },
        offset: query.offset,
        limit: query.limit,
    }
    .run(&state.ctx(&org))
    .await?;

    Ok(Json(
        PaginatedResponse::new(page.data, page.total, page.offset, page.limit).with_urls(&urls),
    ))
}

/// GET /v1/sessions/facets - Facet counts and masthead metrics
#[utoipa::path(
    get,
    path = "/v1/sessions/facets",
    params(SessionFacetsQuery),
    responses(
        (status = 200, description = "Facet counts over the applied filters", body = SessionFacetsResponse),
        (status = 400, description = "Unknown filter value"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn get_session_facets(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<SessionFacetsQuery>,
) -> ApiResult<SessionFacetsResponse> {
    Ok(Json(
        GetSessionFacets {
            filters: SessionFilterArgs {
                agent_id: query.agent_id,
                search: query.search,
                source: query.source,
                status: query.status,
                mine: query.mine,
                include_archived: query.include_archived,
                created_after: query.created_after,
                created_before: query.created_before,
                order: query.order,
            },
        }
        .run(&state.ctx(&org))
        .await?,
    ))
}

/// GET /v1/sessions/stats - Get session counts by status
#[utoipa::path(
    get,
    path = "/v1/sessions/stats",
    responses(
        (status = 200, description = "Session statistics", body = SessionStatsResponse),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn get_session_stats(
    org: ResolvedOrg,
    State(state): State<AppState>,
) -> ApiResult<SessionStatsResponse> {
    Ok(Json(GetSessionStats.run(&state.ctx(&org)).await?))
}

/// GET /v1/sessions/{session_id} - Get session
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    responses(
        (status = 200, description = "Session found", body = WithUrls<Session>),
        (status = 400, description = "Invalid session ID"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn get_session(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> ApiResult<WithUrls<Session>> {
    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    let session = GetSession { session_id }.run(&state.ctx(&org)).await?;

    Ok(Json(urls.wrap(session)))
}

/// The model the runtime will use when a turn has no per-message override.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SessionResolvedModelResponse {
    #[schema(value_type = Option<String>)]
    pub model_id: Option<ModelId>,
}

/// GET /v1/sessions/{session_id}/resolved-model - Resolve the session's active model
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/resolved-model",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    responses(
        (status = 200, description = "Resolved model for turns without a model override", body = SessionResolvedModelResponse),
        (status = 400, description = "Invalid session ID"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn get_session_resolved_model(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> ApiResult<SessionResolvedModelResponse> {
    let session = GetSession { session_id }.run(&state.ctx(&org)).await?;
    let model_id = state
        .session_service
        .resolved_model_id(org.org_id, &session)
        .await
        .log_internal_error_json("resolve session model")?;

    Ok(Json(SessionResolvedModelResponse { model_id }))
}

/// GET /v1/sessions/{session_id}/participants - List session participants
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/participants",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    responses(
        (status = 200, description = "Session participant history", body = Vec<SessionParticipant>),
        (status = 400, description = "Invalid session ID"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn list_session_participants(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> ApiResult<Vec<SessionParticipant>> {
    Ok(Json(
        ListSessionParticipants { session_id }
            .run(&state.ctx(&org))
            .await?,
    ))
}

/// POST /v1/sessions/{session_id}/participants - Add a session participant
#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/participants",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    request_body = AddSessionParticipantRequest,
    responses(
        (status = 201, description = "Participant added successfully", body = SessionParticipant),
        (status = 400, description = "Invalid participant request"),
        (status = 404, description = "Session or agent not found"),
        (status = 409, description = "Participant conflicts with current membership"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn add_session_participant(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(req): Json<AddSessionParticipantRequest>,
) -> Result<(StatusCode, Json<SessionParticipant>), (StatusCode, Json<ErrorResponse>)> {
    let participant = AddSessionParticipant { session_id, req }
        .run(&state.ctx(&org))
        .await?;

    Ok((StatusCode::CREATED, Json(participant)))
}

/// DELETE /v1/sessions/{session_id}/participants/{participant_id} - Leave a participant
#[utoipa::path(
    delete,
    path = "/v1/sessions/{session_id}/participants/{participant_id}",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)"),
        ("participant_id" = String, Path, description = "Participant ID (prefixed, e.g., part_...)")
    ),
    responses(
        (status = 200, description = "Participant left successfully", body = SessionParticipant),
        (status = 400, description = "Invalid ID"),
        (status = 404, description = "Session or participant not found"),
        (status = 409, description = "Host participant cannot leave through this endpoint"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn leave_session_participant(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((session_id, participant_id)): Path<(String, String)>,
) -> ApiResult<SessionParticipant> {
    Ok(Json(
        LeaveSessionParticipant {
            session_id,
            participant_id,
        }
        .run(&state.ctx(&org))
        .await?,
    ))
}

/// GET /v1/sessions/{session_id}/context-report - Latest context breakdown
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/context-report",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    responses(
        (status = 200, description = "Session context report", body = SessionContextReport),
        (status = 400, description = "Invalid session ID"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn get_session_context_report(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> ApiResult<SessionContextReport> {
    Ok(Json(
        GetSessionContextReport { session_id }
            .run(&state.ctx(&org))
            .await?,
    ))
}

/// PATCH /v1/sessions/{session_id} - Update session
#[utoipa::path(
    patch,
    path = "/v1/sessions/{session_id}",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    request_body = UpdateSessionRequest,
    responses(
        (status = 200, description = "Session updated successfully", body = WithUrls<Session>),
        (status = 400, description = "Invalid session ID"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn update_session(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(req): Json<UpdateSessionRequest>,
) -> ApiResult<WithUrls<Session>> {
    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    let session = UpdateSessionCmd { session_id, req }
        .run(&state.ctx(&org))
        .await?;

    Ok(Json(urls.wrap(session)))
}

/// DELETE /v1/sessions/{session_id} - Delete session
#[utoipa::path(
    delete,
    path = "/v1/sessions/{session_id}",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    extensions(
        ("x-side-effect" = json!("reversible")),
    ),
    responses(
        (status = 204, description = "Session deleted successfully"),
        (status = 400, description = "Invalid session ID"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn delete_session(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    DeleteSession { session_id }.run(&state.ctx(&org)).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// PUT /v1/sessions/{session_id}/pin - Pin session for current user
#[utoipa::path(
    put,
    path = "/v1/sessions/{session_id}/pin",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    responses(
        (status = 204, description = "Session pinned successfully"),
        (status = 400, description = "Invalid session ID"),
        (status = 401, description = "Authentication required"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn pin_session(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    if org.user_id.is_none() {
        return Err(
            ErrorResponse::new("Authentication required to pin sessions".to_string())
                .into_response(StatusCode::UNAUTHORIZED),
        );
    }
    PinSession { session_id }.run(&state.ctx(&org)).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /v1/sessions/{session_id}/pin - Unpin session for current user
#[utoipa::path(
    delete,
    path = "/v1/sessions/{session_id}/pin",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    responses(
        (status = 204, description = "Session unpinned successfully"),
        (status = 400, description = "Invalid session ID"),
        (status = 401, description = "Authentication required"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn unpin_session(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    if org.user_id.is_none() {
        return Err(
            ErrorResponse::new("Authentication required to unpin sessions".to_string())
                .into_response(StatusCode::UNAUTHORIZED),
        );
    }
    UnpinSession { session_id }.run(&state.ctx(&org)).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// PUT /v1/sessions/{session_id}/archive - Archive session
#[utoipa::path(
    put,
    path = "/v1/sessions/{session_id}/archive",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    responses(
        (status = 204, description = "Session archived successfully"),
        (status = 400, description = "Invalid session ID"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn archive_session(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    ArchiveSession { session_id }.run(&state.ctx(&org)).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /v1/sessions/{session_id}/archive - Restore an archived session
#[utoipa::path(
    delete,
    path = "/v1/sessions/{session_id}/archive",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    responses(
        (status = 204, description = "Session unarchived successfully"),
        (status = 400, description = "Invalid session ID"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn unarchive_session(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    UnarchiveSession { session_id }
        .run(&state.ctx(&org))
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

/// POST /v1/sessions/{session_id}/cancel - Cancel current turn
///
/// Cancels the currently running turn in the session. If no turn is running,
/// this is a no-op and returns success (idempotent). When a turn is active:
/// 1. Cancel the underlying workflow execution
/// 2. Emit a turn.cancelled event
/// 3. Insert an agent message indicating the turn was cancelled
/// 4. Set the session status back to idle
#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/cancel",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    responses(
        (
            status = 200,
            description = "Turn cancelled, or no-op (status: no_op) if no turn was running",
            body = CancelTurnResponse,
            example = json!({
                "status": "cancelled",
                "message": "Turn cancelled successfully"
            })
        ),
        (
            status = 404,
            description = "Session not found",
            body = ErrorResponse,
            example = json!({
                "type": "https://docs.everruns.com/errors/session_not_found",
                "title": "Not Found",
                "status": 404,
                "detail": "Session session_01933b5a000070008000000000000001 not found in org org_00000000000000000000000000000001.",
                "code": "session_not_found"
            })
        ),
        (status = 400, description = "Invalid session ID"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn cancel_turn(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> ApiResult<CancelTurnResponse> {
    Ok(Json(
        CancelSession { session_id }.run(&state.ctx(&org)).await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{
        StorageBackend,
        models::{CreateHarnessRow, UpdateOrganizationSettings},
    };
    use everruns_durable::UpdateField;

    const TEST_HARNESS_ID: &str = "harness_550e8400e29b41d4a716446655440000";
    const TEST_AGENT_ID: &str = "agent_550e8400e29b41d4a716446655440000";

    #[test]
    fn test_create_session_request_minimal() {
        let json = format!(r#"{{"harness_id": "{}"}}"#, TEST_HARNESS_ID);
        let req: CreateSessionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req.harness_id.unwrap().to_string(), TEST_HARNESS_ID);
        assert_eq!(req.agent_id, None);
        assert_eq!(req.title, None);
        assert_eq!(req.locale, None);
        assert!(req.tags.is_empty());
        assert_eq!(req.model_id, None);
        assert!(req.capabilities.is_empty());
    }

    #[test]
    fn strip_internal_only_fields_drops_client_supplied_delegation_metadata() {
        // A public HTTP client must not be able to forge the internal-only
        // parent link; the boundary drops any caller-supplied value.
        let json = format!(r#"{{"harness_id": "{}"}}"#, TEST_HARNESS_ID);
        let mut req: CreateSessionRequest = serde_json::from_str(&json).unwrap();
        req.parent_session_id = Some(SessionId::new());
        req.forked_from_session_id = Some(SessionId::new());
        req.budget_root_session_id = Some(SessionId::new());
        req.seed = SessionSeedMode::Fork;
        assert!(req.parent_session_id.is_some());

        strip_internal_only_fields(&mut req);

        assert_eq!(req.parent_session_id, None);
        assert_eq!(req.forked_from_session_id, None);
        assert_eq!(req.budget_root_session_id, None);
        assert_eq!(req.seed, SessionSeedMode::Fresh);
    }

    #[test]
    fn strip_internal_only_fields_drops_client_supplied_fork_lineage_and_seed() {
        // Public clients must not be able to trigger internal detached-spawn
        // seeding from another session via the generic create-session API.
        let mut req: CreateSessionRequest =
            serde_json::from_str(&format!(r#"{{"harness_id": "{}"}}"#, TEST_HARNESS_ID)).unwrap();
        req.forked_from_session_id = Some(SessionId::new());
        req.seed = SessionSeedMode::Fork;

        strip_internal_only_fields(&mut req);

        assert_eq!(req.forked_from_session_id, None);
        assert_eq!(req.seed, SessionSeedMode::Fresh);
    }

    #[test]
    fn test_create_session_request_missing_harness_id_is_none() {
        let json = r#"{}"#;
        let req: CreateSessionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.harness_id, None);
    }

    #[test]
    fn test_create_session_request_with_agent_id() {
        // agent_id is optional
        let json = format!(
            r#"{{"harness_id": "{}", "agent_id": "{}"}}"#,
            TEST_HARNESS_ID, TEST_AGENT_ID
        );
        let req: CreateSessionRequest = serde_json::from_str(&json).unwrap();
        let expected_agent_id: AgentId = TEST_AGENT_ID.parse().unwrap();
        assert_eq!(req.agent_id, Some(expected_agent_id));
    }

    #[test]
    fn test_create_session_request_with_title() {
        let json = format!(
            r#"{{"harness_id": "{}", "agent_id": "{}", "title": "Test Session", "locale": "uk-UA"}}"#,
            TEST_HARNESS_ID, TEST_AGENT_ID
        );
        let req: CreateSessionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req.title, Some("Test Session".to_string()));
        assert_eq!(req.locale.as_deref(), Some("uk-UA"));
        assert!(req.tags.is_empty());
        assert_eq!(req.model_id, None);
    }

    #[test]
    fn test_create_session_request_with_model_id() {
        let model_id: ModelId = "model_550e8400e29b41d4a716446655440000".parse().unwrap();
        let json = format!(
            r#"{{"harness_id": "{}", "agent_id": "{}", "model_id": "{}"}}"#,
            TEST_HARNESS_ID, TEST_AGENT_ID, model_id
        );
        let req: CreateSessionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req.model_id, Some(model_id));
    }

    #[test]
    fn test_create_session_request_full() {
        let model_id: ModelId = "model_550e8400e29b41d4a716446655440001".parse().unwrap();
        let json = format!(
            r#"{{"harness_id": "{}", "agent_id": "{}", "title": "Full Session", "tags": ["tag1", "tag2"], "model_id": "{}"}}"#,
            TEST_HARNESS_ID, TEST_AGENT_ID, model_id
        );
        let req: CreateSessionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req.title, Some("Full Session".to_string()));
        assert_eq!(req.tags, vec!["tag1", "tag2"]);
        assert_eq!(req.model_id, Some(model_id));
        assert!(req.capabilities.is_empty());
    }

    #[test]
    fn test_create_session_request_with_capabilities() {
        let json = format!(
            r#"{{
                "harness_id": "{}",
                "agent_id": "{}",
                "capabilities": [
                    {{"ref": "current_time"}},
                    {{"ref": "web_fetch", "config": {{"timeout_ms": 30000}}}}
                ]
            }}"#,
            TEST_HARNESS_ID, TEST_AGENT_ID
        );
        let req: CreateSessionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req.capabilities[0].capability_id(), "current_time");
        assert_eq!(req.capabilities[1].capability_id(), "web_fetch");
    }

    #[test]
    fn test_create_session_request_drops_builtin_tools_with_warn() {
        // Deprecation window: legacy `builtin` entries are dropped, not
        // rejected, so existing SDK/CLI clients keep working while operators
        // monitor the migration.
        let json = format!(
            r#"{{
                "harness_id": "{}",
                "tools": [
                    {{
                        "type": "builtin",
                        "name": "read_file",
                        "description": "Read file",
                        "parameters": {{"type": "object"}}
                    }},
                    {{
                        "type": "client_side",
                        "name": "lookup_crm",
                        "description": "Lookup",
                        "parameters": {{"type": "object"}}
                    }}
                ]
            }}"#,
            TEST_HARNESS_ID
        );

        let req: CreateSessionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req.tools.len(), 1);
        // Only the client_side entry survives.
        assert!(matches!(
            req.tools[0],
            everruns_provider::tool_types::ToolDefinition::ClientSide(_)
        ));
    }

    #[test]
    fn test_create_session_request_rejects_client_side_mcp_tool_name() {
        let json = format!(
            r#"{{
                "harness_id": "{}",
                "tools": [
                    {{
                        "type": "client_side",
                        "name": "mcp_guard__screen",
                        "description": "Spoof guardrail MCP endpoint",
                        "parameters": {{"type": "object"}}
                    }}
                ]
            }}"#,
            TEST_HARNESS_ID
        );

        let err = serde_json::from_str::<CreateSessionRequest>(&json).unwrap_err();
        assert!(
            err.to_string()
                .contains("client_side tool names must not use the reserved mcp_ prefix"),
            "unexpected error: {err}"
        );
    }

    // Strict-mode tests live in `tests/strict_client_tools.rs` so they run in
    // their own process and don't race with the lenient round-trip tests
    // here over the `EVERRUNS_REJECT_NON_CLIENT_SIDE_TOOLS` env var.

    #[test]
    fn test_update_session_request_minimal() {
        let json = r#"{}"#;
        let req: UpdateSessionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.title, None);
        assert_eq!(req.agent_identity_id, UpdateField::Unchanged);
        assert_eq!(req.locale, None);
        assert_eq!(req.tags, None);
    }

    #[test]
    fn test_update_session_request_clears_agent_identity_when_null() {
        let json = r#"{"agent_identity_id":null}"#;
        let req: UpdateSessionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.agent_identity_id, UpdateField::Clear);
    }

    #[test]
    fn test_update_session_request_sets_agent_identity_when_present() {
        let json = r#"{"agent_identity_id":"identity_550e8400e29b41d4a716446655440000"}"#;
        let req: UpdateSessionRequest = serde_json::from_str(json).unwrap();
        assert!(matches!(req.agent_identity_id, UpdateField::Set(_)));
    }

    #[test]
    fn test_update_session_request_with_title() {
        let json = r#"{"title": "Updated Title", "locale": "en-US"}"#;
        let req: UpdateSessionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.title, Some("Updated Title".to_string()));
        assert_eq!(req.locale.as_deref(), Some("en-US"));
        assert_eq!(req.tags, None);
    }

    #[test]
    fn test_update_session_request_with_tags() {
        let json = r#"{"tags": ["new-tag"]}"#;
        let req: UpdateSessionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.title, None);
        assert_eq!(req.tags, Some(vec!["new-tag".to_string()]));
    }

    #[tokio::test]
    async fn test_resolve_session_harness_id_defaults_to_generic() {
        let db = StorageBackend::in_memory();
        let row = db
            .create_harness(
                42,
                CreateHarnessRow {
                    name: "generic".to_string(),
                    display_name: Some("Generic".to_string()),
                    description: Some("Generic".to_string()),
                    system_prompt: Some("You are helpful.".to_string()),
                    parent_harness_id: None,
                    default_model_id: None,
                    tags: vec!["generic".to_string()],
                    initial_files: serde_json::json!([]),
                    mcp_servers: serde_json::json!({}),
                    is_built_in: true,
                    network_access: None,
                    embedder_metadata: serde_json::json!({}),
                },
            )
            .await
            .unwrap();

        let harness_id = crate::domains::sessions::queries::resolve_session_harness_id(
            &db,
            42,
            None,
            None,
            Some("generic"),
        )
        .await
        .unwrap();
        assert_eq!(harness_id, row.id);
    }

    #[tokio::test]
    async fn test_resolve_session_harness_id_uses_org_default_harness() {
        let db = StorageBackend::in_memory();
        let default_harness_id: HarnessId = TEST_HARNESS_ID.parse().unwrap();

        db.patch_organization_settings(
            42,
            UpdateOrganizationSettings {
                default_harness_id: UpdateField::Set(default_harness_id),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let harness_id = crate::domains::sessions::queries::resolve_session_harness_id(
            &db,
            42,
            None,
            None,
            Some("generic"),
        )
        .await
        .unwrap();
        assert_eq!(harness_id, default_harness_id);
    }

    #[tokio::test]
    async fn test_resolve_session_harness_id_prefers_agent_over_org_default() {
        // Agent-first (D4): with no explicit request harness, the agent's harness
        // wins over the org default.
        let db = StorageBackend::in_memory();
        let default_harness_id: HarnessId = TEST_HARNESS_ID.parse().unwrap();
        let agent_harness_id: HarnessId =
            "harness_550e8400e29b41d4a716446655440009".parse().unwrap();

        db.patch_organization_settings(
            42,
            UpdateOrganizationSettings {
                default_harness_id: UpdateField::Set(default_harness_id),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let harness_id = crate::domains::sessions::queries::resolve_session_harness_id(
            &db,
            42,
            None,
            Some(agent_harness_id),
            Some("generic"),
        )
        .await
        .unwrap();
        assert_eq!(harness_id, agent_harness_id);
    }

    #[tokio::test]
    async fn test_resolve_session_harness_id_request_overrides_agent() {
        // Explicit request harness wins over the agent's harness (D4 override).
        let db = StorageBackend::in_memory();
        let requested: HarnessId = TEST_HARNESS_ID.parse().unwrap();
        let agent_harness_id: HarnessId =
            "harness_550e8400e29b41d4a716446655440009".parse().unwrap();

        let harness_id = crate::domains::sessions::queries::resolve_session_harness_id(
            &db,
            42,
            Some(requested),
            Some(agent_harness_id),
            Some("generic"),
        )
        .await
        .unwrap();
        assert_eq!(harness_id, requested);
    }
}
