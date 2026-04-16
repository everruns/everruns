// In-memory storage: App CRUD

use super::super::models::*;
use super::InMemoryDatabase;
use super::matches_search_tokens;
use anyhow::Result;
use uuid::Uuid;

impl InMemoryDatabase {
    // ============================================
    // App CRUD
    // ============================================

    pub async fn create_app(&self, org_id: i64, input: CreateAppRow) -> Result<AppRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = AppRow {
            id,
            org_id,
            public_id: input.public_id,
            name: input.name,
            description: input.description,
            harness_id: input.harness_id,
            agent_id: input.agent_id,
            agent_identity_id: input.agent_identity_id,
            channel_type: input.channel_type,
            channel_config: input.channel_config,
            channel_config_encrypted: input.channel_config_encrypted,
            status: "draft".to_string(),
            published_at: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
            deleted_at: None,
        };
        self.apps.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_app_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<AppRow>> {
        let apps = self.apps.read();
        Ok(apps
            .values()
            .find(|a| a.org_id == org_id && a.public_id == public_id)
            .cloned())
    }

    /// Lookup app by public_id without org scoping (for unauthenticated webhooks).
    pub async fn get_app_by_public_id_unscoped(&self, public_id: &str) -> Result<Option<AppRow>> {
        let apps = self.apps.read();
        Ok(apps.values().find(|a| a.public_id == public_id).cloned())
    }

    pub async fn list_apps(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<AppRow>> {
        let apps = self.apps.read();
        let mut result: Vec<AppRow> = apps
            .values()
            .filter(|a| {
                a.org_id == org_id
                    && if include_archived {
                        a.status != "deleted"
                    } else {
                        a.status != "archived" && a.status != "deleted"
                    }
            })
            .filter(|a| {
                matches_search_tokens(search, &[&a.name, a.description.as_deref().unwrap_or("")])
            })
            .cloned()
            .collect();
        result.sort_by_key(|app| std::cmp::Reverse(app.created_at));
        Ok(result)
    }

    pub async fn update_app(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateApp,
    ) -> Result<Option<AppRow>> {
        let mut apps = self.apps.write();
        let Some(app) = apps.get_mut(&id) else {
            return Ok(None);
        };
        if app.org_id != org_id {
            return Ok(None);
        }
        if let Some(name) = input.name {
            app.name = name;
        }
        if let Some(description) = input.description {
            app.description = Some(description);
        }
        if let Some(harness_id) = input.harness_id {
            app.harness_id = harness_id;
        }
        if let Some(agent_id) = input.agent_id {
            app.agent_id = agent_id;
        }
        input.agent_identity_id.apply(&mut app.agent_identity_id);
        if let Some(channel_type) = input.channel_type {
            app.channel_type = channel_type;
        }
        if let Some(channel_config) = input.channel_config {
            app.channel_config = channel_config;
        }
        if let Some(encrypted) = input.channel_config_encrypted {
            app.channel_config_encrypted = Some(encrypted);
        }
        if let Some(status) = input.status {
            app.status = status;
        }
        input.published_at.apply(&mut app.published_at);
        app.updated_at = Self::now();
        Ok(Some(app.clone()))
    }

    pub async fn delete_app(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let mut apps = self.apps.write();
        let Some(app) = apps.get_mut(&id) else {
            return Ok(false);
        };
        if app.org_id != org_id || !matches!(app.status.as_str(), "draft" | "published") {
            return Ok(false);
        }
        app.status = "archived".to_string();
        app.archived_at = Some(Self::now());
        app.updated_at = Self::now();
        Ok(true)
    }

    pub async fn destroy_app(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let mut apps = self.apps.write();
        let Some(app) = apps.get_mut(&id) else {
            return Ok(false);
        };
        if app.org_id != org_id || app.status != "archived" {
            return Ok(false);
        }
        app.status = "deleted".to_string();
        app.deleted_at = Some(Self::now());
        app.updated_at = Self::now();
        Ok(true)
    }
}
