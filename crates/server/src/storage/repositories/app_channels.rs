// PostgreSQL repository: App Channel CRUD

use super::super::models::*;
use super::Database;
use anyhow::Result;
use uuid::Uuid;

impl Database {
    // ============================================
    // App Channel CRUD
    // ============================================

    pub async fn create_app_channel(
        &self,
        app_id: Uuid,
        input: CreateAppChannelRow,
    ) -> Result<AppChannelRow> {
        let row = sqlx::query_as::<_, AppChannelRow>(
            r#"
            INSERT INTO app_channels (app_id, public_id, channel_type, channel_config, channel_config_encrypted, enabled)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id, app_id, public_id, channel_type, channel_config, channel_config_encrypted, enabled, created_at, updated_at
            "#,
        )
        .bind(app_id)
        .bind(&input.public_id)
        .bind(&input.channel_type)
        .bind(&input.channel_config)
        .bind(&input.channel_config_encrypted)
        .bind(input.enabled)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_app_channels(&self, app_id: Uuid) -> Result<Vec<AppChannelRow>> {
        let rows = sqlx::query_as::<_, AppChannelRow>(
            r#"
            SELECT id, app_id, public_id, channel_type, channel_config, channel_config_encrypted, enabled, created_at, updated_at
            FROM app_channels
            WHERE app_id = $1
            ORDER BY created_at ASC
            "#,
        )
        .bind(app_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn get_app_channel_by_public_id(
        &self,
        public_id: &str,
    ) -> Result<Option<AppChannelRow>> {
        let row = sqlx::query_as::<_, AppChannelRow>(
            r#"
            SELECT id, app_id, public_id, channel_type, channel_config, channel_config_encrypted, enabled, created_at, updated_at
            FROM app_channels
            WHERE public_id = $1
            "#,
        )
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn update_app_channel(
        &self,
        id: Uuid,
        input: UpdateAppChannel,
    ) -> Result<Option<AppChannelRow>> {
        let row = sqlx::query_as::<_, AppChannelRow>(
            r#"
            UPDATE app_channels
            SET
                channel_type = COALESCE($2, channel_type),
                channel_config = COALESCE($3, channel_config),
                channel_config_encrypted = COALESCE($4, channel_config_encrypted),
                enabled = COALESCE($5, enabled),
                updated_at = NOW()
            WHERE id = $1
            RETURNING id, app_id, public_id, channel_type, channel_config, channel_config_encrypted, enabled, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(&input.channel_type)
        .bind(&input.channel_config)
        .bind(&input.channel_config_encrypted)
        .bind(input.enabled)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn delete_app_channel(&self, id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM app_channels WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }
}
