// Notification inbox rows.

use crate::kernel_imports::contracts::typed_id::{MessageId, NotificationId, SessionId};
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// Notification row from database
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct NotificationRow {
    pub id: NotificationId,
    pub org_id: i64,
    pub user_id: Uuid,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub href: Option<String>,
    pub payload: serde_json::Value,
    pub dedupe_key: Option<String>,
    pub occurrence_count: i32,
    pub source_type: Option<String>,
    pub source_id: Option<String>,
    pub source_name: Option<String>,
    pub viewed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating a notification
#[derive(Debug, Clone)]
pub struct CreateNotificationRow {
    pub org_id: i64,
    pub user_id: Uuid,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub href: Option<String>,
    pub payload: serde_json::Value,
    pub dedupe_key: Option<String>,
    pub source: Option<NotificationSourceRow>,
}

/// What sent a notification (an agent, or the system itself).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationSourceRow {
    /// `agent` or `system`.
    pub source_type: String,
    pub source_id: Option<String>,
    pub source_name: Option<String>,
}

/// Input for storing turn -> notification recipient mapping
#[derive(Debug, Clone)]
pub struct CreateNotificationTurnRequestRow {
    pub input_message_id: MessageId,
    pub org_id: i64,
    pub user_id: Uuid,
    pub session_id: SessionId,
}

/// Stored turn -> notification recipient mapping
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct NotificationTurnRequestRow {
    pub input_message_id: MessageId,
    pub org_id: i64,
    pub user_id: Uuid,
    pub session_id: SessionId,
    pub created_at: DateTime<Utc>,
}
