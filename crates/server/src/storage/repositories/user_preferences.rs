// PostgreSQL repository: User Preferences (per-user key/value store)

use super::super::models::*;
use super::Database;
use anyhow::Result;
use uuid::Uuid;

impl Database {
    // ============================================
    // User Preferences
    // ============================================

    /// List all preferences for a user, ordered by key.
    pub async fn list_user_preferences(&self, user_id: Uuid) -> Result<Vec<UserPreferenceRow>> {
        let rows = sqlx::query_as::<_, UserPreferenceRow>(
            r#"
            SELECT id, user_id, key, value, created_at, updated_at
            FROM user_preferences
            WHERE user_id = $1
            ORDER BY key ASC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Get a single preference for a user by key.
    pub async fn get_user_preference(
        &self,
        user_id: Uuid,
        key: &str,
    ) -> Result<Option<UserPreferenceRow>> {
        let row = sqlx::query_as::<_, UserPreferenceRow>(
            r#"
            SELECT id, user_id, key, value, created_at, updated_at
            FROM user_preferences
            WHERE user_id = $1 AND key = $2
            "#,
        )
        .bind(user_id)
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Create or update a preference value for a user by key.
    pub async fn set_user_preference(
        &self,
        user_id: Uuid,
        key: &str,
        value: &str,
    ) -> Result<UserPreferenceRow> {
        let row = sqlx::query_as::<_, UserPreferenceRow>(
            r#"
            INSERT INTO user_preferences (user_id, key, value)
            VALUES ($1, $2, $3)
            ON CONFLICT (user_id, key)
            DO UPDATE SET value = EXCLUDED.value, updated_at = NOW()
            RETURNING id, user_id, key, value, created_at, updated_at
            "#,
        )
        .bind(user_id)
        .bind(key)
        .bind(value)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Delete a user's preference by key. Returns true when a row was removed.
    pub async fn delete_user_preference(&self, user_id: Uuid, key: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM user_preferences WHERE user_id = $1 AND key = $2")
            .bind(user_id)
            .bind(key)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }
}
