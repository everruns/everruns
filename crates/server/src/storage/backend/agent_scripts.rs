//! Agent scripts: saved shell scripts an agent owns.
//! See knowledge/runtime-resources/agent-scripts.md.

use super::*;

impl StorageBackend {
    pub async fn create_agent_script(&self, input: CreateAgentScriptRow) -> Result<AgentScriptRow> {
        dispatch!(self, create_agent_script, input)
    }

    pub async fn get_agent_script(
        &self,
        org_id: i64,
        id: ScriptId,
    ) -> Result<Option<AgentScriptRow>> {
        dispatch!(self, get_agent_script, org_id, id)
    }

    pub async fn list_agent_scripts(
        &self,
        org_id: i64,
        agent_id: AgentId,
        include_archived: bool,
    ) -> Result<Vec<AgentScriptRow>> {
        dispatch!(self, list_agent_scripts, org_id, agent_id, include_archived)
    }

    pub async fn update_agent_script(
        &self,
        org_id: i64,
        id: ScriptId,
        input: UpdateAgentScript,
    ) -> Result<Option<AgentScriptRow>> {
        dispatch!(self, update_agent_script, org_id, id, input)
    }

    pub async fn delete_agent_script(&self, org_id: i64, id: ScriptId) -> Result<bool> {
        dispatch!(self, delete_agent_script, org_id, id)
    }
}
