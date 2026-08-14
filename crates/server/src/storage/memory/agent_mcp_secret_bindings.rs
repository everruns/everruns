use super::InMemoryDatabase;
use crate::storage::models::{AgentMcpSecretBindingRow, UpsertAgentMcpSecretBindingRow};
use anyhow::Result;
use everruns_provider::typed_id::AgentId;
use uuid::Uuid;

impl InMemoryDatabase {
    pub async fn upsert_agent_mcp_secret_binding(
        &self,
        input: UpsertAgentMcpSecretBindingRow,
    ) -> Result<AgentMcpSecretBindingRow> {
        let mut rows = self.agent_mcp_secret_bindings.write();
        let now = Self::now();
        if let Some(row) = rows.values_mut().find(|row| {
            row.agent_id == input.agent_id
                && row.mcp_server_name == input.mcp_server_name
                && row.tool_name == input.tool_name
                && row.parameter_name == input.parameter_name
        }) {
            if row.mcp_server_url != input.mcp_server_url {
                row.value_encrypted = None;
            }
            row.mcp_server_url = input.mcp_server_url;
            row.label = input.label;
            row.description = input.description;
            row.updated_at = now;
            return Ok(row.clone());
        }
        let row = AgentMcpSecretBindingRow {
            id: Uuid::now_v7(),
            org_id: input.org_id,
            agent_id: input.agent_id,
            mcp_server_name: input.mcp_server_name,
            mcp_server_url: input.mcp_server_url,
            tool_name: input.tool_name,
            parameter_name: input.parameter_name,
            label: input.label,
            description: input.description,
            value_encrypted: None,
            created_at: now,
            updated_at: now,
        };
        rows.insert(row.id, row.clone());
        Ok(row)
    }

    pub async fn list_agent_mcp_secret_bindings(
        &self,
        org_id: i64,
        agent_id: AgentId,
    ) -> Result<Vec<AgentMcpSecretBindingRow>> {
        let mut rows = self
            .agent_mcp_secret_bindings
            .read()
            .values()
            .filter(|row| row.org_id == org_id && row.agent_id == agent_id)
            .cloned()
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| {
            (&a.mcp_server_name, &a.tool_name, &a.parameter_name).cmp(&(
                &b.mcp_server_name,
                &b.tool_name,
                &b.parameter_name,
            ))
        });
        Ok(rows)
    }

    pub async fn get_agent_mcp_secret_binding(
        &self,
        org_id: i64,
        agent_id: AgentId,
        binding_id: Uuid,
    ) -> Result<Option<AgentMcpSecretBindingRow>> {
        Ok(self
            .agent_mcp_secret_bindings
            .read()
            .get(&binding_id)
            .filter(|row| row.org_id == org_id && row.agent_id == agent_id)
            .cloned())
    }

    pub async fn set_agent_mcp_secret_binding_value(
        &self,
        org_id: i64,
        agent_id: AgentId,
        binding_id: Uuid,
        value_encrypted: Vec<u8>,
    ) -> Result<Option<AgentMcpSecretBindingRow>> {
        let mut rows = self.agent_mcp_secret_bindings.write();
        let Some(row) = rows
            .get_mut(&binding_id)
            .filter(|row| row.org_id == org_id && row.agent_id == agent_id)
        else {
            return Ok(None);
        };
        row.value_encrypted = Some(value_encrypted);
        row.updated_at = Self::now();
        Ok(Some(row.clone()))
    }

    pub async fn delete_agent_mcp_secret_binding(
        &self,
        org_id: i64,
        agent_id: AgentId,
        binding_id: Uuid,
    ) -> Result<bool> {
        let mut rows = self.agent_mcp_secret_bindings.write();
        let owned = rows
            .get(&binding_id)
            .is_some_and(|row| row.org_id == org_id && row.agent_id == agent_id);
        Ok(owned && rows.remove(&binding_id).is_some())
    }
}
