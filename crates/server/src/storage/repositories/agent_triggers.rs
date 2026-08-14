// PostgreSQL repository: Agent Trigger CRUD

use super::super::models::*;
use super::Database;
use crate::kernel_imports::{
    everruns_provider::typed_id::AgentId, everruns_provider::typed_id::TriggerId,
};
use anyhow::Result;
use uuid::Uuid;

const COLUMNS: &str = "id, org_id, agent_id, trigger_type, config, enabled, durable_schedule_id, execution_harness_id, execution_owner_principal_id, execution_resolved_owner_user_id, execution_agent_identity_id, execution_app_id, status, created_at, updated_at, archived_at, deleted_at";

impl Database {
    // ============================================
    // Agent Trigger CRUD
    // ============================================

    pub async fn create_agent_trigger(
        &self,
        input: CreateAgentTriggerRow,
    ) -> Result<AgentTriggerRow> {
        let row = sqlx::query_as::<_, AgentTriggerRow>(
            r#"
            INSERT INTO agent_triggers (org_id, id, agent_id, trigger_type, config, enabled, durable_schedule_id, execution_harness_id, execution_owner_principal_id, execution_resolved_owner_user_id, execution_agent_identity_id, execution_app_id, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, 'active')
            RETURNING id, org_id, agent_id, trigger_type, config, enabled, durable_schedule_id, execution_harness_id, execution_owner_principal_id, execution_resolved_owner_user_id, execution_agent_identity_id, execution_app_id, status, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(input.org_id)
        .bind(input.id)
        .bind(input.agent_id)
        .bind(&input.trigger_type)
        .bind(&input.config)
        .bind(input.enabled)
        .bind(input.durable_schedule_id)
        .bind(input.execution_harness_id)
        .bind(input.execution_owner_principal_id)
        .bind(input.execution_resolved_owner_user_id)
        .bind(input.execution_agent_identity_id)
        .bind(input.execution_app_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_agent_trigger(
        &self,
        org_id: i64,
        id: TriggerId,
    ) -> Result<Option<AgentTriggerRow>> {
        let sql = format!("SELECT {COLUMNS} FROM agent_triggers WHERE org_id = $1 AND id = $2");
        let row = sqlx::query_as::<_, AgentTriggerRow>(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(org_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;

        Ok(row)
    }

    pub async fn list_agent_triggers(
        &self,
        org_id: i64,
        agent_id: Option<AgentId>,
        include_archived: bool,
    ) -> Result<Vec<AgentTriggerRow>> {
        let status_sql = if include_archived {
            " AND status != 'deleted'"
        } else {
            " AND status NOT IN ('archived', 'deleted')"
        };
        let agent_sql = if agent_id.is_some() {
            " AND agent_id = $2"
        } else {
            ""
        };
        let sql = format!(
            "SELECT {COLUMNS} FROM agent_triggers WHERE org_id = $1{status_sql}{agent_sql} ORDER BY created_at DESC"
        );
        let mut query =
            sqlx::query_as::<_, AgentTriggerRow>(sqlx::AssertSqlSafe(sql.as_str())).bind(org_id);
        if let Some(agent_id) = agent_id {
            query = query.bind(agent_id);
        }
        Ok(query.fetch_all(&self.pool).await?)
    }

    pub async fn update_agent_trigger(
        &self,
        org_id: i64,
        id: TriggerId,
        input: UpdateAgentTrigger,
    ) -> Result<Option<AgentTriggerRow>> {
        let row = sqlx::query_as::<_, AgentTriggerRow>(
            r#"
            UPDATE agent_triggers
            SET
                trigger_type = COALESCE($3, trigger_type),
                config = COALESCE($4, config),
                enabled = COALESCE($5, enabled),
                durable_schedule_id = CASE WHEN $6 THEN $7 ELSE durable_schedule_id END,
                status = COALESCE($8, status),
                updated_at = NOW()
            WHERE org_id = $1 AND id = $2
            RETURNING id, org_id, agent_id, trigger_type, config, enabled, durable_schedule_id, execution_harness_id, execution_owner_principal_id, execution_resolved_owner_user_id, execution_agent_identity_id, execution_app_id, status, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(org_id)
        .bind(id)
        .bind(&input.trigger_type)
        .bind(&input.config)
        .bind(input.enabled)
        .bind(input.durable_schedule_id.is_changed())
        .bind(input.durable_schedule_id.into_value())
        .bind(&input.status)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Bind (or clear) the durable schedule backing a trigger. Mirrors the
    /// app_channel durable-schedule binding update.
    pub async fn set_agent_trigger_durable_schedule_id(
        &self,
        org_id: i64,
        id: TriggerId,
        durable_schedule_id: Option<Uuid>,
    ) -> Result<Option<AgentTriggerRow>> {
        let row = sqlx::query_as::<_, AgentTriggerRow>(
            r#"
            UPDATE agent_triggers
            SET durable_schedule_id = $3, updated_at = NOW()
            WHERE org_id = $1 AND id = $2
            RETURNING id, org_id, agent_id, trigger_type, config, enabled, durable_schedule_id, execution_harness_id, execution_owner_principal_id, execution_resolved_owner_user_id, execution_agent_identity_id, execution_app_id, status, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(org_id)
        .bind(id)
        .bind(durable_schedule_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn delete_agent_trigger(&self, org_id: i64, id: TriggerId) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE agent_triggers
            SET status = 'archived', archived_at = COALESCE(archived_at, NOW()), updated_at = NOW()
            WHERE org_id = $1 AND id = $2 AND status = 'active'
            "#,
        )
        .bind(org_id)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }
}
