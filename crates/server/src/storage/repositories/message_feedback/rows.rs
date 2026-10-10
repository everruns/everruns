// Rows the message feedback repository reads and writes.

use chrono::{DateTime, Utc};
use everruns_server_macros::Columns;
use sqlx::FromRow;
use uuid::Uuid;

/// One person's rating of one message.
#[derive(Debug, Clone, FromRow, Columns)]
pub struct MessageFeedbackRow {
    pub id: Uuid,
    pub org_id: i64,
    pub session_id: Uuid,
    /// Public message id (`msg_...`) from the message event.
    pub message_id: String,
    pub user_id: Uuid,
    /// `good` or `bad`.
    pub rating: String,
    pub comment: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
