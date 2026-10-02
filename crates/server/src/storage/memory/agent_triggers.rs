// In-memory storage: Agent Trigger CRUD

use super::super::agent_trigger_deliveries::*;
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
            ingress_id: input.ingress_id,
            config: input.config,
            config_encrypted: input.config_encrypted,
            enabled: input.enabled,
            durable_schedule_id: input.durable_schedule_id,
            execution_harness_id: input.execution_harness_id,
            execution_owner_principal_id: input.execution_owner_principal_id,
            execution_resolved_owner_user_id: input.execution_resolved_owner_user_id,
            execution_virtual_user_id: input.execution_virtual_user_id,
            execution_app_id: input.execution_app_id,
            legacy_alias_id: input.legacy_alias_id,
            legacy_alias_name: input.legacy_alias_name,
            agent_version_policy: input.agent_version_policy,
            agent_version_id: input.agent_version_id,
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

    pub async fn get_agent_trigger_by_ingress_id_unscoped(
        &self,
        ingress_id: &str,
    ) -> Result<Option<AgentTriggerRow>> {
        Ok(self
            .agent_triggers
            .read()
            .values()
            .find(|row| row.ingress_id.as_deref() == Some(ingress_id) && row.status == "active")
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
        if let Some(config_encrypted) = input.config_encrypted {
            row.config_encrypted = Some(config_encrypted);
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
        if let Some(policy) = input.agent_version_policy {
            row.agent_version_policy = Some(policy);
        }
        input.agent_version_id.apply(&mut row.agent_version_id);
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

    // ============================================
    // Agent trigger deliveries
    // ============================================

    pub async fn record_agent_trigger_delivery(
        &self,
        input: CreateAgentTriggerDeliveryRow,
    ) -> Result<Option<AgentTriggerDeliveryRow>> {
        let mut rows = self.agent_trigger_deliveries.write();
        let claims = |status: &str| status == "dispatched" || status == "filtered";
        if claims(&input.status)
            && let Some(event_id) = input.event_id.as_deref()
            && rows.iter().any(|row| {
                row.trigger_id == input.trigger_id
                    && row.event_id.as_deref() == Some(event_id)
                    && claims(&row.status)
            })
        {
            return Ok(None);
        }
        let row = AgentTriggerDeliveryRow {
            id: Uuid::now_v7(),
            org_id: input.org_id,
            trigger_id: input.trigger_id,
            source: input.source,
            event_id: input.event_id,
            event_type: input.event_type,
            subject: input.subject,
            status: input.status,
            reason: input.reason,
            session_id: None,
            created_at: Self::now(),
        };
        rows.push(row.clone());
        Ok(Some(row))
    }

    pub async fn finish_agent_trigger_delivery(
        &self,
        id: Uuid,
        status: &str,
        reason: Option<&str>,
        session_id: Option<Uuid>,
    ) -> Result<()> {
        if let Some(row) = self
            .agent_trigger_deliveries
            .write()
            .iter_mut()
            .find(|row| row.id == id)
        {
            row.status = status.to_string();
            row.reason = reason.map(ToOwned::to_owned);
            if session_id.is_some() {
                row.session_id = session_id;
            }
        }
        Ok(())
    }

    pub async fn list_agent_trigger_deliveries(
        &self,
        org_id: i64,
        trigger_id: TriggerId,
        limit: i64,
    ) -> Result<Vec<AgentTriggerDeliveryRow>> {
        let mut rows: Vec<_> = self
            .agent_trigger_deliveries
            .read()
            .iter()
            .filter(|row| row.org_id == org_id && row.trigger_id == trigger_id)
            .cloned()
            .collect();
        rows.sort_by_key(|row| std::cmp::Reverse((row.created_at, row.id)));
        rows.truncate(usize::try_from(limit).unwrap_or(0));
        Ok(rows)
    }

    pub async fn prune_agent_trigger_deliveries(
        &self,
        trigger_id: TriggerId,
        keep: i64,
    ) -> Result<u64> {
        let mut rows = self.agent_trigger_deliveries.write();
        let mut mine: Vec<_> = rows
            .iter()
            .filter(|row| row.trigger_id == trigger_id)
            .map(|row| (row.created_at, row.id))
            .collect();
        mine.sort_by_key(|key| std::cmp::Reverse(*key));
        let doomed: std::collections::HashSet<Uuid> = mine
            .into_iter()
            .skip(usize::try_from(keep).unwrap_or(0))
            .map(|(_, id)| id)
            .collect();
        let before = rows.len();
        rows.retain(|row| !doomed.contains(&row.id));
        Ok((before - rows.len()) as u64)
    }
}
