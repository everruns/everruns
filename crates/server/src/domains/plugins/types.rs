// Plugin domain types — API DTOs for marketplaces and installed plugins.

use crate::kernel_imports::{
    everruns_provider::typed_id::PluginInstallId, everruns_provider::typed_id::PluginMarketplaceId,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

pub use crate::storage::models::{
    CreatePluginInstallRow, CreatePluginMarketplaceRow, PluginInstallRow, PluginMarketplaceRow,
    UpdatePluginInstall, UpdatePluginMarketplace,
};

// ============================================================================
// Marketplace DTO
// ============================================================================

/// A registered plugin marketplace (org-scoped catalog source).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PluginMarketplace {
    /// Public resource ID (`plgmkt_<32-hex>`).
    #[serde(rename = "id")]
    #[schema(value_type = String, example = "plgmkt_01933b5a000070008000000000000001")]
    pub public_id: PluginMarketplaceId,
    #[serde(skip)]
    pub internal_id: Uuid,
    /// Unique name within the org (kebab-case).
    pub name: String,
    /// Source type: `github`, `url`, or `local_path`.
    pub source_type: String,
    /// Source configuration (owner/repo string, URL, or path).
    pub source: serde_json::Value,
    /// Lifecycle status: `active` or `disabled`.
    pub status: String,
    /// ISO timestamp of the last successful catalog sync.
    pub last_synced_at: Option<DateTime<Utc>>,
    /// Resolved commit SHA when the source is a GitHub repo.
    pub last_synced_sha: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Request body for creating a plugin marketplace.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreatePluginMarketplaceRequest {
    /// Unique name within the org (kebab-case, e.g. `my-org-plugins`).
    #[schema(example = "everruns-plugins")]
    pub name: String,
    /// Source type: `github`, `url`, or `local_path` (dev/test only).
    #[schema(example = "github")]
    pub source_type: String,
    /// Source value. For `github`: `"owner/repo"`. For `url`: full HTTPS URL.
    /// For `local_path` (dev only): absolute filesystem path.
    #[schema(example = "everruns/plugins")]
    pub source: String,
}

/// Request body for updating a plugin marketplace.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdatePluginMarketplaceRequest {
    pub name: Option<String>,
    pub status: Option<String>,
}

/// A single entry in a synced marketplace catalog.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MarketplaceCatalogEntry {
    /// Plugin package name.
    pub name: String,
    /// Human-readable display name.
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub version: Option<String>,
    pub author: Option<String>,
    pub category: Option<String>,
    /// Whether this plugin is already installed in the org.
    pub installed: bool,
}

// ============================================================================
// Installed Plugin DTO
// ============================================================================

/// An installed plugin that contributes a stable `plugin:{install_id}` capability.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InstalledPlugin {
    /// Public resource ID (`plugin_<32-hex>`).
    #[serde(rename = "id")]
    #[schema(value_type = String, example = "plugin_01933b5a000070008000000000000001")]
    pub public_id: PluginInstallId,
    #[serde(skip)]
    pub internal_id: Uuid,
    /// Plugin package name. This is display/discovery metadata, not capability identity.
    pub name: String,
    /// Human-readable display name from the plugin manifest.
    pub display_name: Option<String>,
    /// Short description from the plugin manifest.
    pub description: Option<String>,
    /// Plugin version from the manifest (if declared).
    pub version: Option<String>,
    /// Pinned commit SHA at install time.
    pub pinned_sha: Option<String>,
    /// Name of the marketplace this plugin was installed from.
    pub marketplace: Option<String>,
    /// Stable capability ref: `plugin:{install_id}`.
    pub capability_ref: String,
    /// Lifecycle status: `active` or `disabled`.
    pub status: String,
    /// Non-fatal install warnings (e.g. unsupported hooks/lspServers).
    pub warnings: Vec<String>,
    /// True when the marketplace catalog has a newer version or SHA.
    pub update_available: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Request body for installing a plugin from a marketplace catalog entry.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct InstallPluginRequest {
    /// Public ID of the marketplace to install from.
    #[schema(example = "plgmkt_01933b5a000070008000000000000001")]
    pub marketplace_id: String,
    /// Name of the plugin entry in the marketplace catalog.
    #[schema(example = "microsoft-docs")]
    pub plugin_name: String,
}

/// Request body for updating an installed plugin (status only; use POST .../update for recompile).
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateInstalledPluginRequest {
    /// New lifecycle status: `active` or `disabled`.
    pub status: Option<String>,
}
