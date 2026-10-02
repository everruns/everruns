//! A2A push-notification configs (A2A 1.0 §3.1.7). See
//! `knowledge/integrations/a2a-channel.md`.

use chrono::{DateTime, Utc};
use everruns_provider::typed_id::SessionId;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Clone, Debug, FromRow)]
pub struct A2aPushConfigRow {
    pub id: Uuid,
    pub org_id: i64,
    /// The A2A task: an endpoint session.
    pub session_id: SessionId,
    /// `TaskPushNotificationConfig.id`, unique per task.
    pub config_id: String,
    pub url: String,
    /// `AuthenticationInfo.scheme`; the credentials live in `secrets_encrypted`.
    pub auth_scheme: Option<String>,
    /// Encrypted JSON `{ "token", "credentials" }`, `None` when the client
    /// sent neither.
    pub secrets_encrypted: Option<Vec<u8>>,
    /// A2A version the config was created under (`1.0` or `0.3`).
    pub wire_version: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct UpsertA2aPushConfig {
    pub org_id: i64,
    pub session_id: SessionId,
    pub config_id: String,
    pub url: String,
    pub auth_scheme: Option<String>,
    pub secrets_encrypted: Option<Vec<u8>>,
    pub wire_version: String,
}
