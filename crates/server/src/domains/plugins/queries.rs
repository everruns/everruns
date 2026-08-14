// Plugin query helpers — row-to-DTO conversion + update_available check.

use super::types::*;
use crate::kernel_imports::{
    everruns_provider::typed_id::PluginInstallId, everruns_provider::typed_id::PluginMarketplaceId,
    plugin_capability_id,
};

pub fn row_to_marketplace(row: &PluginMarketplaceRow) -> PluginMarketplace {
    PluginMarketplace {
        public_id: row
            .public_id
            .parse()
            .unwrap_or_else(|_| PluginMarketplaceId::from_uuid(row.id)),
        internal_id: row.id,
        name: row.name.clone(),
        source_type: row.source_type.clone(),
        source: row.source.clone(),
        status: row.status.clone(),
        last_synced_at: row.last_synced_at,
        last_synced_sha: row.last_synced_sha.clone(),
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

/// Derive `update_available` by comparing the installed version/sha against the
/// marketplace catalog entry for this plugin.
///
/// Returns `true` when:
/// - There is a catalog version that differs from the installed version, OR
/// - The source type is git-based and the catalog's SHA (if present) differs.
///
/// Returns `false` when there is no catalog or no matching entry.
pub fn compute_update_available(
    install: &PluginInstallRow,
    marketplace: Option<&PluginMarketplaceRow>,
) -> bool {
    let Some(marketplace) = marketplace else {
        return false;
    };
    let Some(catalog) = &marketplace.catalog else {
        return false;
    };

    // Find the matching plugin entry in the catalog.
    let plugins = catalog.get("plugins").and_then(|v| v.as_array());
    let entry = plugins.and_then(|arr| {
        arr.iter().find(|e| {
            e.get("name")
                .and_then(|v| v.as_str())
                .is_some_and(|n| n == install.name)
        })
    });
    let Some(entry) = entry else {
        return false;
    };

    // Compare version if both sides declare one.
    if let (Some(catalog_version), Some(installed_version)) = (
        entry.get("version").and_then(|v| v.as_str()),
        &install.version,
    ) {
        return catalog_version != installed_version;
    }

    // If the marketplace resolved a SHA at last sync and it differs, update is available.
    if let Some(catalog_sha) = &marketplace.last_synced_sha {
        if let Some(installed_sha) = &install.pinned_sha {
            return catalog_sha != installed_sha;
        }
        // No pinned_sha but marketplace has a SHA → consider an update available.
        return true;
    }

    false
}

pub fn row_to_installed_plugin(
    install: &PluginInstallRow,
    marketplace: Option<&PluginMarketplaceRow>,
) -> InstalledPlugin {
    let warnings: Vec<String> =
        serde_json::from_value(install.warnings.clone()).unwrap_or_default();
    let manifest = &install.manifest;
    let display_name = manifest
        .get("displayName")
        .or_else(|| manifest.get("display_name"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let description = manifest
        .get("description")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let update_available = compute_update_available(install, marketplace);

    InstalledPlugin {
        public_id: install
            .public_id
            .parse()
            .unwrap_or_else(|_| PluginInstallId::from_uuid(install.id)),
        internal_id: install.id,
        name: install.name.clone(),
        display_name,
        description,
        version: install.version.clone(),
        pinned_sha: install.pinned_sha.clone(),
        marketplace: marketplace.map(|m| m.name.clone()),
        capability_ref: plugin_capability_id(&install.public_id),
        status: install.status.clone(),
        warnings,
        update_available,
        created_at: install.created_at,
        updated_at: install.updated_at,
    }
}
