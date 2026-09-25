use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Clone, Debug, FromRow)]
pub struct OrgSlackConnectionRow {
    pub id: Uuid,
    pub org_id: i64,
    pub access_token_encrypted: Option<Vec<u8>>,
    pub refresh_token_encrypted: Option<Vec<u8>>,
    pub access_token_expires_at: Option<DateTime<Utc>>,
    pub state: String,
    pub token_generation: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct UpsertOrgSlackConnection {
    pub org_id: i64,
    pub access_token_encrypted: Vec<u8>,
    pub refresh_token_encrypted: Vec<u8>,
    pub access_token_expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct RotateOrgSlackConnection {
    pub org_id: i64,
    pub expected_generation: i64,
    pub access_token_encrypted: Vec<u8>,
    pub refresh_token_encrypted: Vec<u8>,
    pub access_token_expires_at: DateTime<Utc>,
}
