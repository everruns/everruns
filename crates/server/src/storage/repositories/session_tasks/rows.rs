// Rows the session tasks repository reads and writes.

use crate::kernel_imports::contracts::typed_id::SessionId;
use chrono::{DateTime, Utc};
use sqlx::FromRow;

/// Row in `session_tasks`. JSONB columns serialize the corresponding
/// `everruns_core::session_task` types; conversions live on this struct so
/// both backends share identical (de)serialization.
#[derive(Debug, Clone, FromRow)]
pub struct SessionTaskRow {
    pub id: String,
    pub session_id: SessionId,
    /// Root of the owning session's delegation tree (EVE-680). Denormalized
    /// from `sessions.root_session_id` at insert so `GET /v1/tasks` can filter a
    /// whole tree's work by a local column. DB-only — not part of the core
    /// `SessionTask`; `#[sqlx(default)]` so queries that omit it still map.
    #[sqlx(default)]
    pub root_session_id: Option<SessionId>,
    pub kind: String,
    pub display_name: String,
    pub spec: serde_json::Value,
    pub state: String,
    pub state_detail: Option<String>,
    pub progress: Option<serde_json::Value>,
    pub input_request: Option<serde_json::Value>,
    pub cancel_requested_at: Option<DateTime<Utc>>,
    pub summary: Option<String>,
    pub result_path: Option<String>,
    pub artifacts: serde_json::Value,
    pub error: Option<serde_json::Value>,
    pub attempt: i32,
    pub worker_id: Option<String>,
    pub heartbeat_at: Option<DateTime<Utc>>,
    pub links: serde_json::Value,
    pub wake_policy: String,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

impl SessionTaskRow {
    pub fn from_task(task: &everruns_core::SessionTask) -> anyhow::Result<Self> {
        use serde_json::to_value;
        Ok(Self {
            id: task.id.clone(),
            session_id: task.session_id,
            root_session_id: task.root_session_id,
            kind: task.kind.clone(),
            display_name: task.display_name.clone(),
            spec: task.spec.clone(),
            state: task.state.to_string(),
            state_detail: task.state_detail.clone(),
            progress: task.progress.as_ref().map(to_value).transpose()?,
            input_request: task.input_request.as_ref().map(to_value).transpose()?,
            cancel_requested_at: task.cancel_requested_at,
            summary: task.summary.clone(),
            result_path: task.result_path.clone(),
            artifacts: to_value(&task.artifacts)?,
            error: task.error.as_ref().map(to_value).transpose()?,
            attempt: task.attempt,
            worker_id: task.worker_id.clone(),
            heartbeat_at: task.heartbeat_at,
            links: to_value(&task.links)?,
            wake_policy: to_value(task.wake_policy)?
                .as_str()
                .unwrap_or("silent")
                .to_string(),
            created_at: task.created_at,
            started_at: task.started_at,
            finished_at: task.finished_at,
            updated_at: task.updated_at,
        })
    }

    pub fn to_task(&self) -> anyhow::Result<everruns_core::SessionTask> {
        use serde_json::from_value;
        Ok(everruns_core::SessionTask {
            id: self.id.clone(),
            session_id: self.session_id,
            root_session_id: self.root_session_id,
            kind: self.kind.clone(),
            display_name: self.display_name.clone(),
            spec: self.spec.clone(),
            state: everruns_core::SessionTaskState::from(self.state.as_str()),
            state_detail: self.state_detail.clone(),
            progress: self.progress.clone().map(from_value).transpose()?,
            input_request: self.input_request.clone().map(from_value).transpose()?,
            cancel_requested_at: self.cancel_requested_at,
            summary: self.summary.clone(),
            result_path: self.result_path.clone(),
            artifacts: from_value(self.artifacts.clone())?,
            error: self.error.clone().map(from_value).transpose()?,
            attempt: self.attempt,
            worker_id: self.worker_id.clone(),
            heartbeat_at: self.heartbeat_at,
            links: from_value(self.links.clone())?,
            wake_policy: from_value(serde_json::Value::String(self.wake_policy.clone()))
                .unwrap_or_default(),
            created_at: self.created_at,
            started_at: self.started_at,
            finished_at: self.finished_at,
            updated_at: self.updated_at,
        })
    }
}

/// Row in `session_task_messages`.
#[derive(Debug, Clone, FromRow)]
pub struct SessionTaskMessageRow {
    pub id: String,
    pub task_id: String,
    pub session_id: SessionId,
    pub direction: String,
    pub content: serde_json::Value,
    pub in_reply_to: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl SessionTaskMessageRow {
    pub fn to_message(&self) -> anyhow::Result<everruns_core::TaskMessage> {
        Ok(everruns_core::TaskMessage {
            id: self.id.clone(),
            task_id: self.task_id.clone(),
            direction: everruns_core::TaskMessageDirection::from(self.direction.as_str()),
            content: serde_json::from_value(self.content.clone())?,
            in_reply_to: self.in_reply_to.clone(),
            created_at: self.created_at,
        })
    }
}

/// Input for inserting a task message row.
#[derive(Debug, Clone)]
pub struct NewSessionTaskMessageRow {
    pub id: String,
    pub task_id: String,
    pub session_id: SessionId,
    pub direction: String,
    pub content: serde_json::Value,
    pub in_reply_to: Option<String>,
}

/// Per-task push-notification config row (EVE-682).
///
/// Session/task-scoped outbound webhook target. Has no `org_id`: authorization
/// is via the owning session's org. `event_filter` selects which task
/// transitions deliver ('terminal', 'awaiting_input', 'message').
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SessionTaskPushConfigRow {
    pub id: i64,
    pub public_id: String,
    pub session_id: SessionId,
    pub task_id: String,
    pub url: String,
    pub secret: Option<String>,
    pub event_filter: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating a per-task push-notification config.
#[derive(Debug, Clone)]
pub struct CreateSessionTaskPushConfig {
    pub public_id: String,
    pub session_id: SessionId,
    pub task_id: String,
    pub url: String,
    pub secret: Option<String>,
    pub event_filter: Vec<String>,
}
