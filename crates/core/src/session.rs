// Session domain types
//
// These types represent the Session entity and its status.
// Used by both API and worker crates.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

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
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionStatus::Started => write!(f, "started"),
            SessionStatus::Active => write!(f, "active"),
            SessionStatus::Idle => write!(f, "idle"),
        }
    }
}

impl From<&str> for SessionStatus {
    fn from(s: &str) -> Self {
        match s {
            "active" => SessionStatus::Active,
            "idle" => SessionStatus::Idle,
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
    /// Unique identifier for the session.
    pub id: Uuid,
    /// ID of the agent this session belongs to.
    pub agent_id: Uuid,
    /// Human-readable title for the session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Tags for organizing and filtering sessions.
    #[serde(default)]
    pub tags: Vec<String>,
    /// LLM model ID to use for this session.
    /// Overrides the agent's default model if set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<Uuid>,
    /// Current execution status of the session.
    pub status: SessionStatus,
    /// Timestamp when the session was created.
    pub created_at: DateTime<Utc>,
    /// Timestamp when the session started executing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    /// Timestamp when the session finished (completed or failed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
}
