// Capability service - business logic for capabilities
//
// Uses CapabilityRegistry from everruns-core as the single source of truth
// for capability definitions.
//
// Note: Agent-specific capability management is handled by AgentService.

use crate::storage::Database;
use everruns_core::capabilities::CapabilityRegistry;
use everruns_core::{CapabilityId, CapabilityInfo};
use std::sync::Arc;

pub struct CapabilityService {
    #[allow(dead_code)]
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
}
