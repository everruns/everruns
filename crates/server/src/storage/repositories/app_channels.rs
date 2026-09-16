// PostgreSQL repository: App Channel CRUD
//
// Rows live in `agent_endpoints`, which is owned by an Agent rather than an App
// (EVE-1003). `app_channels` remains as a read-only view over this table for one
// release so App read paths keep working; every write goes to `agent_endpoints`
// directly, because the view cannot supply the NOT NULL columns the endpoint
// carries (`agent_id`, `owner_principal_id`).
//
// The lifted exposure policy — status, identity, version policy, owner — is
// derived from the owning App on insert, which is exactly where those values
// came from before the re-parenting, so behavior is unchanged.

use super::super::models::*;
use super::Database;
use crate::errors::BadRequestError;
use anyhow::Result;
use uuid::Uuid;

fn schedule_cap_error(max: i64, count: i64) -> anyhow::Error {
    BadRequestError::new(format!(
        "Organization may have at most {max} enabled schedule channel(s); currently has {count}"
    ))
    .into()
}

fn missing_agent_error(app_id: Uuid) -> anyhow::Error {
    BadRequestError::new(format!(
        "App {app_id} was not found or has no agent; an endpoint must be owned by an agent"
    ))
    .into()
}

/// Insert an endpoint, deriving its agent and lifted exposure policy from the
/// owning App. Selecting from `apps` rather than binding the values keeps the
/// derivation atomic with the insert. Yields no row when the App is missing or
/// still agent-less, which callers turn into `missing_agent_error`.
const INSERT_CHANNEL_SQL: &str = r#"
    INSERT INTO agent_endpoints (
        app_id, agent_id, public_id, channel_type, channel_config,
        channel_config_encrypted, durable_schedule_id, enabled,
        status, agent_identity_id, agent_version_policy, agent_version_id,
        owner_principal_id, resolved_owner_user_id
    )
    SELECT
        app.id, app.agent_id, $2, $3, $4, $5, $6, $7,
        CASE
            WHEN NOT $7 THEN 'disabled'
            WHEN app.status = 'published' THEN 'live'
            ELSE 'draft'
        END,
        app.agent_identity_id, app.agent_version_policy, app.agent_version_id,
        app.owner_principal_id, app.resolved_owner_user_id
    FROM apps AS app
    WHERE app.id = $1 AND app.agent_id IS NOT NULL
    RETURNING id, app_id, public_id, channel_type, channel_config, channel_config_encrypted, durable_schedule_id, enabled, created_at, updated_at
"#;

fn schedule_cap_lock_key(org_id: i64) -> i64 {
    // Namespace the advisory lock so schedule-cap checks for one org serialize
    // without blocking unrelated PostgreSQL advisory lock users.
    org_id ^ 0x4556_4552_5343_4844_i64
}

impl Database {
    // ============================================
    // App Channel CRUD
    // ============================================

    pub async fn create_app_channel(
        &self,
        app_id: Uuid,
        input: CreateAppChannelRow,
    ) -> Result<AppChannelRow> {
        let row = sqlx::query_as::<_, AppChannelRow>(INSERT_CHANNEL_SQL)
            .bind(app_id)
            .bind(&input.public_id)
            .bind(&input.channel_type)
            .bind(&input.channel_config)
            .bind(&input.channel_config_encrypted)
            .bind(input.durable_schedule_id)
            .bind(input.enabled)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| missing_agent_error(app_id))?;

        Ok(row)
    }

    pub async fn create_app_channel_enforcing_schedule_cap(
        &self,
        org_id: i64,
        app_id: Uuid,
        input: CreateAppChannelRow,
        max_enabled_schedule_channels: i64,
    ) -> Result<AppChannelRow> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(schedule_cap_lock_key(org_id))
            .execute(&mut *tx)
            .await?;
        let count = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*)
            FROM agent_endpoints ae
            JOIN apps a ON ae.app_id = a.id
            WHERE a.org_id = $1 AND ae.channel_type = 'schedule' AND ae.enabled = true
            "#,
        )
        .bind(org_id)
        .fetch_one(&mut *tx)
        .await?;
        if count >= max_enabled_schedule_channels {
            return Err(schedule_cap_error(max_enabled_schedule_channels, count));
        }

        let row = sqlx::query_as::<_, AppChannelRow>(INSERT_CHANNEL_SQL)
            .bind(app_id)
            .bind(&input.public_id)
            .bind(&input.channel_type)
            .bind(&input.channel_config)
            .bind(&input.channel_config_encrypted)
            .bind(input.durable_schedule_id)
            .bind(input.enabled)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| missing_agent_error(app_id))?;
        tx.commit().await?;
        Ok(row)
    }

    pub async fn list_app_channels(&self, app_id: Uuid) -> Result<Vec<AppChannelRow>> {
        let rows = sqlx::query_as::<_, AppChannelRow>(
            r#"
            SELECT id, app_id, public_id, channel_type, channel_config, channel_config_encrypted, durable_schedule_id, enabled, created_at, updated_at
            FROM agent_endpoints
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
                FROM agent_endpoints
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
            FROM agent_endpoints
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
            UPDATE agent_endpoints AS ae
            SET
                channel_type = COALESCE($2, ae.channel_type),
                channel_config = COALESCE($3, ae.channel_config),
                channel_config_encrypted = COALESCE($4, ae.channel_config_encrypted),
                durable_schedule_id = CASE WHEN $5 THEN $6 ELSE ae.durable_schedule_id END,
                enabled = COALESCE($7, ae.enabled),
                -- Publish still reads App-level status this release, so keep the
                -- derived endpoint status in step with the pair it is derived
                -- from rather than letting it drift.
                status = CASE
                    WHEN NOT COALESCE($7, ae.enabled) THEN 'disabled'
                    WHEN (SELECT a.status FROM apps AS a WHERE a.id = ae.app_id) = 'published' THEN 'live'
                    ELSE 'draft'
                END,
                updated_at = NOW()
            WHERE ae.id = $1
            RETURNING ae.id, ae.app_id, ae.public_id, ae.channel_type, ae.channel_config, ae.channel_config_encrypted, ae.durable_schedule_id, ae.enabled, ae.created_at, ae.updated_at
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

    pub async fn update_app_channel_enforcing_schedule_cap(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateAppChannel,
        max_enabled_schedule_channels: i64,
    ) -> Result<Option<AppChannelRow>> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(schedule_cap_lock_key(org_id))
            .execute(&mut *tx)
            .await?;
        let count = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*)
            FROM agent_endpoints ae
            JOIN apps a ON ae.app_id = a.id
            WHERE a.org_id = $1 AND ae.channel_type = 'schedule' AND ae.enabled = true
            "#,
        )
        .bind(org_id)
        .fetch_one(&mut *tx)
        .await?;
        if count >= max_enabled_schedule_channels {
            return Err(schedule_cap_error(max_enabled_schedule_channels, count));
        }

        let row = sqlx::query_as::<_, AppChannelRow>(
            r#"
            UPDATE agent_endpoints AS ae
            SET
                channel_type = COALESCE($2, ae.channel_type),
                channel_config = COALESCE($3, ae.channel_config),
                channel_config_encrypted = COALESCE($4, ae.channel_config_encrypted),
                durable_schedule_id = CASE WHEN $5 THEN $6 ELSE ae.durable_schedule_id END,
                enabled = COALESCE($7, ae.enabled),
                -- Publish still reads App-level status this release, so keep the
                -- derived endpoint status in step with the pair it is derived
                -- from rather than letting it drift.
                status = CASE
                    WHEN NOT COALESCE($7, ae.enabled) THEN 'disabled'
                    WHEN (SELECT a.status FROM apps AS a WHERE a.id = ae.app_id) = 'published' THEN 'live'
                    ELSE 'draft'
                END,
                updated_at = NOW()
            WHERE ae.id = $1
            RETURNING ae.id, ae.app_id, ae.public_id, ae.channel_type, ae.channel_config, ae.channel_config_encrypted, ae.durable_schedule_id, ae.enabled, ae.created_at, ae.updated_at
            "#,
        )
        .bind(id)
        .bind(&input.channel_type)
        .bind(&input.channel_config)
        .bind(&input.channel_config_encrypted)
        .bind(input.durable_schedule_id.is_changed())
        .bind(input.durable_schedule_id.into_value())
        .bind(input.enabled)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row)
    }

    // THREAT[TM-TENANT-012]: bare-`id` mutator — see the note on
    // `get_app_channel_by_public_id`. Callers MUST have already resolved the
    // channel under their org-scoped app.
    pub async fn delete_app_channel(&self, id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM agent_endpoints WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn count_enabled_schedule_channels_for_org(&self, org_id: i64) -> Result<i64> {
        let count = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*)
            FROM agent_endpoints ae
            JOIN apps a ON ae.app_id = a.id
            WHERE a.org_id = $1 AND ae.channel_type = 'schedule' AND ae.enabled = true
            "#,
        )
        .bind(org_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(count)
    }
}
