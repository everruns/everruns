// Neutral session identity and turn execution state (EVE-882).
//
// Decision: the persisted `Session` database/API aggregate — source/facet
// classifications, participants and ownership references, UI/list activity
// projections, timestamps, catalog relationships — lives in
// `everruns-platform`. Core keeps only this portable, execution-facing view:
// the session correlation values and effective per-session configuration a
// turn consumes, plus the small neutral execution state the host lifecycle
// drives. The platform loading seam (server repositories, worker adapters,
// hosted stores) projects stored records into these values before host
// execution begins — the host never requests or receives a stored Session.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::capability_types::AgentCapabilityConfig;
use crate::events::TokenUsage;
use crate::mcp_server::{ScopedMcpServers, scoped_mcp_servers_is_empty};
use crate::network_access::NetworkAccessList;
use crate::session_file::InitialFile;
use crate::tool_types::ToolDefinition;
use crate::typed_id::{AgentId, HarnessId, ModelId, SessionId, WorkspaceId};

/// Subagent lifecycle status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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

/// Neutral session execution state (EVE-882).
///
/// The small value host planning and lifecycle transitions operate on:
/// - `started`: session created, no turn executed yet
/// - `active`: a turn is currently running
/// - `idle`: turn completed, session waiting for next input
/// - `waiting_for_tool_results`: waiting for the client to submit tool results
/// - `paused`: budget limit reached, waiting for the user to resume
///
/// The persisted product status enum lives in `everruns-platform`
/// (`SessionStatus`) and maps to/from this value at the adapter boundary; its
/// wire strings are the [`SessionExecutionState::as_str`] values below.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionExecutionState {
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

impl SessionExecutionState {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionExecutionState::Started => "started",
            SessionExecutionState::Active => "active",
            SessionExecutionState::Idle => "idle",
            SessionExecutionState::WaitingForToolResults => "waiting_for_tool_results",
            SessionExecutionState::Paused => "paused",
        }
    }
}

impl std::fmt::Display for SessionExecutionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for SessionExecutionState {
    fn from(s: &str) -> Self {
        match s {
            "active" => SessionExecutionState::Active,
            "idle" => SessionExecutionState::Idle,
            "waiting_for_tool_results" => SessionExecutionState::WaitingForToolResults,
            "paused" => SessionExecutionState::Paused,
            // Handle legacy values during migration
            "running" => SessionExecutionState::Active,
            "pending" | "completed" | "failed" => SessionExecutionState::Idle,
            _ => SessionExecutionState::Started,
        }
    }
}

/// Portable execution view of one session (EVE-882).
///
/// Carries exactly what turn execution consumes: the typed correlation
/// values, the session's own configuration overlay layer (leaf of the
/// harness → agent → session chain), the neutral execution state, and
/// cumulative usage accounting. It is not a persistence record — origin
/// facets, activity projections, participants, ownership summaries,
/// timestamps, and list/UI metadata stay in `everruns-platform`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionSession {
    /// Session identity (correlation value).
    pub id: SessionId,
    /// Public (`org_…`) organization id used for execution scoping. This is a
    /// correlation value, not the organization record.
    pub organization_id: String,
    /// Workspace owning the session's virtual filesystem. For the default 1:1
    /// case this mirrors the session id.
    pub workspace_id: WorkspaceId,
    /// Harness providing the base environment configuration layer.
    pub harness_id: HarnessId,
    /// Optional agent working in this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    /// Human-readable title (readable/writable through the session capability).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Session objective visible to the runtime agent at system-prompt level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    /// Locale for localized agent behavior and formatting (BCP 47).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    /// Tags consulted by execution modes (e.g. progress reporting).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Session-level model override (higher priority than agent/harness).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<ModelId>,
    /// Session-level capabilities (additive to agent capabilities).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// Session-level initial files (additive to agent initial_files).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub initial_files: Vec<InitialFile>,
    /// Session-level client hints; per-message `controls.hints` override these
    /// key-by-key (shallow merge).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hints: Option<HashMap<String, serde_json::Value>>,
    /// Network access list merged with harness and agent layers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_access: Option<NetworkAccessList>,
    /// Maximum number of LLM iterations per turn for this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_iterations: Option<usize>,
    /// Request-level parallel tool calling preference (EVE-598).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    /// Neutral execution state driven by the host lifecycle.
    pub status: SessionExecutionState,
    /// Cumulative token usage for all LLM calls in this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    /// Parent session that spawned this subagent (subagent nesting depth).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<SessionId>,
    /// Session this one was forked from (delegation-result correlation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forked_from_session_id: Option<SessionId>,
    /// Blueprint ID. When set, execution builds the RuntimeAgent from the
    /// blueprint definition instead of harness/agent configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blueprint_id: Option<String>,
    /// Validated config passed by host at blueprint spawn time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blueprint_config: Option<serde_json::Value>,
}

impl ExecutionSession {
    /// Create an execution session with the given correlation identity; all
    /// configuration starts empty, status starts at `started`, and the
    /// organization defaults to the single-tenant default org.
    pub fn new(id: SessionId, workspace_id: WorkspaceId, harness_id: HarnessId) -> Self {
        Self {
            id,
            organization_id: crate::DEFAULT_ORG_PUBLIC_ID.to_string(),
            workspace_id,
            harness_id,
            agent_id: None,
            title: None,
            goal: None,
            locale: None,
            tags: Vec::new(),
            model_id: None,
            capabilities: Vec::new(),
            tools: Vec::new(),
            mcp_servers: ScopedMcpServers::default(),
            system_prompt: None,
            initial_files: Vec::new(),
            hints: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            status: SessionExecutionState::Started,
            usage: None,
            parent_session_id: None,
            forked_from_session_id: None,
            blueprint_id: None,
            blueprint_config: None,
        }
    }

    /// Create an execution session under the default 1:1 workspace identity
    /// (`workspace.id == session.id`).
    pub fn with_own_workspace(id: SessionId, harness_id: HarnessId) -> Self {
        Self::new(id, WorkspaceId::from_uuid(id.uuid()), harness_id)
    }
}

/// Seed mode used when creating a peer session from an existing session.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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

    #[test]
    fn execution_state_round_trips_through_its_wire_string() {
        for state in [
            SessionExecutionState::Started,
            SessionExecutionState::Active,
            SessionExecutionState::Idle,
            SessionExecutionState::WaitingForToolResults,
            SessionExecutionState::Paused,
        ] {
            assert_eq!(SessionExecutionState::from(state.as_str()), state);
        }
        // Legacy values degrade the same way the stored status column does.
        assert_eq!(
            SessionExecutionState::from("running"),
            SessionExecutionState::Active
        );
        assert_eq!(
            SessionExecutionState::from("completed"),
            SessionExecutionState::Idle
        );
        assert_eq!(
            SessionExecutionState::from("garbage"),
            SessionExecutionState::Started
        );
    }

    #[test]
    fn execution_session_carries_no_persistence_metadata() {
        let session = ExecutionSession::with_own_workspace(SessionId::new(), HarnessId::new());
        let json = serde_json::to_value(&session).unwrap();
        for persistence_field in [
            "source",
            "activity",
            "owner_principal_id",
            "resolved_owner_user_id",
            "owner",
            "effective_owner",
            "agent_version_id",
            "agent_identity_id",
            "preview",
            "output_preview",
            "created_at",
            "updated_at",
            "started_at",
            "finished_at",
            "is_pinned",
            "active_schedule_count",
            "features",
            "forked_from_sequence",
        ] {
            assert!(
                json.get(persistence_field).is_none(),
                "portable execution session must not expose {persistence_field}"
            );
        }
    }
}
