// Capability service - business logic for capabilities
//
// Uses CapabilityRegistry from everruns-core as the single source of truth
// for capability definitions.

use anyhow::Result;
use everruns_core::capabilities::CapabilityRegistry;
use everruns_core::{AgentCapability, CapabilityId, CapabilityInfo};
use everruns_storage::Database;
use std::sync::Arc;
use uuid::Uuid;

pub struct CapabilityService {
    db: Arc<Database>,
    registry: CapabilityRegistry,
}

impl CapabilityService {
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            db,
            registry: CapabilityRegistry::with_builtins(),
        }
    }

    /// List all available capabilities (public info only)
    pub fn list_all(&self) -> Vec<CapabilityInfo> {
        self.registry
            .list()
            .into_iter()
            .map(|cap| CapabilityInfo::from_core(cap.as_ref()))
            .collect()
    }

    /// Get a specific capability by ID
    pub fn get(&self, id: &CapabilityId) -> Option<CapabilityInfo> {
        self.registry
            .get(id.as_str())
            .map(|cap| CapabilityInfo::from_core(cap.as_ref()))
    }

    /// Check if a capability exists in the registry
    pub fn has(&self, id: &CapabilityId) -> bool {
        self.registry.has(id.as_str())
    }

    /// Get capabilities for an agent
    pub async fn get_agent_capabilities(&self, agent_id: Uuid) -> Result<Vec<AgentCapability>> {
        let rows = self.db.get_agent_capabilities(agent_id).await?;

        let capabilities: Vec<AgentCapability> = rows
            .into_iter()
            .map(|row| AgentCapability {
                capability_id: CapabilityId::new(&row.capability_id),
                position: row.position,
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
            .map(|row| AgentCapability {
                capability_id: CapabilityId::new(&row.capability_id),
                position: row.position,
            })
            .collect();

        Ok(result)
    }
}
