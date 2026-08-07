// In-memory storage: User Preferences (per-user key/value store)

use super::super::models::*;
use super::InMemoryDatabase;
use anyhow::Result;
use uuid::Uuid;

impl InMemoryDatabase {
    // ============================================
    // User Preferences
    // ============================================

    pub async fn list_user_preferences(
        &self,
        user_id: Uuid,
        limit: usize,
    ) -> Result<Vec<UserPreferenceRow>> {
        let mut prefs: Vec<_> = self
            .user_preferences
            .read()
            .values()
            .filter(|p| p.user_id == user_id)
            .cloned()
            .collect();
        prefs.sort_by(|a, b| a.key.cmp(&b.key));
        prefs.truncate(limit);
        Ok(prefs)
    }

    pub async fn get_user_preference(
        &self,
        user_id: Uuid,
        key: &str,
    ) -> Result<Option<UserPreferenceRow>> {
        Ok(self
            .user_preferences
            .read()
            .get(&(user_id, key.to_string()))
            .cloned())
    }

    pub async fn set_user_preference(
        &self,
        user_id: Uuid,
        key: &str,
        value: &str,
        max_preferences: usize,
    ) -> Result<UserPreferenceRow> {
        let now = Self::now();
        let mut prefs = self.user_preferences.write();
        let map_key = (user_id, key.to_string());

        if !prefs.contains_key(&map_key)
            && prefs.values().filter(|row| row.user_id == user_id).count() >= max_preferences
        {
            anyhow::bail!(super::super::backend::USER_PREFERENCE_LIMIT_EXCEEDED);
        }

        let row = match prefs.get(&map_key) {
            Some(existing) => UserPreferenceRow {
                value: value.to_string(),
                updated_at: now,
                ..existing.clone()
            },
            None => UserPreferenceRow {
                id: Uuid::now_v7(),
                user_id,
                key: key.to_string(),
                value: value.to_string(),
                created_at: now,
                updated_at: now,
            },
        };
        prefs.insert(map_key, row.clone());
        Ok(row)
    }

    pub async fn delete_user_preference(&self, user_id: Uuid, key: &str) -> Result<bool> {
        Ok(self
            .user_preferences
            .write()
            .remove(&(user_id, key.to_string()))
            .is_some())
    }
}
