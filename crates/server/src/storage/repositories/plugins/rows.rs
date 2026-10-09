// Rows the plugins repository reads and writes.

use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct PluginMarketplaceRow {
    pub id: Uuid,
    pub org_id: i64,
    pub public_id: String,
    pub name: String,
    pub source_type: String,
    pub source: serde_json::Value,
    pub status: String,
    pub catalog: Option<serde_json::Value>,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub last_synced_sha: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreatePluginMarketplaceRow {
    pub public_id: String,
    pub name: String,
    pub source_type: String,
    pub source: serde_json::Value,
}

#[derive(Debug, Clone, Default)]
pub struct UpdatePluginMarketplace {
    pub name: Option<String>,
    pub source_type: Option<String>,
    pub source: Option<serde_json::Value>,
    pub status: Option<String>,
    pub catalog: Option<serde_json::Value>,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub last_synced_sha: Option<Option<String>>,
}

#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct PluginInstallRow {
    pub id: Uuid,
    pub org_id: i64,
    pub public_id: String,
    pub name: String,
    pub marketplace_id: Option<Uuid>,
    pub source: serde_json::Value,
    pub version: Option<String>,
    pub pinned_sha: Option<String>,
    pub manifest: serde_json::Value,
    pub definition: serde_json::Value,
    pub warnings: serde_json::Value,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreatePluginInstallRow {
    pub public_id: String,
    pub name: String,
    pub marketplace_id: Option<Uuid>,
    pub source: serde_json::Value,
    pub version: Option<String>,
    pub pinned_sha: Option<String>,
    pub manifest: serde_json::Value,
    pub definition: serde_json::Value,
    pub warnings: serde_json::Value,
}

#[derive(Debug, Clone, Default)]
pub struct UpdatePluginInstall {
    pub status: Option<String>,
    pub source: Option<serde_json::Value>,
    pub version: Option<String>,
    pub pinned_sha: Option<String>,
    pub manifest: Option<serde_json::Value>,
    pub definition: Option<serde_json::Value>,
    pub warnings: Option<serde_json::Value>,
}
