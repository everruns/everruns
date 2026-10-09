//! Models, agent capabilities, session files, git refs, MCP servers, and plugin installs.

use super::*;

impl StorageBackend {
    /// Create a model with a specific ID (for seeding)
    /// Returns None if model already exists (idempotent)
    pub async fn create_model_with_id(
        &self,
        org_id: i64,
        id: Uuid,
        input: CreateModelRow,
    ) -> Result<Option<ModelRow>> {
        let mut input = input;
        if let Some(provider) = self.get_provider(org_id, input.provider_id.uuid()).await?
            && input
                .provider_metadata
                .as_ref()
                .and_then(|m| m.get(crate::domains::models::catalog::BINDING))
                .is_none()
        {
            input.provider_metadata = Some(crate::domains::models::catalog::assign(
                &provider.provider_type,
                &input.model_id,
                &input.capabilities,
                None,
                None,
                input.provider_metadata,
            )?);
        }
        if let Some(service) = input
            .provider_metadata
            .as_ref()
            .map(|metadata| crate::domains::models::catalog::service(Some(metadata)))
        {
            let tag = service.to_string();
            if !input.capabilities.contains(&tag) {
                input.capabilities.push(tag);
            }
        }
        dispatch!(self, create_model_with_id, org_id, id, input)
    }

    // ============================================
    // Agent Capabilities
    // ============================================

    pub async fn get_agent_capabilities(&self, agent_id: Uuid) -> Result<Vec<AgentCapabilityRow>> {
        #[cfg(test)]
        self.record_session_list_lookup().await;
        dispatch!(self, get_agent_capabilities, agent_id)
    }

    pub async fn get_agent_capabilities_by_agent_ids(
        &self,
        org_id: i64,
        agent_ids: &[AgentId],
    ) -> Result<Vec<AgentCapabilityRow>> {
        #[cfg(test)]
        self.record_session_list_lookup().await;
        dispatch!(self, get_agent_capabilities_by_agent_ids, org_id, agent_ids)
    }

    // ============================================
    // Harness Capabilities
    // ============================================

    pub async fn get_harness_capabilities(
        &self,
        harness_id: Uuid,
    ) -> Result<Vec<HarnessCapabilityRow>> {
        #[cfg(test)]
        self.record_session_list_lookup().await;
        dispatch!(self, get_harness_capabilities, harness_id)
    }

    pub async fn get_harness_capabilities_by_harness_ids(
        &self,
        org_id: i64,
        harness_ids: &[HarnessId],
    ) -> Result<Vec<HarnessCapabilityRow>> {
        #[cfg(test)]
        self.record_session_list_lookup().await;
        dispatch!(
            self,
            get_harness_capabilities_by_harness_ids,
            org_id,
            harness_ids
        )
    }
}
