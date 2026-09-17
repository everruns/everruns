// PostgreSQL repository: App Channel CRUD
//
// Rows live in `agent_endpoints`, which is owned by an Agent rather than an App
// (EVE-1003). `app_channels` remains as a read-only view over this table for one
// release so App read paths keep working; every write goes to `agent_endpoints`
// directly, because the view cannot supply the NOT NULL columns the endpoint
// carries (`agent_id`, `owner_principal_id`).
//
// The lifted identity, version policy, and owner are derived from the owning
// App on insert, which is exactly where those values came from before the
// re-parenting. A new endpoint's status is independent of App publish state.

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

/// Insert an endpoint, deriving its agent, identity, version policy, and owner
/// from the owning App. Selecting from `apps` rather than binding the values
/// keeps the derivation atomic with the insert. Yields no row when the App is
/// missing or still agent-less, which callers turn into `missing_agent_error`.
const INSERT_CHANNEL_SQL: &str = r#"
    INSERT INTO agent_endpoints (
        app_id, agent_id, public_id, channel_type, channel_config,
        channel_config_encrypted, auth, auth_encrypted, durable_schedule_id, enabled,
        status, agent_identity_id, agent_version_policy, agent_version_id,
        owner_principal_id, resolved_owner_user_id
    )
    SELECT
        app.id, app.agent_id, $2, $3, $4, $5, $6, $7, $8, $9,
        CASE WHEN $9 THEN 'draft' ELSE 'disabled' END,
        app.agent_identity_id, app.agent_version_policy, app.agent_version_id,
        app.owner_principal_id, app.resolved_owner_user_id
    FROM apps AS app
    WHERE app.id = $1 AND app.agent_id IS NOT NULL
    RETURNING id, app_id, public_id, channel_type, channel_config, channel_config_encrypted, auth, auth_encrypted, durable_schedule_id, enabled, status, created_at, updated_at
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
            .bind(&input.auth)
            .bind(&input.auth_encrypted)
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
            .bind(&input.auth)
            .bind(&input.auth_encrypted)
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
            SELECT id, app_id, public_id, channel_type, channel_config, channel_config_encrypted, auth, auth_encrypted, durable_schedule_id, enabled, status, created_at, updated_at
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
            SELECT id, app_id, public_id, channel_type, channel_config, channel_config_encrypted, auth, auth_encrypted, durable_schedule_id, enabled, status, created_at, updated_at
            FROM agent_endpoints
            WHERE public_id = $1
            "#,
        )
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_agent_endpoint_public_id(
        &self,
        org_id: i64,
        endpoint_id: Uuid,
    ) -> Result<Option<String>> {
        let public_id = sqlx::query_scalar::<_, String>(
            r#"
            SELECT ae.public_id
            FROM agent_endpoints AS ae
            JOIN agents AS a ON a.id = ae.agent_id
            WHERE a.org_id = $1 AND ae.id = $2
            "#,
        )
        .bind(org_id)
        .bind(endpoint_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(public_id)
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
                auth = CASE WHEN $5 THEN $6 ELSE ae.auth END,
                auth_encrypted = CASE WHEN $7 THEN $8 ELSE ae.auth_encrypted END,
                durable_schedule_id = CASE WHEN $9 THEN $10 ELSE ae.durable_schedule_id END,
                enabled = COALESCE($11, ae.enabled),
                -- `status` is authoritative for ingress (EVE-1007). An explicit
                -- value wins; otherwise an `enabled` change still moves it, so
                -- the App API's enable/disable cannot leave a disabled endpoint
                -- reachable while that API is still the everyday control.
                status = COALESCE($12, CASE
                    WHEN NOT COALESCE($11, ae.enabled) THEN 'disabled'
                    WHEN (SELECT a.status FROM apps AS a WHERE a.id = ae.app_id) = 'published' THEN 'live'
                    ELSE 'draft'
                END),
                updated_at = NOW()
            WHERE ae.id = $1
            RETURNING ae.id, ae.app_id, ae.public_id, ae.channel_type, ae.channel_config, ae.channel_config_encrypted, ae.auth, ae.auth_encrypted, ae.durable_schedule_id, ae.enabled, ae.status, ae.created_at, ae.updated_at
            "#,
        )
        .bind(id)
        .bind(&input.channel_type)
        .bind(&input.channel_config)
        .bind(&input.channel_config_encrypted)
        .bind(input.auth.is_changed())
        .bind(input.auth.into_value())
        .bind(input.auth_encrypted.is_changed())
        .bind(input.auth_encrypted.into_value())
        .bind(input.durable_schedule_id.is_changed())
        .bind(input.durable_schedule_id.into_value())
        .bind(input.enabled)
        .bind(&input.status)
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
                auth = CASE WHEN $5 THEN $6 ELSE ae.auth END,
                auth_encrypted = CASE WHEN $7 THEN $8 ELSE ae.auth_encrypted END,
                durable_schedule_id = CASE WHEN $9 THEN $10 ELSE ae.durable_schedule_id END,
                enabled = COALESCE($11, ae.enabled),
                -- `status` is authoritative for ingress (EVE-1007). An explicit
                -- value wins; otherwise an `enabled` change still moves it, so
                -- the App API's enable/disable cannot leave a disabled endpoint
                -- reachable while that API is still the everyday control.
                status = COALESCE($12, CASE
                    WHEN NOT COALESCE($11, ae.enabled) THEN 'disabled'
                    WHEN (SELECT a.status FROM apps AS a WHERE a.id = ae.app_id) = 'published' THEN 'live'
                    ELSE 'draft'
                END),
                updated_at = NOW()
            WHERE ae.id = $1
            RETURNING ae.id, ae.app_id, ae.public_id, ae.channel_type, ae.channel_config, ae.channel_config_encrypted, ae.auth, ae.auth_encrypted, ae.durable_schedule_id, ae.enabled, ae.status, ae.created_at, ae.updated_at
            "#,
        )
        .bind(id)
        .bind(&input.channel_type)
        .bind(&input.channel_config)
        .bind(&input.channel_config_encrypted)
        .bind(input.auth.is_changed())
        .bind(input.auth.into_value())
        .bind(input.auth_encrypted.is_changed())
        .bind(input.auth_encrypted.into_value())
        .bind(input.durable_schedule_id.is_changed())
        .bind(input.durable_schedule_id.into_value())
        .bind(input.enabled)
        .bind(&input.status)
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

    /// Agent ids, from the given set, that currently have at least one live
    /// endpoint (EVE-1007).
    ///
    /// Exposure state is derived on read and never stored: a stored flag would
    /// be a second writer for state the endpoint rows already own, and it would
    /// drift the moment an endpoint changed by any other path. Batched so a list
    /// page costs one query rather than one per agent.
    ///
    /// This answers the endpoint half only. The agent-level terms
    /// (`status`, `exposures_suspended`) are applied by the caller, which
    /// already holds the agent row.
    pub async fn agents_with_live_endpoints(
        &self,
        agent_ids: &[Uuid],
    ) -> Result<std::collections::HashSet<Uuid>> {
        if agent_ids.is_empty() {
            return Ok(std::collections::HashSet::new());
        }
        let rows = sqlx::query_scalar::<_, Uuid>(
            r#"
            SELECT DISTINCT agent_id
            FROM agent_endpoints
            WHERE agent_id = ANY($1) AND status = 'live'
            "#,
        )
        .bind(agent_ids)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().collect())
    }

    /// Bridge the App publish switch onto endpoint status (EVE-1007).
    ///
    /// Ingress reads `agent_endpoints.status` alone now, so the App-level
    /// publish/unpublish API — which is still the everyday control until the
    /// App domain is deleted — has to move the endpoints it owns. Publishing
    /// only raises endpoints the operator had enabled, and unpublishing lowers
    /// only the live ones, so an explicitly disabled endpoint stays disabled
    /// across a publish cycle.
    pub async fn set_app_endpoint_publish(&self, app_id: Uuid, published: bool) -> Result<u64> {
        let sql = if published {
            "UPDATE agent_endpoints SET status = 'live', updated_at = NOW()
             WHERE app_id = $1 AND enabled = true AND status <> 'live'"
        } else {
            "UPDATE agent_endpoints SET status = 'draft', updated_at = NOW()
             WHERE app_id = $1 AND status = 'live'"
        };
        let result = sqlx::query(sql).bind(app_id).execute(&self.pool).await?;
        Ok(result.rows_affected())
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
