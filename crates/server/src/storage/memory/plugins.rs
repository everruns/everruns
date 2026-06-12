// In-memory plugin marketplace and install storage.

use super::super::models::*;
use super::{InMemoryDatabase, matches_search_tokens};
use anyhow::Result;
use uuid::Uuid;

impl InMemoryDatabase {
    // ========================================================================
    // Plugin Marketplaces
    // ========================================================================

    pub async fn create_plugin_marketplace(
        &self,
        org_id: i64,
        input: CreatePluginMarketplaceRow,
    ) -> Result<PluginMarketplaceRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = PluginMarketplaceRow {
            id,
            org_id,
            public_id: input.public_id,
            name: input.name,
            source_type: input.source_type,
            source: input.source,
            status: "active".to_string(),
            catalog: None,
            last_synced_at: None,
            last_synced_sha: None,
            created_at: now,
            updated_at: now,
        };
        self.plugin_marketplaces.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_plugin_marketplace(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<PluginMarketplaceRow>> {
        Ok(self
            .plugin_marketplaces
            .read()
            .get(&id)
            .filter(|r| r.org_id == org_id)
            .cloned())
    }

    pub async fn get_plugin_marketplace_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<PluginMarketplaceRow>> {
        Ok(self
            .plugin_marketplaces
            .read()
            .values()
            .find(|r| r.org_id == org_id && r.public_id == public_id)
            .cloned())
    }

    pub async fn list_plugin_marketplaces(
        &self,
        org_id: i64,
        search: Option<&str>,
    ) -> Result<Vec<PluginMarketplaceRow>> {
        let mut rows: Vec<_> = self
            .plugin_marketplaces
            .read()
            .values()
            .filter(|r| r.org_id == org_id)
            .filter(|r| matches_search_tokens(search, &[&r.name]))
            .cloned()
            .collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.created_at));
        Ok(rows)
    }

    pub async fn update_plugin_marketplace(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdatePluginMarketplace,
    ) -> Result<Option<PluginMarketplaceRow>> {
        let mut rows = self.plugin_marketplaces.write();
        let Some(row) = rows.get_mut(&id) else {
            return Ok(None);
        };
        if row.org_id != org_id {
            return Ok(None);
        }
        if let Some(name) = input.name {
            row.name = name;
        }
        if let Some(source_type) = input.source_type {
            row.source_type = source_type;
        }
        if let Some(source) = input.source {
            row.source = source;
        }
        if let Some(status) = input.status {
            row.status = status;
        }
        if let Some(catalog) = input.catalog {
            row.catalog = Some(catalog);
        }
        if let Some(last_synced_at) = input.last_synced_at {
            row.last_synced_at = Some(last_synced_at);
        }
        if let Some(sha_opt) = input.last_synced_sha {
            row.last_synced_sha = sha_opt;
        }
        row.updated_at = Self::now();
        Ok(Some(row.clone()))
    }

    pub async fn delete_plugin_marketplace(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let mut rows = self.plugin_marketplaces.write();
        let Some(row) = rows.get(&id) else {
            return Ok(false);
        };
        if row.org_id != org_id {
            return Ok(false);
        }
        rows.remove(&id);
        // Also clear marketplace_id on installs that reference this marketplace.
        drop(rows);
        for install in self.plugin_installs.write().values_mut() {
            if install.marketplace_id == Some(id) {
                install.marketplace_id = None;
            }
        }
        Ok(true)
    }

    // ========================================================================
    // Plugin Installs
    // ========================================================================

    pub async fn create_plugin_install(
        &self,
        org_id: i64,
        input: CreatePluginInstallRow,
    ) -> Result<PluginInstallRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = PluginInstallRow {
            id,
            org_id,
            public_id: input.public_id,
            name: input.name,
            marketplace_id: input.marketplace_id,
            source: input.source,
            version: input.version,
            pinned_sha: input.pinned_sha,
            manifest: input.manifest,
            definition: input.definition,
            warnings: input.warnings,
            status: "active".to_string(),
            created_at: now,
            updated_at: now,
        };
        self.plugin_installs.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_plugin_install(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<PluginInstallRow>> {
        Ok(self
            .plugin_installs
            .read()
            .get(&id)
            .filter(|r| r.org_id == org_id)
            .cloned())
    }

    pub async fn get_plugin_install_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<PluginInstallRow>> {
        Ok(self
            .plugin_installs
            .read()
            .values()
            .find(|r| r.org_id == org_id && r.public_id == public_id)
            .cloned())
    }

    pub async fn get_plugin_install_by_name(
        &self,
        org_id: i64,
        name: &str,
    ) -> Result<Option<PluginInstallRow>> {
        Ok(self
            .plugin_installs
            .read()
            .values()
            .find(|r| r.org_id == org_id && r.name == name)
            .cloned())
    }

    pub async fn list_plugin_installs(
        &self,
        org_id: i64,
        search: Option<&str>,
    ) -> Result<Vec<PluginInstallRow>> {
        let mut rows: Vec<_> = self
            .plugin_installs
            .read()
            .values()
            .filter(|r| r.org_id == org_id)
            .filter(|r| matches_search_tokens(search, &[&r.name]))
            .cloned()
            .collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.created_at));
        Ok(rows)
    }

    pub async fn list_active_plugin_installs(&self, org_id: i64) -> Result<Vec<PluginInstallRow>> {
        let mut rows: Vec<_> = self
            .plugin_installs
            .read()
            .values()
            .filter(|r| r.org_id == org_id && r.status == "active")
            .cloned()
            .collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.created_at));
        Ok(rows)
    }

    pub async fn update_plugin_install(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdatePluginInstall,
    ) -> Result<Option<PluginInstallRow>> {
        let mut rows = self.plugin_installs.write();
        let Some(row) = rows.get_mut(&id) else {
            return Ok(None);
        };
        if row.org_id != org_id {
            return Ok(None);
        }
        if let Some(status) = input.status {
            row.status = status;
        }
        if let Some(source) = input.source {
            row.source = source;
        }
        if let Some(version) = input.version {
            row.version = Some(version);
        }
        if let Some(pinned_sha) = input.pinned_sha {
            row.pinned_sha = Some(pinned_sha);
        }
        if let Some(manifest) = input.manifest {
            row.manifest = manifest;
        }
        if let Some(definition) = input.definition {
            row.definition = definition;
        }
        if let Some(warnings) = input.warnings {
            row.warnings = warnings;
        }
        row.updated_at = Self::now();
        Ok(Some(row.clone()))
    }

    pub async fn delete_plugin_install(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let mut rows = self.plugin_installs.write();
        let exists = rows.get(&id).is_some_and(|r| r.org_id == org_id);
        if exists {
            rows.remove(&id);
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
