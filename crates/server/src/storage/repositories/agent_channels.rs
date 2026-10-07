// PostgreSQL repository: agent channel CRUD
//
// Rows live in `agent_channels`, which is owned by an Agent rather than an App
// while `app_channels` remains as a read-only compatibility view. Every
// internal write goes directly to `agent_channels`, because the view cannot
// supply the NOT NULL columns the endpoint carries (`agent_id`,
// `owner_principal_id`).
//
// The lifted identity and owner are derived from the owning
// App on insert, which is exactly where those values came from before the
// re-parenting. A new endpoint's status is independent of App publish state.

use super::super::models::*;
use super::super::{CreateAgentChannelRow, IngressChannelRow, UpdateAgentChannelRow};
use super::Database;
use crate::errors::BadRequestError;
use anyhow::Result;
use everruns_server_macros::sql;
use uuid::Uuid;

fn missing_agent_error(app_id: Uuid) -> anyhow::Error {
    BadRequestError::new(format!(
        "App {app_id} was not found or has no agent; an endpoint must be owned by an agent"
    ))
    .into()
}

/// Insert an endpoint, deriving its agent, identity and owner
/// from the owning App. Selecting from `apps` rather than binding the values
/// keeps the derivation atomic with the insert. Yields no row when the App is
/// missing or still agent-less, which callers turn into `missing_agent_error`.
const INSERT_CHANNEL_SQL: &str = r#"
    INSERT INTO agent_channels (
        app_id, legacy_alias_id, agent_id, public_id, channel_type, channel_config,
        channel_config_encrypted, auth, auth_encrypted, durable_schedule_id, enabled,
        status, virtual_user_id, owner_principal_id, resolved_owner_user_id
    )
    SELECT
        app.id, app.public_id, app.agent_id, $2, $3, $4, $5, $6, $7, $8, $9,
        CASE WHEN $9 THEN 'draft' ELSE 'disabled' END,
        app.virtual_user_id, app.owner_principal_id, app.resolved_owner_user_id
    FROM apps AS app
    WHERE app.id = $1 AND app.agent_id IS NOT NULL
    RETURNING id, app_id, public_id, channel_type, channel_config, channel_config_encrypted, auth, auth_encrypted, durable_schedule_id, enabled, status, created_at, updated_at
"#;

impl Database {
    pub async fn list_agent_channel_summaries(
        &self,
        org_id: i64,
        agent_ids: &[Uuid],
    ) -> Result<Vec<super::super::AgentChannelSummaryRow>> {
        // Project only non-secret fields and fetch the current page in one query.
        sqlx::query_as(
            r#"
            SELECT ae.agent_id, ae.public_id, ae.channel_type, ae.enabled, ae.status
            FROM agent_channels ae
            JOIN agents agent ON agent.id = ae.agent_id
            WHERE agent.org_id = $1 AND ae.agent_id = ANY($2)
                AND ae.channel_type <> 'schedule'
            ORDER BY ae.created_at, ae.id
            "#,
        )
        .bind(org_id)
        .bind(agent_ids)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    // ============================================
    // Agent channel CRUD
    // ============================================

    /// Hold a transaction-scoped, cross-instance lock for one Slack endpoint.
    pub async fn lock_slack_install(
        &self,
        channel_id: Uuid,
    ) -> Result<sqlx::Transaction<'static, sqlx::Postgres>> {
        // Holders re-read and persist through the shared pool and call Slack
        // under this lock, so waiters must not pin connections (TM-DOS-042).
        self.advisory_xact_lock_polling(
            "slack_install",
            &channel_id.to_string(),
            super::advisory_locks::ADVISORY_LOCK_WAIT,
        )
        .await
    }

    pub async fn create_legacy_alias_channel(
        &self,
        app_id: Uuid,
        input: CreateLegacyAliasChannelRow,
    ) -> Result<AgentChannelRow> {
        let row = sqlx::query_as::<_, AgentChannelRow>(INSERT_CHANNEL_SQL)
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

    pub async fn get_ingress_channel_by_public_id(
        &self,
        public_id: &str,
    ) -> Result<Option<IngressChannelRow>> {
        sqlx::query_as::<_, IngressChannelRow>(
            r#"
            SELECT
                ae.id AS channel_id,
                ae.public_id AS channel_public_id,
                ae.app_id AS legacy_app_id,
                ae.legacy_alias_id,
                agent.org_id,
                ae.agent_id,
                agent.public_id AS agent_public_id,
                COALESCE(NULLIF(agent.display_name, ''), agent.name) AS agent_name,
                agent.description AS agent_description,
                agent.harness_id,
                agent.status AS agent_status,
                agent.exposures_suspended,
                ae.virtual_user_id,
                ae.owner_principal_id,
                ae.resolved_owner_user_id,
                ae.channel_type,
                ae.channel_config,
                ae.channel_config_encrypted,
                ae.auth,
                ae.auth_encrypted,
                ae.enabled,
                ae.status AS channel_status,
                ae.created_at,
                ae.updated_at
            FROM agent_channels AS ae
            JOIN agents AS agent ON agent.id = ae.agent_id
            WHERE ae.public_id = $1
            "#,
        )
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn list_agent_channels(
        &self,
        org_id: i64,
        agent_id: Uuid,
    ) -> Result<Vec<IngressChannelRow>> {
        sqlx::query_as::<_, IngressChannelRow>(
            r#"
            SELECT ae.id AS channel_id, ae.public_id AS channel_public_id,
                ae.app_id AS legacy_app_id, ae.legacy_alias_id, agent.org_id,
                ae.agent_id, agent.public_id AS agent_public_id,
                COALESCE(NULLIF(agent.display_name, ''), agent.name) AS agent_name,
                agent.description AS agent_description, agent.harness_id,
                agent.status AS agent_status, agent.exposures_suspended,
                ae.virtual_user_id,
                ae.owner_principal_id, ae.resolved_owner_user_id, ae.channel_type,
                ae.channel_config, ae.channel_config_encrypted, ae.auth, ae.auth_encrypted,
                ae.enabled, ae.status AS channel_status, ae.created_at, ae.updated_at
            FROM agent_channels AS ae
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

    pub async fn get_agent_channel(
        &self,
        org_id: i64,
        agent_id: Uuid,
        public_id: &str,
    ) -> Result<Option<IngressChannelRow>> {
        sqlx::query_as::<_, IngressChannelRow>(
            r#"
            SELECT ae.id AS channel_id, ae.public_id AS channel_public_id,
                ae.app_id AS legacy_app_id, ae.legacy_alias_id, agent.org_id,
                ae.agent_id, agent.public_id AS agent_public_id,
                COALESCE(NULLIF(agent.display_name, ''), agent.name) AS agent_name,
                agent.description AS agent_description, agent.harness_id,
                agent.status AS agent_status, agent.exposures_suspended,
                ae.virtual_user_id,
                ae.owner_principal_id, ae.resolved_owner_user_id, ae.channel_type,
                ae.channel_config, ae.channel_config_encrypted, ae.auth, ae.auth_encrypted,
                ae.enabled, ae.status AS channel_status, ae.created_at, ae.updated_at
            FROM agent_channels AS ae
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

    pub async fn create_agent_channel(
        &self,
        org_id: i64,
        input: CreateAgentChannelRow,
    ) -> Result<IngressChannelRow> {
        let inserted = sqlx::query_scalar::<_, Uuid>(
            r#"
            INSERT INTO agent_channels (
                agent_id, app_id, legacy_alias_id, public_id, channel_type,
                channel_config, channel_config_encrypted, auth, auth_encrypted,
                enabled, status, virtual_user_id, owner_principal_id,
                resolved_owner_user_id
            )
            SELECT $2, NULL, NULL, $3, $4, $5, $6, $7, $8, $9, $10,
                $11, $12, $13
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
        .bind(input.virtual_user_id)
        .bind(input.owner_principal_id)
        .bind(input.resolved_owner_user_id)
        .fetch_optional(&self.pool)
        .await?;
        if inserted.is_none() {
            return Err(BadRequestError::new("Agent was not found or is not active").into());
        }
        self.get_agent_channel(org_id, input.agent_id, &input.public_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("created Agent channel could not be reloaded"))
    }

    pub async fn update_agent_channel(
        &self,
        org_id: i64,
        agent_id: Uuid,
        public_id: &str,
        input: UpdateAgentChannelRow,
    ) -> Result<Option<IngressChannelRow>> {
        sqlx::query(
            r#"
            UPDATE agent_channels AS ae
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
        self.get_agent_channel(org_id, agent_id, public_id).await
    }

    pub async fn delete_agent_channel(
        &self,
        org_id: i64,
        agent_id: Uuid,
        public_id: &str,
    ) -> Result<bool> {
        let archived = sqlx::query(
            r#"
            UPDATE agent_channels AS ae
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
            DELETE FROM agent_channels AS ae
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

    /// Slack deletion is irreversible; keep its progress even if the enclosing
    /// Agent mutation rolls back after another app fails. The lifecycle lock
    /// prevents installs from replacing these credentials during cleanup.
    pub async fn record_slack_app_removed(
        &self,
        org_id: i64,
        agent_id: Uuid,
        public_id: &str,
        config: serde_json::Value,
        encrypted: Option<Vec<u8>>,
    ) -> Result<bool> {
        let mut tx = self.pool.begin_detached().await?;
        let result = sqlx::query(
            "UPDATE agent_channels AS channel SET channel_config = $4, \
             channel_config_encrypted = $5, enabled = false, status = 'disabled', updated_at = NOW() \
             FROM agents AS agent WHERE agent.org_id = $1 AND agent.id = $2 \
             AND channel.agent_id = agent.id AND channel.public_id = $3 AND channel.channel_type = 'slack'",
        )
        .bind(org_id).bind(agent_id).bind(public_id).bind(config).bind(encrypted)
        .execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn list_ingress_channels_by_legacy_alias(
        &self,
        legacy_alias_id: &str,
        channel_type: &str,
    ) -> Result<Vec<IngressChannelRow>> {
        sqlx::query_as::<_, IngressChannelRow>(
            r#"
            SELECT
                ae.id AS channel_id,
                ae.public_id AS channel_public_id,
                ae.app_id AS legacy_app_id,
                ae.legacy_alias_id,
                agent.org_id,
                ae.agent_id,
                agent.public_id AS agent_public_id,
                COALESCE(NULLIF(agent.display_name, ''), agent.name) AS agent_name,
                agent.description AS agent_description,
                agent.harness_id,
                agent.status AS agent_status,
                agent.exposures_suspended,
                ae.virtual_user_id,
                ae.owner_principal_id,
                ae.resolved_owner_user_id,
                ae.channel_type,
                ae.channel_config,
                ae.channel_config_encrypted,
                ae.auth,
                ae.auth_encrypted,
                ae.enabled,
                ae.status AS channel_status,
                ae.created_at,
                ae.updated_at
            FROM agent_channels AS ae
            JOIN agents AS agent ON agent.id = ae.agent_id
            WHERE ae.legacy_alias_id = $1
              AND ae.channel_type = $2
              AND ae.enabled = true
            ORDER BY ae.created_at, ae.id
            "#,
        )
        .bind(legacy_alias_id)
        .bind(channel_type)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn list_legacy_alias_channels(&self, app_id: Uuid) -> Result<Vec<AgentChannelRow>> {
        let rows = sqlx::query_as::<_, AgentChannelRow>(sql!(
            r#"
            SELECT {AgentChannelRow}
            FROM agent_channels
            WHERE app_id = $1
            ORDER BY created_at ASC
            "#
        ))
        .bind(app_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    // THREAT[TM-TENANT-012]: `get_channel_row_by_public_id` /
    // `update_channel_by_id` / `delete_channel_by_id` take a bare global
    // identifier (`public_id` / row `id`) with no `app_id`/`org_id` filter, and
    // the row carries `channel_config_encrypted` (secrets). They do NOT enforce
    // tenant isolation on their own. Every caller MUST already hold the endpoint
    // through a tenant-resolving path — e.g. `update_channel_config_unscoped`
    // only receives ids that `api::channel_ingress::resolve_channel` returned.
    // Do not add a caller that takes the id straight from a request.
    pub async fn get_channel_row_by_public_id(
        &self,
        public_id: &str,
    ) -> Result<Option<AgentChannelRow>> {
        let row = sqlx::query_as::<_, AgentChannelRow>(sql!(
            r#"
            SELECT {AgentChannelRow}
            FROM agent_channels
            WHERE public_id = $1
            "#
        ))
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_agent_channel_public_id(
        &self,
        org_id: i64,
        channel_id: Uuid,
    ) -> Result<Option<String>> {
        let public_id = sqlx::query_scalar::<_, String>(
            r#"
            SELECT ae.public_id
            FROM agent_channels AS ae
            JOIN agents AS a ON a.id = ae.agent_id
            WHERE a.org_id = $1 AND ae.id = $2
            "#,
        )
        .bind(org_id)
        .bind(channel_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(public_id)
    }

    /// Persist ingress configuration without decoding the archival App-linked
    /// row: native endpoints deliberately have a NULL app_id.
    /// THREAT[TM-TENANT-012]: caller must first resolve the endpoint and its tenant.
    pub async fn update_channel_config_by_id(
        &self,
        id: Uuid,
        config: serde_json::Value,
        encrypted: Option<Vec<u8>>,
    ) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE agent_channels SET channel_config = $2, channel_config_encrypted = $3, updated_at = NOW() WHERE id = $1",
        )
        .bind(id)
        .bind(config)
        .bind(encrypted)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    // THREAT[TM-TENANT-012]: bare-`id` mutator — see the note on
    // `get_channel_row_by_public_id`. Callers MUST have already resolved the
    // endpoint through a tenant-resolving path.
    pub async fn update_channel_by_id(
        &self,
        id: Uuid,
        input: UpdateChannelByIdRow,
    ) -> Result<Option<AgentChannelRow>> {
        let row = sqlx::query_as::<_, AgentChannelRow>(
            r#"
            UPDATE agent_channels AS ae
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

    // THREAT[TM-TENANT-012]: bare-`id` mutator — see the note on
    // `get_channel_row_by_public_id`. Callers MUST have already resolved the
    // endpoint through a tenant-resolving path.
    pub async fn delete_channel_by_id(&self, id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM agent_channels WHERE id = $1")
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
    pub async fn agents_with_live_channels(
        &self,
        agent_ids: &[Uuid],
    ) -> Result<std::collections::HashSet<Uuid>> {
        if agent_ids.is_empty() {
            return Ok(std::collections::HashSet::new());
        }
        let rows = sqlx::query_scalar::<_, Uuid>(
            r#"
            SELECT DISTINCT agent_id
            FROM agent_channels
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
    pub async fn set_app_channel_publish(&self, app_id: Uuid, published: bool) -> Result<u64> {
        let sql = if published {
            "UPDATE agent_channels SET status = 'live', updated_at = NOW()
             WHERE app_id = $1 AND enabled = true AND status <> 'live'"
        } else {
            "UPDATE agent_channels SET status = 'draft', updated_at = NOW()
             WHERE app_id = $1 AND status = 'live'"
        };
        let result = sqlx::query(sql).bind(app_id).execute(&self.pool).await?;
        Ok(result.rows_affected())
    }
}
