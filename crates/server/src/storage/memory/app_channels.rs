// In-memory storage: App Channel CRUD

use super::super::models::*;
use super::InMemoryDatabase;
use anyhow::Result;
use uuid::Uuid;

impl InMemoryDatabase {
    // ============================================
    // App Channel CRUD
    // ============================================

    pub async fn create_app_channel(
        &self,
        app_id: Uuid,
        input: CreateAppChannelRow,
    ) -> Result<AppChannelRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = AppChannelRow {
            id,
            app_id,
            public_id: input.public_id,
            channel_type: input.channel_type,
            channel_config: input.channel_config,
            channel_config_encrypted: input.channel_config_encrypted,
            enabled: input.enabled,
            created_at: now,
            updated_at: now,
        };
        self.app_channels.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn list_app_channels(&self, app_id: Uuid) -> Result<Vec<AppChannelRow>> {
        let channels = self.app_channels.read();
        let mut result: Vec<AppChannelRow> = channels
            .values()
            .filter(|ch| ch.app_id == app_id)
            .cloned()
            .collect();
        result.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(result)
    }

    pub async fn get_app_channel_by_public_id(
        &self,
        public_id: &str,
    ) -> Result<Option<AppChannelRow>> {
        let channels = self.app_channels.read();
        Ok(channels
            .values()
            .find(|ch| ch.public_id == public_id)
            .cloned())
    }

    pub async fn update_app_channel(
        &self,
        id: Uuid,
        input: UpdateAppChannel,
    ) -> Result<Option<AppChannelRow>> {
        let mut channels = self.app_channels.write();
        let Some(ch) = channels.get_mut(&id) else {
            return Ok(None);
        };
        if let Some(channel_type) = input.channel_type {
            ch.channel_type = channel_type;
        }
        if let Some(channel_config) = input.channel_config {
            ch.channel_config = channel_config;
        }
        if let Some(encrypted) = input.channel_config_encrypted {
            ch.channel_config_encrypted = Some(encrypted);
        }
        if let Some(enabled) = input.enabled {
            ch.enabled = enabled;
        }
        ch.updated_at = Self::now();
        Ok(Some(ch.clone()))
    }

    pub async fn delete_app_channel(&self, id: Uuid) -> Result<bool> {
        let mut channels = self.app_channels.write();
        Ok(channels.remove(&id).is_some())
    }
}
