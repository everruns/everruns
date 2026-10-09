// PostgreSQL repository: Agent Script CRUD

use super::super::models::*;
use super::Database;
use crate::kernel_imports::{contracts::typed_id::AgentId, contracts::typed_id::ScriptId};
use anyhow::Result;
use everruns_server_macros::sql;

impl Database {
    pub async fn create_agent_script(&self, input: CreateAgentScriptRow) -> Result<AgentScriptRow> {
        let row = sqlx::query_as::<_, AgentScriptRow>(sql!(
            r#"
            INSERT INTO agent_scripts (org_id, id, agent_id, name, description, input_schema, body, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, 'active')
            RETURNING {AgentScriptRow}
            "#
        ))
        .bind(input.org_id)
        .bind(input.id)
        .bind(input.agent_id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.input_schema)
        .bind(&input.body)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_agent_script(
        &self,
        org_id: i64,
        id: ScriptId,
    ) -> Result<Option<AgentScriptRow>> {
        let row = sqlx::query_as::<_, AgentScriptRow>(sql!(
            r#"
            SELECT {AgentScriptRow} FROM agent_scripts
            WHERE org_id = $1 AND id = $2
            "#
        ))
        .bind(org_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Scripts of one agent, oldest first so the order is stable.
    pub async fn list_agent_scripts(
        &self,
        org_id: i64,
        agent_id: AgentId,
        include_archived: bool,
    ) -> Result<Vec<AgentScriptRow>> {
        let rows = sqlx::query_as::<_, AgentScriptRow>(sql!(
            r#"
            SELECT {AgentScriptRow} FROM agent_scripts
            WHERE org_id = $1 AND agent_id = $2
              AND CASE WHEN $3 THEN status != 'deleted' ELSE status = 'active' END
            ORDER BY created_at, id
            "#
        ))
        .bind(org_id)
        .bind(agent_id)
        .bind(include_archived)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn update_agent_script(
        &self,
        org_id: i64,
        id: ScriptId,
        input: UpdateAgentScript,
    ) -> Result<Option<AgentScriptRow>> {
        let row = sqlx::query_as::<_, AgentScriptRow>(sql!(
            r#"
            UPDATE agent_scripts
            SET
                description = COALESCE($3, description),
                input_schema = COALESCE($4, input_schema),
                body = COALESCE($5, body),
                updated_at = NOW()
            WHERE org_id = $1 AND id = $2 AND status = 'active'
            RETURNING {AgentScriptRow}
            "#
        ))
        .bind(org_id)
        .bind(id)
        .bind(&input.description)
        .bind(&input.input_schema)
        .bind(&input.body)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Archive (soft delete). Frees the name for a new active script.
    pub async fn delete_agent_script(&self, org_id: i64, id: ScriptId) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE agent_scripts
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
