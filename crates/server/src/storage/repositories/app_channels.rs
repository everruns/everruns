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
            INSERT INTO app_channels (app_id, public_id, channel_type, channel_config, channel_config_encrypted, durable_schedule_id, enabled)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING id, app_id, public_id, channel_type, channel_config, channel_config_encrypted, durable_schedule_id, enabled, created_at, updated_at
            "#,
        )
        .bind(app_id)
        .bind(&input.public_id)
        .bind(&input.channel_type)
        .bind(&input.channel_config)
        .bind(&input.channel_config_encrypted)
        .bind(input.durable_schedule_id)
        .bind(input.enabled)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_app_channels(&self, app_id: Uuid) -> Result<Vec<AppChannelRow>> {
        let rows = sqlx::query_as::<_, AppChannelRow>(
            r#"
            SELECT id, app_id, public_id, channel_type, channel_config, channel_config_encrypted, durable_schedule_id, enabled, created_at, updated_at
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

    pub async fn app_has_channels(&self, app_id: Uuid) -> Result<bool> {
        let exists = sqlx::query_scalar::<_, bool>(
            r#"
            SELECT EXISTS (
                SELECT 1
                FROM app_channels
                WHERE app_id = $1
            )
            "#,
        )
        .bind(app_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(exists)
    }

    // THREAT[TM-TENANT-012]: `get_app_channel_by_public_id` /
    // `update_app_channel` / `delete_app_channel` take a bare global identifier
    // (`public_id` / row `id`) with no `app_id`/`org_id` filter, and the row
    // carries `channel_config_encrypted` (secrets). They do NOT enforce tenant
    // isolation on their own. Every caller MUST first fetch the parent app under
    // the caller's org and then assert `channel_row.app_id == app.id` before
    // reading config or mutating — see `domains/apps/commands.rs`
    // (`get_app_channel_by_public_id` callers all gate on that equality). Do not
    // add a caller that skips the org-scoped parent fetch.
    pub async fn get_app_channel_by_public_id(
        &self,
        public_id: &str,
    ) -> Result<Option<AppChannelRow>> {
        let row = sqlx::query_as::<_, AppChannelRow>(
            r#"
            SELECT id, app_id, public_id, channel_type, channel_config, channel_config_encrypted, durable_schedule_id, enabled, created_at, updated_at
            FROM app_channels
            WHERE public_id = $1
            "#,
        )
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    // THREAT[TM-TENANT-012]: bare-`id` mutator — see the note on
    // `get_app_channel_by_public_id`. Callers MUST have already resolved the
    // channel under their org-scoped app.
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
                durable_schedule_id = CASE WHEN $5 THEN $6 ELSE durable_schedule_id END,
                enabled = COALESCE($7, enabled),
                updated_at = NOW()
            WHERE id = $1
            RETURNING id, app_id, public_id, channel_type, channel_config, channel_config_encrypted, durable_schedule_id, enabled, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(&input.channel_type)
        .bind(&input.channel_config)
        .bind(&input.channel_config_encrypted)
        .bind(input.durable_schedule_id.is_changed())
        .bind(input.durable_schedule_id.into_value())
        .bind(input.enabled)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    // THREAT[TM-TENANT-012]: bare-`id` mutator — see the note on
    // `get_app_channel_by_public_id`. Callers MUST have already resolved the
    // channel under their org-scoped app.
    pub async fn delete_app_channel(&self, id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM app_channels WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn count_enabled_schedule_channels_for_org(&self, org_id: i64) -> Result<i64> {
        let count = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*)
            FROM app_channels ac
            JOIN apps a ON ac.app_id = a.id
            WHERE a.org_id = $1 AND ac.channel_type = 'schedule' AND ac.enabled = true
            "#,
        )
        .bind(org_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(count)
    }
}
