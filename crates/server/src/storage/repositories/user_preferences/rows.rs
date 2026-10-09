// Rows the user preferences repository reads and writes.

use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// User preference (key/value) row from database
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct UserPreferenceRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub key: String,
    pub value: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
