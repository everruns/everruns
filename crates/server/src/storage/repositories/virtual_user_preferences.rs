// PostgreSQL repository: User Preferences (per-user key/value store)

use super::super::VirtualUserPreferenceRow;
use super::Database;
use anyhow::Result;
use everruns_server_macros::sql;

impl Database {
    // ============================================
    // User Preferences
    // ============================================

    /// List all preferences for a user, ordered by key.
    pub async fn list_virtual_user_preferences(
        &self,
        virtual_user_id: everruns_contracts::typed_id::VirtualUserId,
        limit: usize,
    ) -> Result<Vec<VirtualUserPreferenceRow>> {
        let rows = sqlx::query_as::<_, VirtualUserPreferenceRow>(sql!(
            r#"
            SELECT {VirtualUserPreferenceRow}
            FROM virtual_user_preferences
            WHERE virtual_user_id = $1
            ORDER BY key ASC
            LIMIT $2
            "#
        ))
        .bind(virtual_user_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Get a single preference for a user by key.
    pub async fn get_virtual_user_preference(
        &self,
        virtual_user_id: everruns_contracts::typed_id::VirtualUserId,
        key: &str,
    ) -> Result<Option<VirtualUserPreferenceRow>> {
        let row = sqlx::query_as::<_, VirtualUserPreferenceRow>(sql!(
            r#"
            SELECT {VirtualUserPreferenceRow}
            FROM virtual_user_preferences
            WHERE virtual_user_id = $1 AND key = $2
            "#
        ))
        .bind(virtual_user_id)
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Create or update a preference value for a user by key.
    pub async fn set_virtual_user_preference(
        &self,
        virtual_user_id: everruns_contracts::typed_id::VirtualUserId,
        key: &str,
        value: &str,
        max_preferences: usize,
    ) -> Result<VirtualUserPreferenceRow> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
            .bind(virtual_user_id.to_string())
            .execute(&mut *transaction)
            .await?;

        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM virtual_user_preferences WHERE virtual_user_id = $1 AND key = $2)",
        )
        .bind(virtual_user_id)
        .bind(key)
        .fetch_one(&mut *transaction)
        .await?;
        if !exists {
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM virtual_user_preferences WHERE virtual_user_id = $1",
            )
            .bind(virtual_user_id)
            .fetch_one(&mut *transaction)
            .await?;
            if count >= max_preferences as i64 {
                anyhow::bail!(super::super::backend::USER_PREFERENCE_LIMIT_EXCEEDED);
            }
        }

        let row = sqlx::query_as::<_, VirtualUserPreferenceRow>(sql!(
            r#"
            INSERT INTO virtual_user_preferences (virtual_user_id, key, value)
            VALUES ($1, $2, $3)
            ON CONFLICT (virtual_user_id, key)
            DO UPDATE SET value = EXCLUDED.value, updated_at = NOW()
            RETURNING {VirtualUserPreferenceRow}
            "#
        ))
        .bind(virtual_user_id)
        .bind(key)
        .bind(value)
        .fetch_one(&mut *transaction)
        .await?;

        transaction.commit().await?;

        Ok(row)
    }

    /// Delete a user's preference by key. Returns true when a row was removed.
    pub async fn delete_virtual_user_preference(
        &self,
        virtual_user_id: everruns_contracts::typed_id::VirtualUserId,
        key: &str,
    ) -> Result<bool> {
        let result = sqlx::query(
            "DELETE FROM virtual_user_preferences WHERE virtual_user_id = $1 AND key = $2",
        )
        .bind(virtual_user_id)
        .bind(key)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }
}
