// In-memory storage: User Preferences (per-user key/value store)

use super::super::models::*;
use super::InMemoryDatabase;
use anyhow::Result;
use uuid::Uuid;

impl InMemoryDatabase {
    // ============================================
    // User Preferences
    // ============================================

    pub async fn list_virtual_user_preferences(
        &self,
        virtual_user_id: everruns_provider::typed_id::VirtualUserId,
        limit: usize,
    ) -> Result<Vec<VirtualUserPreferenceRow>> {
        let mut prefs: Vec<_> = self
            .virtual_user_preferences
            .read()
            .values()
            .filter(|p| p.virtual_user_id == virtual_user_id)
            .cloned()
            .collect();
        prefs.sort_by(|a, b| a.key.cmp(&b.key));
        prefs.truncate(limit);
        Ok(prefs)
    }

    pub async fn get_virtual_user_preference(
        &self,
        virtual_user_id: everruns_provider::typed_id::VirtualUserId,
        key: &str,
    ) -> Result<Option<VirtualUserPreferenceRow>> {
        Ok(self
            .virtual_user_preferences
            .read()
            .get(&(virtual_user_id, key.to_string()))
            .cloned())
    }

    pub async fn set_virtual_user_preference(
        &self,
        virtual_user_id: everruns_provider::typed_id::VirtualUserId,
        key: &str,
        value: &str,
        max_preferences: usize,
    ) -> Result<VirtualUserPreferenceRow> {
        let now = Self::now();
        let mut prefs = self.virtual_user_preferences.write();
        let map_key = (virtual_user_id, key.to_string());

        if !prefs.contains_key(&map_key)
            && prefs
                .values()
                .filter(|row| row.virtual_user_id == virtual_user_id)
                .count()
                >= max_preferences
        {
            anyhow::bail!(super::super::backend::USER_PREFERENCE_LIMIT_EXCEEDED);
        }

        let row = match prefs.get(&map_key) {
            Some(existing) => VirtualUserPreferenceRow {
                value: value.to_string(),
                updated_at: now,
                ..existing.clone()
            },
            None => VirtualUserPreferenceRow {
                id: Uuid::now_v7(),
                virtual_user_id,
                key: key.to_string(),
                value: value.to_string(),
                created_at: now,
                updated_at: now,
            },
        };
        prefs.insert(map_key, row.clone());
        Ok(row)
    }

    pub async fn delete_virtual_user_preference(
        &self,
        virtual_user_id: everruns_provider::typed_id::VirtualUserId,
        key: &str,
    ) -> Result<bool> {
        Ok(self
            .virtual_user_preferences
            .write()
            .remove(&(virtual_user_id, key.to_string()))
            .is_some())
    }
}
