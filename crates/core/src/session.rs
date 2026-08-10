// Session domain types
//
// These types represent the Session entity and its status.
// Used by both API and worker crates.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::capability_types::AgentCapabilityConfig;
use crate::events::TokenUsage;
use crate::mcp_server::{ScopedMcpServers, scoped_mcp_servers_is_empty};
use crate::network_access::NetworkAccessList;
use crate::principal::PrincipalSummary;
use crate::tool_types::ToolDefinition;
use crate::typed_id::{
    AgentId, AgentIdentityId, AgentVersionId, HarnessId, ModelId, PrincipalId, SessionId,
    SessionParticipantId, WorkspaceId,
};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Subagent lifecycle status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SubagentStatus {
    Spawning,
    Running,
    Completed,
    Failed,
    Cancelled,
    MaxIterationsReached,
    /// The durable engine deliberately stopped (sealed) the child's turn to
    /// prevent further waste (no forward progress, or budget exhausted). This is
    /// terminal and non-retryable, and is intentionally distinct from `Failed`
    /// so the parent agent can decide what to do next (the seal reason is
    /// carried in the child's final assistant message / spawn `result`).
    Sealed,
}

impl std::fmt::Display for SubagentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SubagentStatus::Spawning => write!(f, "spawning"),
            SubagentStatus::Running => write!(f, "running"),
            SubagentStatus::Completed => write!(f, "completed"),
            SubagentStatus::Failed => write!(f, "failed"),
            SubagentStatus::Cancelled => write!(f, "cancelled"),
            SubagentStatus::MaxIterationsReached => write!(f, "max_iterations_reached"),
            SubagentStatus::Sealed => write!(f, "sealed"),
        }
    }
}

impl From<&str> for SubagentStatus {
    fn from(s: &str) -> Self {
        match s {
            "spawning" => SubagentStatus::Spawning,
            "running" => SubagentStatus::Running,
            "completed" => SubagentStatus::Completed,
            "failed" => SubagentStatus::Failed,
            "cancelled" => SubagentStatus::Cancelled,
            "max_iterations_reached" => SubagentStatus::MaxIterationsReached,
            "sealed" => SubagentStatus::Sealed,
            _ => SubagentStatus::Spawning,
        }
    }
}

/// Session execution status.
/// - `started`: Session just created, no turn executed yet
/// - `active`: A turn is currently running
/// - `idle`: Turn completed, session waiting for next input
/// - `paused`: Budget limit reached, waiting for user to increase limit or resume
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    /// Session just created, no turn executed yet.
    Started,
    /// A turn is currently running (session is active).
    Active,
    /// Turn completed, session waiting for next input (idle).
    Idle,
    /// Waiting for client to submit tool results.
    WaitingForToolResults,
    /// Budget limit reached — session paused until user resumes or increases limit.
    Paused,
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionStatus::Started => write!(f, "started"),
            SessionStatus::Active => write!(f, "active"),
            SessionStatus::Idle => write!(f, "idle"),
            SessionStatus::WaitingForToolResults => write!(f, "waiting_for_tool_results"),
            SessionStatus::Paused => write!(f, "paused"),
        }
    }
}

impl From<&str> for SessionStatus {
    fn from(s: &str) -> Self {
        match s {
            "active" => SessionStatus::Active,
            "idle" => SessionStatus::Idle,
            "waiting_for_tool_results" => SessionStatus::WaitingForToolResults,
            "paused" => SessionStatus::Paused,
            // Handle legacy values during migration
            "running" => SessionStatus::Active,
            "pending" | "completed" | "failed" => SessionStatus::Idle,
            _ => SessionStatus::Started,
        }
    }
}

/// How a session came into existence.
///
/// Closed set: the sessions facet rail enumerates every variant, so the value
/// is typed rather than a free-form string. Ingress paths set it server-side —
/// clients may only declare the two variants they can legitimately be
/// (`Chat` and `Api`), which keeps a facet like "started by a schedule"
/// trustworthy. Rows that predate the column, or whose origin could not be
/// inferred at backfill time, carry `Unknown`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SessionSource {
    /// Interactive chat thread (UI chat surface, global chat, public chat).
    Chat,
    /// Direct `POST /v1/sessions` from the API, CLI, or an SDK.
    Api,
    /// Slack channel ingress.
    Slack,
    /// AG-UI channel ingress.
    AgUi,
    /// FCP channel ingress.
    Fcp,
    /// Fired by a schedule (app schedule channel or agent trigger).
    Schedule,
    /// Inbound webhook or app API endpoint.
    Webhook,
    /// Inbound A2A request.
    A2a,
    /// Created by an eval run.
    Eval,
    /// Spawned as a subagent / delegated peer of another session.
    Subagent,
    /// Origin not recorded (pre-migration rows that could not be inferred).
    #[default]
    Unknown,
}

impl SessionSource {
    pub const ALL: &'static [SessionSource] = &[
        SessionSource::Chat,
        SessionSource::Api,
        SessionSource::Slack,
        SessionSource::AgUi,
        SessionSource::Fcp,
        SessionSource::Schedule,
        SessionSource::Webhook,
        SessionSource::A2a,
        SessionSource::Eval,
        SessionSource::Subagent,
        SessionSource::Unknown,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            SessionSource::Chat => "chat",
            SessionSource::Api => "api",
            SessionSource::Slack => "slack",
            SessionSource::AgUi => "ag_ui",
            SessionSource::Fcp => "fcp",
            SessionSource::Schedule => "schedule",
            SessionSource::Webhook => "webhook",
            SessionSource::A2a => "a2a",
            SessionSource::Eval => "eval",
            SessionSource::Subagent => "subagent",
            SessionSource::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|v| v.as_str() == s)
    }

    /// Whether a client may declare this source on `POST /v1/sessions`.
    /// Everything else is server-owned so facets cannot be spoofed.
    pub fn is_client_declarable(self) -> bool {
        matches!(self, SessionSource::Chat | SessionSource::Api)
    }
}

impl std::fmt::Display for SessionSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for SessionSource {
    fn from(s: &str) -> Self {
        Self::parse(s).unwrap_or(SessionSource::Unknown)
    }
}

/// Outcome-oriented view of a session, as the sessions list and facet rail
/// present it. Derived from the session's execution status plus the outcome of
/// its most recent turn — `SessionStatus` alone has no notion of failure.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SessionActivity {
    /// A turn is executing, or the session is waiting on client tool results.
    Running,
    /// Budget limit reached; waiting for the user to resume.
    Paused,
    /// Last completed turn failed or was cancelled.
    Failed,
    /// Last completed turn succeeded and nothing is running.
    Completed,
    /// Created but idle with no completed turn yet.
    #[default]
    Idle,
}

impl SessionActivity {
    pub const ALL: &'static [SessionActivity] = &[
        SessionActivity::Running,
        SessionActivity::Paused,
        SessionActivity::Failed,
        SessionActivity::Completed,
        SessionActivity::Idle,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            SessionActivity::Running => "running",
            SessionActivity::Paused => "paused",
            SessionActivity::Failed => "failed",
            SessionActivity::Completed => "completed",
            SessionActivity::Idle => "idle",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|v| v.as_str() == s)
    }

    /// Single source of truth for the derivation, mirrored by
    /// `session_activity_sql` in the sessions repository so the list, the facet
    /// counts, and the in-memory backend cannot drift apart.
    pub fn derive(status: &SessionStatus, last_turn_status: Option<&str>) -> Self {
        match status {
            SessionStatus::Active | SessionStatus::WaitingForToolResults => {
                SessionActivity::Running
            }
            SessionStatus::Paused => SessionActivity::Paused,
            _ => match last_turn_status {
                Some("failed") | Some("cancelled") => SessionActivity::Failed,
                Some("completed") => SessionActivity::Completed,
                _ => SessionActivity::Idle,
            },
        }
    }
}

impl std::fmt::Display for SessionActivity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Kind of actor participating in a session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SessionParticipantKind {
    Agent,
    User,
}

impl std::fmt::Display for SessionParticipantKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionParticipantKind::Agent => write!(f, "agent"),
            SessionParticipantKind::User => write!(f, "user"),
        }
    }
}

impl From<&str> for SessionParticipantKind {
    fn from(s: &str) -> Self {
        match s {
            "agent" => SessionParticipantKind::Agent,
            "user" => SessionParticipantKind::User,
            _ => SessionParticipantKind::User,
        }
    }
}

/// Role a participant has inside a session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SessionParticipantRole {
    Host,
    Member,
}

impl std::fmt::Display for SessionParticipantRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionParticipantRole::Host => write!(f, "host"),
            SessionParticipantRole::Member => write!(f, "member"),
        }
    }
}

impl From<&str> for SessionParticipantRole {
    fn from(s: &str) -> Self {
        match s {
            "host" => SessionParticipantRole::Host,
            "member" => SessionParticipantRole::Member,
            _ => SessionParticipantRole::Member,
        }
    }
}

/// Session participant - an agent or user that has joined a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SessionParticipant {
    /// Unique identifier for the participant row (format: part_{32-hex}).
    #[cfg_attr(
        feature = "openapi",
        schema(
            value_type = String,
            example = "part_01933b5a00007000800000000000001"
        )
    )]
    pub id: SessionParticipantId,
    /// Session this participant belongs to.
    #[cfg_attr(
        feature = "openapi",
        schema(
            value_type = String,
            example = "session_01933b5a00007000800000000000001"
        )
    )]
    pub session_id: SessionId,
    pub kind: SessionParticipantKind,
    /// Present for agent participants.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "openapi",
        schema(
            value_type = Option<String>,
            example = "agent_01933b5a00007000800000000000001"
        )
    )]
    pub agent_id: Option<AgentId>,
    /// Immutable agent version captured for an agent participant when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "openapi",
        schema(
            value_type = Option<String>,
            example = "agentver_01933b5a00007000800000000000001"
        )
    )]
    pub agent_version_id: Option<AgentVersionId>,
    /// Principal that joined the session.
    #[cfg_attr(
        feature = "openapi",
        schema(
            value_type = String,
            example = "principal_01933b5a000070008000000000000001"
        )
    )]
    pub principal_id: PrincipalId,
    /// Human-readable name captured for this participant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub role: SessionParticipantRole,
    pub joined_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left_at: Option<DateTime<Utc>>,
}

/// Session - instance of agentic loop execution.
/// A session represents a single conversation with an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Session {
    /// Unique identifier for the session (format: session_{32-hex}).
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "session_01933b5a00007000800000000000001"))]
    pub id: SessionId,
    /// Organization this session belongs to (format: org_{32-hex}).
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "org_00000000000000000000000000000001"))]
    pub organization_id: String,
    /// Workspace this session is attached to (format: wsp_{32-hex}). Owns the
    /// session's virtual filesystem. For the default 1:1 case this mirrors the
    /// session id, but clients should read it here rather than deriving it.
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "wsp_01933b5a00007000800000000000001"))]
    pub workspace_id: WorkspaceId,
    /// ID of the harness for this session (format: harness_{32-hex}).
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "harness_01933b5a00007000800000000000001"))]
    pub harness_id: HarnessId,
    /// ID of the agent working in this session (format: agent_{32-hex}). Optional.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "agent_01933b5a00007000800000000000001"))]
    pub agent_id: Option<AgentId>,
    /// Immutable agent version captured when the session was created or rebound.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "agentver_01933b5a00007000800000000000001"))]
    pub agent_version_id: Option<AgentVersionId>,
    /// Optional resident agent identity for unattended/background execution.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "identity_01933b5a00007000800000000000001"))]
    pub agent_identity_id: Option<AgentIdentityId>,
    /// Owning principal for this session.
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "principal_01933b5a000070008000000000000001"))]
    pub owner_principal_id: PrincipalId,
    /// Denormalized effective human owner of the owning principal lineage.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "openapi",
        schema(example = "550e8400-e29b-41d4-a716-446655440000")
    )]
    pub resolved_owner_user_id: Option<uuid::Uuid>,
    /// Owning principal summary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<PrincipalSummary>,
    /// Effective human owner summary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_owner: Option<PrincipalSummary>,
    /// Human-readable title for the session.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "Q3 marketing brief"))]
    pub title: Option<String>,
    /// Session objective visible to the runtime agent at system-prompt level.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "openapi",
        schema(example = "Investigate the queue latency regression")
    )]
    pub goal: Option<String>,
    /// Locale for localized agent behavior and formatting (BCP 47, e.g. `uk-UA`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "en-US"))]
    pub locale: Option<String>,
    /// Preview text from the first user message (truncated).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "openapi",
        schema(example = "Help me draft the Q3 marketing plan")
    )]
    pub preview: Option<String>,
    /// Preview text from the last assistant response (truncated).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "openapi",
        schema(example = "Here is a Q3 plan covering the three pillars we discussed...")
    )]
    pub output_preview: Option<String>,
    /// Tags for organizing and filtering sessions.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(example = json!(["marketing", "q3", "draft"])))]
    pub tags: Vec<String>,
    /// LLM model ID to use for this session (format: model_{32-hex}).
    /// Overrides the agent's default model if set.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "model_01933b5a00007000800000000000001"))]
    pub model_id: Option<ModelId>,
    /// Session-level capabilities (additive to agent capabilities).
    /// Applied after agent capabilities when building RuntimeAgent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = Vec<crate::capability_types::AgentCapabilityConfigSchema>)
    )]
    pub capabilities: Vec<AgentCapabilityConfig>,
    /// Client-side tools for this session (additive to agent tools).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDefinition>,
    /// Remote MCP servers scoped to this session only.
    #[serde(
        default,
        rename = "mcpServers",
        alias = "mcp_servers",
        skip_serializing_if = "scoped_mcp_servers_is_empty"
    )]
    pub mcp_servers: ScopedMcpServers,
    /// Session-level system prompt override.
    /// Prepended to the agent's system prompt when building RuntimeAgent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// Session-level initial files (additive to agent initial_files).
    /// Files with matching paths override agent/harness files; new paths are appended.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub initial_files: Vec<crate::session_file::InitialFile>,
    /// Session-level client hints — arbitrary key-value pairs declared by the
    /// client at session creation time. These are defaults for every turn;
    /// per-message `controls.hints` override these key-by-key (shallow merge).
    ///
    /// Examples: `{"setup_connection": true, "rich_media": true}`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hints: Option<std::collections::HashMap<String, serde_json::Value>>,
    /// Network access list controlling which hosts/URLs this session can reach.
    /// Merged with harness and agent layers (allowed: intersect, blocked: union).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_access: Option<NetworkAccessList>,
    /// Maximum number of LLM iterations per turn for this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = 50))]
    pub max_iterations: Option<usize>,
    /// Request-level parallel tool calling preference (EVE-598).
    ///
    /// `None` (default) preserves provider defaults. `Some(true)` signals the
    /// provider that parallel tool calls are wanted; `Some(false)` requests at
    /// most one tool call per turn and forces serial execution. Merged across
    /// harness/agent/session layers (overlay wins).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = true))]
    pub parallel_tool_calls: Option<bool>,
    /// Current execution status of the session.
    pub status: SessionStatus,
    /// How this session was started. Server-owned for every ingress path.
    #[serde(default)]
    pub source: SessionSource,
    /// Outcome-oriented status derived from `status` and the last turn result.
    /// This is the value the sessions list groups by and the facet rail counts.
    #[serde(default)]
    pub activity: SessionActivity,
    /// Timestamp when the session was created.
    #[cfg_attr(feature = "openapi", schema(example = "2026-05-25T10:00:00Z"))]
    pub created_at: DateTime<Utc>,
    /// Timestamp when the session was last updated.
    #[cfg_attr(feature = "openapi", schema(example = "2026-05-25T10:14:32Z"))]
    pub updated_at: DateTime<Utc>,
    /// Timestamp when the session started executing.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "2026-05-25T10:00:01Z"))]
    pub started_at: Option<DateTime<Utc>>,
    /// Timestamp when the session finished (completed or failed).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "2026-05-25T10:14:32Z"))]
    pub finished_at: Option<DateTime<Utc>>,
    /// Cumulative token usage for all LLM calls in this session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    /// Whether this session is pinned by the current user.
    /// Only populated when the request has an authenticated user context.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = false))]
    pub is_pinned: Option<bool>,
    /// Number of active (enabled) schedules for this session.
    /// Populated when the session is fetched for API responses.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = 2))]
    pub active_schedule_count: Option<u32>,
    /// Aggregated UI features from all active capabilities (harness + agent + session).
    /// Computed at read time from the capability registry.
    /// Known features: "file_system", "schedules", "secrets", "key_value",
    /// "sql_database", "leased_resources".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "openapi", schema(example = json!(["file_system", "secrets"])))]
    pub features: Vec<String>,

    // -- Subagent nesting fields --
    /// Parent session that spawned this subagent. NULL for top-level sessions.
    /// Used to compute governed subagent delegation depth.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub parent_session_id: Option<SessionId>,

    // -- Fork lineage fields (knowledge/runtime-resources/forking-sessions.md) --
    /// Session this one was forked from. NULL for sessions that were not forked.
    /// Distinct from `parent_session_id` (subagent nesting): forking is a
    /// user-initiated "branch from here" relationship.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub forked_from_session_id: Option<SessionId>,
    /// Parent event sequence the fork was taken at (the fork point). NULL unless
    /// this session is a fork.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = 42))]
    pub forked_from_sequence: Option<i32>,

    // -- Blueprint fields (only set when this session runs a blueprint agent) --
    /// Blueprint ID. When set, reason_activity and act_activity build RuntimeAgent
    /// from the blueprint definition instead of from harness_id/agent_id.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "blueprint_research_pack"))]
    pub blueprint_id: Option<String>,
    /// Validated config passed by host at blueprint spawn time.
    /// Example: `{"target_repo": "acme/everruns"}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub blueprint_config: Option<serde_json::Value>,
}

/// Seed mode used when creating a peer session from an existing session.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SessionSeedMode {
    /// Create an empty session and only record lineage when provided.
    #[default]
    Fresh,
    /// Copy conversation events, workspace files, and durable session storage.
    Fork,
    /// Copy workspace files only.
    Workspace,
}

impl SessionSeedMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Fork => "fork",
            Self::Workspace => "workspace",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The list filters activity in SQL and the in-memory backend filters it in
    /// Rust, so this truth table is the contract both sides implement. It is
    /// duplicated verbatim as a comment beside `ACTIVITY_SQL` in
    /// `crates/server/src/storage/repositories/sessions.rs`.
    #[test]
    fn activity_derivation_truth_table() {
        use SessionActivity as A;
        use SessionStatus as S;

        let cases: &[(SessionStatus, Option<&str>, SessionActivity)] = &[
            // Execution state wins: a running turn is running whatever the
            // previous turn did.
            (S::Active, None, A::Running),
            (S::Active, Some("failed"), A::Running),
            (S::WaitingForToolResults, Some("completed"), A::Running),
            (S::Paused, Some("failed"), A::Paused),
            // Otherwise the last terminal turn decides.
            (S::Idle, Some("completed"), A::Completed),
            (S::Idle, Some("failed"), A::Failed),
            (S::Idle, Some("cancelled"), A::Failed),
            (S::Started, Some("completed"), A::Completed),
            // No completed turn yet, or an unrecognized outcome.
            (S::Started, None, A::Idle),
            (S::Idle, None, A::Idle),
            (S::Idle, Some("weird"), A::Idle),
        ];

        for (status, last_turn, expected) in cases {
            assert_eq!(
                SessionActivity::derive(status, *last_turn),
                *expected,
                "status={status} last_turn={last_turn:?}"
            );
        }
    }

    #[test]
    fn session_source_round_trips_through_its_wire_string() {
        for source in SessionSource::ALL {
            assert_eq!(SessionSource::parse(source.as_str()), Some(*source));
        }
        // Unknown text degrades to the explicit unknown bucket rather than
        // inventing a facet.
        assert_eq!(SessionSource::from("not_a_source"), SessionSource::Unknown);
    }

    #[test]
    fn only_chat_and_api_are_client_declarable() {
        let declarable: Vec<_> = SessionSource::ALL
            .iter()
            .filter(|s| s.is_client_declarable())
            .copied()
            .collect();
        assert_eq!(declarable, vec![SessionSource::Chat, SessionSource::Api]);
    }

    #[test]
    fn session_activity_round_trips_through_its_wire_string() {
        for activity in SessionActivity::ALL {
            assert_eq!(SessionActivity::parse(activity.as_str()), Some(*activity));
        }
        assert_eq!(SessionActivity::parse("nope"), None);
    }
}
