pub(super) mod rows;
use super::Database;
use anyhow::Result;
use everruns_contracts::typed_id::AgentId;
use everruns_server_macros::sql;
use rows::*;
use uuid::Uuid;

impl Database {
    pub async fn upsert_agent_mcp_secret_binding(
        &self,
        input: UpsertAgentMcpSecretBindingRow,
    ) -> Result<AgentMcpSecretBindingRow> {
        sqlx::query_as::<_, AgentMcpSecretBindingRow>(sql!(
            r#"
            INSERT INTO agent_mcp_secret_bindings
                (org_id, agent_id, mcp_server_name, mcp_server_url, tool_name,
                 parameter_name, label, description)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (agent_id, mcp_server_name, tool_name, parameter_name)
            DO UPDATE SET
                mcp_server_url = EXCLUDED.mcp_server_url,
                label = EXCLUDED.label,
                description = EXCLUDED.description,
                value_encrypted = CASE
                    WHEN agent_mcp_secret_bindings.mcp_server_url = EXCLUDED.mcp_server_url
                    THEN agent_mcp_secret_bindings.value_encrypted
                    ELSE NULL
                END,
                updated_at = NOW()
            RETURNING {AgentMcpSecretBindingRow}
            "#
        ))
        .bind(input.org_id)
        .bind(input.agent_id)
        .bind(input.mcp_server_name)
        .bind(input.mcp_server_url)
        .bind(input.tool_name)
        .bind(input.parameter_name)
        .bind(input.label)
        .bind(input.description)
        .fetch_one(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// Declare an HTTP credential. Re-declaring keeps the stored value only
    /// when the binding (URL and auth form) is unchanged, so moving a key to
    /// another host always needs the person to paste it again.
    pub async fn upsert_agent_http_credential(
        &self,
        input: UpsertAgentHttpCredentialRow,
    ) -> Result<AgentMcpSecretBindingRow> {
        sqlx::query_as::<_, AgentMcpSecretBindingRow>(sql!(
            r#"
            INSERT INTO agent_mcp_secret_bindings
                (org_id, agent_id, kind, env_name, binding, label, description)
            VALUES ($1, $2, 'http', $3, $4, $5, $6)
            ON CONFLICT (agent_id, env_name) WHERE kind = 'http'
            DO UPDATE SET
                binding = EXCLUDED.binding,
                label = EXCLUDED.label,
                description = EXCLUDED.description,
                value_encrypted = CASE
                    WHEN agent_mcp_secret_bindings.binding = EXCLUDED.binding
                    THEN agent_mcp_secret_bindings.value_encrypted
                    ELSE NULL
                END,
                updated_at = NOW()
            RETURNING {AgentMcpSecretBindingRow}
            "#
        ))
        .bind(input.org_id)
        .bind(input.agent_id)
        .bind(input.env_name)
        .bind(sqlx::types::Json(input.binding))
        .bind(input.label)
        .bind(input.description)
        .fetch_one(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn list_agent_mcp_secret_bindings(
        &self,
        org_id: i64,
        agent_id: AgentId,
    ) -> Result<Vec<AgentMcpSecretBindingRow>> {
        sqlx::query_as::<_, AgentMcpSecretBindingRow>(sql!(
            r#"
            SELECT {AgentMcpSecretBindingRow}
            FROM agent_mcp_secret_bindings
            WHERE org_id = $1 AND agent_id = $2
            ORDER BY kind, mcp_server_name, tool_name, parameter_name, env_name
            "#
        ))
        .bind(org_id)
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn get_agent_mcp_secret_binding(
        &self,
        org_id: i64,
        agent_id: AgentId,
        binding_id: Uuid,
    ) -> Result<Option<AgentMcpSecretBindingRow>> {
        sqlx::query_as::<_, AgentMcpSecretBindingRow>(sql!(
            r#"
            SELECT {AgentMcpSecretBindingRow}
            FROM agent_mcp_secret_bindings
            WHERE org_id = $1 AND agent_id = $2 AND id = $3
            "#
        ))
        .bind(org_id)
        .bind(agent_id)
        .bind(binding_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn set_agent_mcp_secret_binding_value(
        &self,
        org_id: i64,
        agent_id: AgentId,
        binding_id: Uuid,
        value_encrypted: Vec<u8>,
    ) -> Result<Option<AgentMcpSecretBindingRow>> {
        sqlx::query_as::<_, AgentMcpSecretBindingRow>(sql!(
            r#"
            UPDATE agent_mcp_secret_bindings
            SET value_encrypted = $4, updated_at = NOW()
            WHERE org_id = $1 AND agent_id = $2 AND id = $3
            RETURNING {AgentMcpSecretBindingRow}
            "#
        ))
        .bind(org_id)
        .bind(agent_id)
        .bind(binding_id)
        .bind(value_encrypted)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn delete_agent_mcp_secret_binding(
        &self,
        org_id: i64,
        agent_id: AgentId,
        binding_id: Uuid,
    ) -> Result<bool> {
        let result = sqlx::query(
            "DELETE FROM agent_mcp_secret_bindings WHERE org_id = $1 AND agent_id = $2 AND id = $3",
        )
        .bind(org_id)
        .bind(agent_id)
        .bind(binding_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }
}
