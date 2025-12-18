// Capability service - business logic for capabilities

use anyhow::Result;
use everruns_contracts::{AgentCapability, Capability, CapabilityId};
use everruns_storage::Database;
use std::sync::Arc;
use uuid::Uuid;

use crate::capabilities::{get_capability_definition, get_capability_registry};

pub struct CapabilityService {
    db: Arc<Database>,
}

impl CapabilityService {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// List all available capabilities (public info only)
    pub fn list_all(&self) -> Vec<Capability> {
        get_capability_registry()
            .into_iter()
            .map(|c| c.info)
            .collect()
    }

    /// Get a specific capability by ID
    pub fn get(&self, id: CapabilityId) -> Option<Capability> {
        get_capability_definition(id).map(|c| c.info)
    }

    /// Get capabilities for an agent
    pub async fn get_agent_capabilities(&self, agent_id: Uuid) -> Result<Vec<AgentCapability>> {
        let rows = self.db.get_agent_capabilities(agent_id).await?;

        let capabilities: Vec<AgentCapability> = rows
            .into_iter()
            .filter_map(|row| {
                let capability_id: Result<CapabilityId, _> = row.capability_id.parse();
                capability_id.ok().map(|cap_id| AgentCapability {
                    capability_id: cap_id,
                    position: row.position,
                })
            })
            .collect();

        Ok(capabilities)
    }

    /// Set capabilities for an agent (replaces existing)
    pub async fn set_agent_capabilities(
        &self,
        agent_id: Uuid,
        capabilities: Vec<CapabilityId>,
    ) -> Result<Vec<AgentCapability>> {
        // Convert to (capability_id string, position) tuples
        let cap_tuples: Vec<(String, i32)> = capabilities
            .iter()
            .enumerate()
            .map(|(idx, cap)| (cap.to_string(), idx as i32))
            .collect();

        let rows = self.db.set_agent_capabilities(agent_id, cap_tuples).await?;

        let result: Vec<AgentCapability> = rows
            .into_iter()
            .filter_map(|row| {
                let capability_id: Result<CapabilityId, _> = row.capability_id.parse();
                capability_id.ok().map(|cap_id| AgentCapability {
                    capability_id: cap_id,
                    position: row.position,
                })
            })
            .collect();

        Ok(result)
    }
}
