// Session schedule types
//
// A session schedule represents a future task that an agent has scheduled
// within a session. At the scheduled time, an "app" role message is injected
// into the session to trigger a new turn.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::typed_id::{AgentId, HarnessId, MessageId, SessionId};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Status of a session schedule
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SessionScheduleStatus {
    /// Waiting to be triggered
    Pending,
    /// Successfully triggered (message injected, turn started)
    Triggered,
    /// Cancelled by the agent or user
    Cancelled,
    /// Failed to trigger
    Failed,
}

impl std::fmt::Display for SessionScheduleStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Triggered => write!(f, "triggered"),
            Self::Cancelled => write!(f, "cancelled"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

impl From<&str> for SessionScheduleStatus {
    fn from(s: &str) -> Self {
        match s {
            "pending" => Self::Pending,
            "triggered" => Self::Triggered,
            "cancelled" => Self::Cancelled,
            "failed" => Self::Failed,
            _ => Self::Pending,
        }
    }
}

/// A scheduled task within a session
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SessionSchedule {
    pub id: Uuid,
    pub session_id: SessionId,
    pub organization_id: String,
    pub harness_id: HarnessId,
    pub agent_id: Option<AgentId>,
    pub description: String,
    pub scheduled_at: DateTime<Utc>,
    pub status: SessionScheduleStatus,
    pub triggered_at: Option<DateTime<Utc>>,
    pub message_id: Option<MessageId>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Input for creating a session schedule
#[derive(Debug, Clone)]
pub struct CreateSessionSchedule {
    pub session_id: SessionId,
    pub organization_id: String,
    pub harness_id: HarnessId,
    pub agent_id: Option<AgentId>,
    pub description: String,
    pub scheduled_at: DateTime<Utc>,
}
