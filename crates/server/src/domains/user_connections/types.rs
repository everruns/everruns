use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;

/// Sanitized current-user connection state. Credential material is never part
/// of the command surface.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UserConnectionInfo {
    pub provider: String,
    pub connection_type: String,
    pub provider_username: Option<String>,
    pub connected_at: DateTime<Utc>,
    pub ui_link: String,
}

/// A connector the current organization can offer to users.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ConnectionProviderInfo {
    pub provider_id: String,
    pub display_name: String,
    pub description: String,
    pub connection_type: String,
    pub ui_link: String,
}
