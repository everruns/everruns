// Session domain types
//
// These types represent the Session entity and its status.
// Used by both API and worker crates.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::capability_types::AgentCapabilityConfig;
use crate::events::TokenUsage;
use crate::tool_types::ToolDefinition;
use crate::typed_id::{AgentId, AgentIdentityId, HarnessId, ModelId, SessionId};

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
            _ => SubagentStatus::Spawning,
        }
    }
}

/// Session execution status.
/// - `started`: Session just created, no turn executed yet
/// - `active`: A turn is currently running
/// - `idle`: Turn completed, session waiting for next input
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
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionStatus::Started => write!(f, "started"),
            SessionStatus::Active => write!(f, "active"),
            SessionStatus::Idle => write!(f, "idle"),
            SessionStatus::WaitingForToolResults => write!(f, "waiting_for_tool_results"),
        }
    }
}

impl From<&str> for SessionStatus {
    fn from(s: &str) -> Self {
        match s {
            "active" => SessionStatus::Active,
            "idle" => SessionStatus::Idle,
            "waiting_for_tool_results" => SessionStatus::WaitingForToolResults,
            // Handle legacy values during migration
            "running" => SessionStatus::Active,
            "pending" | "completed" | "failed" => SessionStatus::Idle,
            _ => SessionStatus::Started,
        }
    }
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
    /// ID of the harness for this session (format: harness_{32-hex}).
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "harness_01933b5a00007000800000000000001"))]
    pub harness_id: HarnessId,
    /// ID of the agent working in this session (format: agent_{32-hex}). Optional.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "agent_01933b5a00007000800000000000001"))]
    pub agent_id: Option<AgentId>,
    /// Optional resident agent identity for unattended/background execution.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "identity_01933b5a00007000800000000000001"))]
    pub agent_identity_id: Option<AgentIdentityId>,
    /// Human-readable title for the session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Locale for localized agent behavior and formatting (BCP 47, e.g. `uk-UA`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    /// Preview text from the first user message (truncated).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    /// Preview text from the last assistant response (truncated).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_preview: Option<String>,
    /// Tags for organizing and filtering sessions.
    #[serde(default)]
    pub tags: Vec<String>,
    /// LLM model ID to use for this session (format: model_{32-hex}).
    /// Overrides the agent's default model if set.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "model_01933b5a00007000800000000000001"))]
    pub model_id: Option<ModelId>,
    /// Session-level capabilities (additive to agent capabilities).
    /// Applied after agent capabilities when building RuntimeAgent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<AgentCapabilityConfig>,
    /// Client-side tools for this session (additive to agent tools).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDefinition>,
    /// Session-level client hints — arbitrary key-value pairs declared by the
    /// client at session creation time. These are defaults for every turn;
    /// per-message `controls.hints` override these key-by-key (shallow merge).
    ///
    /// Examples: `{"setup_connection": true, "rich_media": true}`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hints: Option<std::collections::HashMap<String, serde_json::Value>>,
    /// Current execution status of the session.
    pub status: SessionStatus,
    /// Timestamp when the session was created.
    pub created_at: DateTime<Utc>,
    /// Timestamp when the session was last updated.
    pub updated_at: DateTime<Utc>,
    /// Timestamp when the session started executing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    /// Timestamp when the session finished (completed or failed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    /// Cumulative token usage for all LLM calls in this session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    /// Whether this session is pinned by the current user.
    /// Only populated when the request has an authenticated user context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_pinned: Option<bool>,
    /// Number of active (enabled) schedules for this session.
    /// Populated when the session is fetched for API responses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_schedule_count: Option<u32>,
    /// Aggregated UI features from all active capabilities (harness + agent + session).
    /// Computed at read time from the capability registry.
    /// Known features: "file_system", "schedules", "secrets", "key_value",
    /// "sql_database", "leased_resources".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,

    // -- Subagent fields (only set when this session is a subagent) --
    /// Parent session that spawned this subagent. NULL for top-level sessions.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub parent_session_id: Option<SessionId>,
    /// Human-readable subagent name ("Test Runner"), unique per parent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subagent_name: Option<String>,
    /// Original task description given to this subagent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subagent_task: Option<String>,
    /// Subagent lifecycle status: spawning, running, completed, failed, cancelled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subagent_status: Option<SubagentStatus>,

    // -- Blueprint fields (only set when this session runs a blueprint agent) --
    /// Blueprint ID. When set, reason_activity and act_activity build RuntimeAgent
    /// from the blueprint definition instead of from harness_id/agent_id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blueprint_id: Option<String>,
    /// Validated config passed by host at blueprint spawn time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blueprint_config: Option<serde_json::Value>,
}
