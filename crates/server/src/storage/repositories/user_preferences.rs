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
    pub async fn list_user_preferences(
        &self,
        user_id: Uuid,
        limit: usize,
    ) -> Result<Vec<UserPreferenceRow>> {
        let rows = sqlx::query_as::<_, UserPreferenceRow>(
            r#"
            SELECT id, user_id, key, value, created_at, updated_at
            FROM user_preferences
            WHERE user_id = $1
            ORDER BY key ASC
            LIMIT $2
            "#,
        )
        .bind(user_id)
        .bind(limit as i64)
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
        max_preferences: usize,
    ) -> Result<UserPreferenceRow> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
            .bind(user_id.to_string())
            .execute(&mut *transaction)
            .await?;

        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM user_preferences WHERE user_id = $1 AND key = $2)",
        )
        .bind(user_id)
        .bind(key)
        .fetch_one(&mut *transaction)
        .await?;
        if !exists {
            let count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM user_preferences WHERE user_id = $1")
                    .bind(user_id)
                    .fetch_one(&mut *transaction)
                    .await?;
            if count >= max_preferences as i64 {
                anyhow::bail!(super::super::backend::USER_PREFERENCE_LIMIT_EXCEEDED);
            }
        }

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
        .fetch_one(&mut *transaction)
        .await?;

        transaction.commit().await?;

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
