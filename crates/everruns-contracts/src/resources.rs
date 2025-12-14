// Core resource DTOs for public API (auth, threads, runs)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// User authenticated via Google OAuth (future implementation)
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub roles: Vec<String>,
}

/// Thread represents a conversation context for agent runs
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Thread {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
}

/// Message in a thread conversation
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Message {
    pub id: Uuid,
    pub thread_id: Uuid,
    pub role: String,
    pub content: String,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

/// Run represents a single execution of an agent on a thread
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Run {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub agent_version: i32,
    pub thread_id: Uuid,
    pub status: RunStatus,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// Status of a run execution
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl std::fmt::Display for RunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunStatus::Pending => write!(f, "pending"),
            RunStatus::Running => write!(f, "running"),
            RunStatus::Completed => write!(f, "completed"),
            RunStatus::Failed => write!(f, "failed"),
            RunStatus::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::str::FromStr for RunStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(RunStatus::Pending),
            "running" => Ok(RunStatus::Running),
            "completed" => Ok(RunStatus::Completed),
            "failed" => Ok(RunStatus::Failed),
            "cancelled" => Ok(RunStatus::Cancelled),
            _ => Err(format!("Unknown run status: {}", s)),
        }
    }
}

/// Action represents a user interaction with a run (HITL approve/deny, cancel, etc.)
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Action {
    pub id: Uuid,
    pub run_id: Uuid,
    pub kind: ActionKind,
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub by_user_id: Option<Uuid>,
}

/// Kind of action a user can take on a run
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Approve,
    Deny,
    Resume,
    Cancel,
}
