// PostgreSQL repository: App Channel CRUD
//
// Rows live in `agent_endpoints`, which is owned by an Agent rather than an App
// while `app_channels` remains as a read-only compatibility view. Every
// internal write goes directly to `agent_endpoints`, because the view cannot
// supply the NOT NULL columns the endpoint carries (`agent_id`,
// `owner_principal_id`).
//
// The lifted identity, version policy, and owner are derived from the owning
// App on insert, which is exactly where those values came from before the
// re-parenting. A new endpoint's status is independent of App publish state.

use super::super::models::*;
use super::super::{CreateAgentEndpointRow, IngressEndpointRow, UpdateAgentEndpointRow};
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
        app_id, legacy_app_public_id, agent_id, public_id, channel_type, channel_config,
        channel_config_encrypted, auth, auth_encrypted, durable_schedule_id, enabled,
        status, agent_identity_id, agent_version_policy, agent_version_id,
        owner_principal_id, resolved_owner_user_id
    )
    SELECT
        app.id, app.public_id, app.agent_id, $2, $3, $4, $5, $6, $7, $8, $9,
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

    /// Hold a transaction-scoped, cross-instance lock for one Slack endpoint.
    pub async fn lock_slack_install(
        &self,
        endpoint_id: Uuid,
    ) -> Result<sqlx::Transaction<'static, sqlx::Postgres>> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "SELECT pg_advisory_xact_lock(hashtextextended('slack_install:' || $1::text, 0))",
        )
        .bind(endpoint_id)
        .execute(&mut *tx)
        .await?;
        Ok(tx)
    }

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

    pub async fn get_ingress_endpoint_by_public_id(
        &self,
        public_id: &str,
    ) -> Result<Option<IngressEndpointRow>> {
        sqlx::query_as::<_, IngressEndpointRow>(
            r#"
            SELECT
                ae.id AS endpoint_id,
                ae.public_id AS endpoint_public_id,
                ae.app_id AS legacy_app_id,
                ae.legacy_app_public_id,
                agent.org_id,
                ae.agent_id,
                agent.public_id AS agent_public_id,
                COALESCE(agent.display_name, agent.name) AS agent_name,
                agent.description AS agent_description,
                agent.harness_id,
                agent.status AS agent_status,
                agent.exposures_suspended,
                ae.agent_identity_id,
                ae.agent_version_policy,
                ae.agent_version_id,
                ae.owner_principal_id,
                ae.resolved_owner_user_id,
                ae.channel_type,
                ae.channel_config,
                ae.channel_config_encrypted,
                ae.auth,
                ae.auth_encrypted,
                ae.enabled,
                ae.status AS endpoint_status,
                ae.created_at,
                ae.updated_at
            FROM agent_endpoints AS ae
            JOIN agents AS agent ON agent.id = ae.agent_id
            WHERE ae.public_id = $1
            "#,
        )
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn list_agent_endpoints(
        &self,
        org_id: i64,
        agent_id: Uuid,
    ) -> Result<Vec<IngressEndpointRow>> {
        sqlx::query_as::<_, IngressEndpointRow>(
            r#"
            SELECT ae.id AS endpoint_id, ae.public_id AS endpoint_public_id,
                ae.app_id AS legacy_app_id, ae.legacy_app_public_id, agent.org_id,
                ae.agent_id, agent.public_id AS agent_public_id,
                COALESCE(agent.display_name, agent.name) AS agent_name,
                agent.description AS agent_description, agent.harness_id,
                agent.status AS agent_status, agent.exposures_suspended,
                ae.agent_identity_id, ae.agent_version_policy, ae.agent_version_id,
                ae.owner_principal_id, ae.resolved_owner_user_id, ae.channel_type,
                ae.channel_config, ae.channel_config_encrypted, ae.auth, ae.auth_encrypted,
                ae.enabled, ae.status AS endpoint_status, ae.created_at, ae.updated_at
            FROM agent_endpoints AS ae
            JOIN agents AS agent ON agent.id = ae.agent_id
            WHERE agent.org_id = $1 AND ae.agent_id = $2
            ORDER BY ae.created_at, ae.id
            "#,
        )
        .bind(org_id)
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn get_agent_endpoint(
        &self,
        org_id: i64,
        agent_id: Uuid,
        public_id: &str,
    ) -> Result<Option<IngressEndpointRow>> {
        sqlx::query_as::<_, IngressEndpointRow>(
            r#"
            SELECT ae.id AS endpoint_id, ae.public_id AS endpoint_public_id,
                ae.app_id AS legacy_app_id, ae.legacy_app_public_id, agent.org_id,
                ae.agent_id, agent.public_id AS agent_public_id,
                COALESCE(agent.display_name, agent.name) AS agent_name,
                agent.description AS agent_description, agent.harness_id,
                agent.status AS agent_status, agent.exposures_suspended,
                ae.agent_identity_id, ae.agent_version_policy, ae.agent_version_id,
                ae.owner_principal_id, ae.resolved_owner_user_id, ae.channel_type,
                ae.channel_config, ae.channel_config_encrypted, ae.auth, ae.auth_encrypted,
                ae.enabled, ae.status AS endpoint_status, ae.created_at, ae.updated_at
            FROM agent_endpoints AS ae
            JOIN agents AS agent ON agent.id = ae.agent_id
            WHERE agent.org_id = $1 AND ae.agent_id = $2 AND ae.public_id = $3
            "#,
        )
        .bind(org_id)
        .bind(agent_id)
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn create_agent_endpoint(
        &self,
        org_id: i64,
        input: CreateAgentEndpointRow,
    ) -> Result<IngressEndpointRow> {
        let inserted = sqlx::query_scalar::<_, Uuid>(
            r#"
            INSERT INTO agent_endpoints (
                agent_id, app_id, legacy_app_public_id, public_id, channel_type,
                channel_config, channel_config_encrypted, auth, auth_encrypted,
                enabled, status, agent_identity_id, agent_version_policy,
                agent_version_id, owner_principal_id, resolved_owner_user_id
            )
            SELECT $2, NULL, NULL, $3, $4, $5, $6, $7, $8, $9, $10,
                $11, $12, $13, $14, $15
            FROM agents
            WHERE org_id = $1 AND id = $2 AND status = 'active'
            RETURNING id
            "#,
        )
        .bind(org_id)
        .bind(input.agent_id)
        .bind(&input.public_id)
        .bind(&input.channel_type)
        .bind(&input.channel_config)
        .bind(&input.channel_config_encrypted)
        .bind(&input.auth)
        .bind(&input.auth_encrypted)
        .bind(input.enabled)
        .bind(&input.status)
        .bind(input.agent_identity_id)
        .bind(&input.agent_version_policy)
        .bind(input.agent_version_id)
        .bind(input.owner_principal_id)
        .bind(input.resolved_owner_user_id)
        .fetch_optional(&self.pool)
        .await?;
        if inserted.is_none() {
            return Err(BadRequestError::new("Agent was not found or is not active").into());
        }
        self.get_agent_endpoint(org_id, input.agent_id, &input.public_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("created Agent endpoint could not be reloaded"))
    }

    pub async fn update_agent_endpoint(
        &self,
        org_id: i64,
        agent_id: Uuid,
        public_id: &str,
        input: UpdateAgentEndpointRow,
    ) -> Result<Option<IngressEndpointRow>> {
        sqlx::query(
            r#"
            UPDATE agent_endpoints AS ae
            SET channel_type = COALESCE($4, ae.channel_type),
                channel_config = COALESCE($5, ae.channel_config),
                channel_config_encrypted = CASE WHEN $6 THEN $7 ELSE ae.channel_config_encrypted END,
                auth = CASE WHEN $8 THEN $9 ELSE ae.auth END,
                auth_encrypted = CASE WHEN $10 THEN $11 ELSE ae.auth_encrypted END,
                enabled = COALESCE($12, ae.enabled),
                status = COALESCE($13, ae.status),
                updated_at = NOW()
            FROM agents AS agent
            WHERE agent.org_id = $1 AND agent.id = $2
              AND ae.agent_id = agent.id AND ae.public_id = $3
            "#,
        )
        .bind(org_id)
        .bind(agent_id)
        .bind(public_id)
        .bind(&input.channel_type)
        .bind(&input.channel_config)
        .bind(input.channel_config_encrypted.is_changed())
        .bind(input.channel_config_encrypted.into_value())
        .bind(input.auth.is_changed())
        .bind(input.auth.into_value())
        .bind(input.auth_encrypted.is_changed())
        .bind(input.auth_encrypted.into_value())
        .bind(input.enabled)
        .bind(&input.status)
        .execute(&self.pool)
        .await?;
        self.get_agent_endpoint(org_id, agent_id, public_id).await
    }

    pub async fn delete_agent_endpoint(
        &self,
        org_id: i64,
        agent_id: Uuid,
        public_id: &str,
    ) -> Result<bool> {
        let archived = sqlx::query(
            r#"
            UPDATE agent_endpoints AS ae
            SET enabled = false, status = 'disabled', updated_at = NOW()
            FROM agents AS agent
            WHERE agent.org_id = $1 AND agent.id = $2
              AND ae.agent_id = agent.id AND ae.public_id = $3 AND ae.app_id IS NOT NULL
            "#,
        )
        .bind(org_id)
        .bind(agent_id)
        .bind(public_id)
        .execute(&self.pool)
        .await?;
        if archived.rows_affected() > 0 {
            return Ok(true);
        }
        let deleted = sqlx::query(
            r#"
            DELETE FROM agent_endpoints AS ae
            USING agents AS agent
            WHERE agent.org_id = $1 AND agent.id = $2
              AND ae.agent_id = agent.id AND ae.public_id = $3 AND ae.app_id IS NULL
            "#,
        )
        .bind(org_id)
        .bind(agent_id)
        .bind(public_id)
        .execute(&self.pool)
        .await?;
        Ok(deleted.rows_affected() > 0)
    }

    pub async fn list_ingress_endpoints_by_legacy_alias(
        &self,
        legacy_app_public_id: &str,
        channel_type: &str,
    ) -> Result<Vec<IngressEndpointRow>> {
        sqlx::query_as::<_, IngressEndpointRow>(
            r#"
            SELECT
                ae.id AS endpoint_id,
                ae.public_id AS endpoint_public_id,
                ae.app_id AS legacy_app_id,
                ae.legacy_app_public_id,
                agent.org_id,
                ae.agent_id,
                agent.public_id AS agent_public_id,
                COALESCE(agent.display_name, agent.name) AS agent_name,
                agent.description AS agent_description,
                agent.harness_id,
                agent.status AS agent_status,
                agent.exposures_suspended,
                ae.agent_identity_id,
                ae.agent_version_policy,
                ae.agent_version_id,
                ae.owner_principal_id,
                ae.resolved_owner_user_id,
                ae.channel_type,
                ae.channel_config,
                ae.channel_config_encrypted,
                ae.auth,
                ae.auth_encrypted,
                ae.enabled,
                ae.status AS endpoint_status,
                ae.created_at,
                ae.updated_at
            FROM agent_endpoints AS ae
            JOIN agents AS agent ON agent.id = ae.agent_id
            WHERE ae.legacy_app_public_id = $1
              AND ae.channel_type = $2
              AND ae.enabled = true
            ORDER BY ae.created_at, ae.id
            "#,
        )
        .bind(legacy_app_public_id)
        .bind(channel_type)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
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
                channel_config_encrypted = CASE WHEN $4 THEN $5 ELSE ae.channel_config_encrypted END,
                auth = CASE WHEN $6 THEN $7 ELSE ae.auth END,
                auth_encrypted = CASE WHEN $8 THEN $9 ELSE ae.auth_encrypted END,
                durable_schedule_id = CASE WHEN $10 THEN $11 ELSE ae.durable_schedule_id END,
                enabled = COALESCE($12, ae.enabled),
                status = COALESCE($13, CASE
                    WHEN $12 = false THEN 'disabled'
                    ELSE ae.status
                END),
                updated_at = NOW()
            WHERE ae.id = $1
            RETURNING ae.id, ae.app_id, ae.public_id, ae.channel_type, ae.channel_config, ae.channel_config_encrypted, ae.auth, ae.auth_encrypted, ae.durable_schedule_id, ae.enabled, ae.status, ae.created_at, ae.updated_at
            "#,
        )
        .bind(id)
        .bind(&input.channel_type)
        .bind(&input.channel_config)
        .bind(input.channel_config_encrypted.is_changed())
        .bind(input.channel_config_encrypted.into_value())
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
                channel_config_encrypted = CASE WHEN $4 THEN $5 ELSE ae.channel_config_encrypted END,
                auth = CASE WHEN $6 THEN $7 ELSE ae.auth END,
                auth_encrypted = CASE WHEN $8 THEN $9 ELSE ae.auth_encrypted END,
                durable_schedule_id = CASE WHEN $10 THEN $11 ELSE ae.durable_schedule_id END,
                enabled = COALESCE($12, ae.enabled),
                status = COALESCE($13, CASE
                    WHEN $12 = false THEN 'disabled'
                    ELSE ae.status
                END),
                updated_at = NOW()
            WHERE ae.id = $1
            RETURNING ae.id, ae.app_id, ae.public_id, ae.channel_type, ae.channel_config, ae.channel_config_encrypted, ae.auth, ae.auth_encrypted, ae.durable_schedule_id, ae.enabled, ae.status, ae.created_at, ae.updated_at
            "#,
        )
        .bind(id)
        .bind(&input.channel_type)
        .bind(&input.channel_config)
        .bind(input.channel_config_encrypted.is_changed())
        .bind(input.channel_config_encrypted.into_value())
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

    /// Bridge archived App publish behavior onto endpoint status.
    ///
    /// Publishing only raises endpoints the operator enabled, and unpublishing
    /// lowers only live endpoints. An explicitly disabled endpoint therefore
    /// stays disabled across a compatibility publish cycle.
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
