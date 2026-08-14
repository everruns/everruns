// In-memory storage: Agent Trigger CRUD

use super::super::models::*;
use super::InMemoryDatabase;
use crate::kernel_imports::{
    everruns_provider::typed_id::AgentId, everruns_provider::typed_id::TriggerId,
};
use anyhow::Result;
use uuid::Uuid;

impl InMemoryDatabase {
    // ============================================
    // Agent Trigger CRUD
    // ============================================

    pub async fn create_agent_trigger(
        &self,
        input: CreateAgentTriggerRow,
    ) -> Result<AgentTriggerRow> {
        let row = AgentTriggerRow {
            id: input.id,
            org_id: input.org_id,
            agent_id: input.agent_id,
            trigger_type: input.trigger_type,
            config: input.config,
            enabled: input.enabled,
            durable_schedule_id: input.durable_schedule_id,
            execution_harness_id: input.execution_harness_id,
            execution_owner_principal_id: input.execution_owner_principal_id,
            execution_resolved_owner_user_id: input.execution_resolved_owner_user_id,
            execution_agent_identity_id: input.execution_agent_identity_id,
            execution_app_id: input.execution_app_id,
            status: "active".to_string(),
            created_at: Self::now(),
            updated_at: Self::now(),
            archived_at: None,
            deleted_at: None,
        };
        self.agent_triggers.write().insert(row.id, row.clone());
        Ok(row)
    }

    pub async fn get_agent_trigger(
        &self,
        org_id: i64,
        id: TriggerId,
    ) -> Result<Option<AgentTriggerRow>> {
        Ok(self
            .agent_triggers
            .read()
            .get(&id)
            .filter(|row| row.org_id == org_id)
            .cloned())
    }

    pub async fn list_agent_triggers(
        &self,
        org_id: i64,
        agent_id: Option<AgentId>,
        include_archived: bool,
    ) -> Result<Vec<AgentTriggerRow>> {
        let mut rows: Vec<_> = self
            .agent_triggers
            .read()
            .values()
            .filter(|row| row.org_id == org_id)
            .filter(|row| row.status != "deleted")
            .filter(|row| include_archived || row.status != "archived")
            .filter(|row| agent_id.is_none_or(|aid| row.agent_id == aid))
            .cloned()
            .collect();
        rows.sort_by_key(|row| std::cmp::Reverse(row.created_at));
        Ok(rows)
    }

    pub async fn update_agent_trigger(
        &self,
        org_id: i64,
        id: TriggerId,
        input: UpdateAgentTrigger,
    ) -> Result<Option<AgentTriggerRow>> {
        let mut rows = self.agent_triggers.write();
        let Some(row) = rows.get_mut(&id) else {
            return Ok(None);
        };
        if row.org_id != org_id {
            return Ok(None);
        }
        if let Some(trigger_type) = input.trigger_type {
            row.trigger_type = trigger_type;
        }
        if let Some(config) = input.config {
            row.config = config;
        }
        if let Some(enabled) = input.enabled {
            row.enabled = enabled;
        }
        input
            .durable_schedule_id
            .apply(&mut row.durable_schedule_id);
        if let Some(status) = input.status {
            row.status = status;
        }
        row.updated_at = Self::now();
        Ok(Some(row.clone()))
    }

    /// Bind (or clear) the durable schedule backing a trigger.
    pub async fn set_agent_trigger_durable_schedule_id(
        &self,
        org_id: i64,
        id: TriggerId,
        durable_schedule_id: Option<Uuid>,
    ) -> Result<Option<AgentTriggerRow>> {
        let mut rows = self.agent_triggers.write();
        let Some(row) = rows.get_mut(&id) else {
            return Ok(None);
        };
        if row.org_id != org_id {
            return Ok(None);
        }
        row.durable_schedule_id = durable_schedule_id;
        row.updated_at = Self::now();
        Ok(Some(row.clone()))
    }

    pub async fn delete_agent_trigger(&self, org_id: i64, id: TriggerId) -> Result<bool> {
        let mut rows = self.agent_triggers.write();
        let Some(row) = rows.get_mut(&id) else {
            return Ok(false);
        };
        if row.org_id != org_id || row.status != "active" {
            return Ok(false);
        }
        row.status = "archived".to_string();
        row.archived_at = Some(Self::now());
        row.updated_at = Self::now();
        Ok(true)
    }
}
