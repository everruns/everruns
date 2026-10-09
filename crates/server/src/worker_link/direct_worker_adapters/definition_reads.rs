// Stored definition loading stays in the server; callers project portable execution views.
use super::*;

impl DirectWorkerAdapters {
    pub(super) async fn hydrate_capability_rows(
        &self,
        org_id: i64,
        capability_rows: Vec<AgentCapabilityRow>,
    ) -> Result<Vec<AgentCapabilityConfig>> {
        let capabilities = capability_rows
            .into_iter()
            .map(|c| AgentCapabilityConfig::with_config(c.capability_id, c.config))
            .collect();
        crate::domains::capabilities::queries::hydrate_declarative_capability_configs(
            &self.db,
            org_id,
            capabilities,
        )
        .await
        .map_err(|error| store_error(format!("Failed to hydrate capabilities: {error}")))
    }

    /// Get an agent by public ID (direct DB access).
    pub(super) async fn get_agent_record(
        &self,
        org_id: i64,
        agent_id: Uuid,
    ) -> Result<Option<Agent>> {
        // Look up by public_id, then fetch capabilities using internal id from the row.
        let public_id = AgentId::from_uuid(agent_id).to_string();
        let row = self
            .db
            .get_agent_by_public_id(org_id, &public_id)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Failed to get agent");
                store_error("Failed to get agent")
            })?;
        match row {
            Some(r) => {
                let capabilities = self
                    .db
                    .get_agent_capabilities(r.id.uuid())
                    .await
                    .unwrap_or_default();
                let capabilities = self.hydrate_capability_rows(org_id, capabilities).await?;
                Ok(Some(Self::row_to_agent(r, capabilities)))
            }
            None => Ok(None),
        }
    }

    pub(crate) async fn get_harness_impl(
        &self,
        org_id: i64,
        harness_id: Uuid,
    ) -> Result<Option<Harness>> {
        crate::harness_chain::resolve_effective_harness(&self.db, org_id, harness_id).await
    }

    /// Convert an AgentRow plus pre-loaded capability rows into an Agent.
    ///
    /// Uses the one agent row mapping, then clears what the execution side
    /// never reads: presentation, usage totals, the service identity, and
    /// exposure state are control-plane concerns.
    pub(super) fn row_to_agent(r: AgentRow, capabilities: Vec<AgentCapabilityConfig>) -> Agent {
        Agent {
            avatar: None,
            service_virtual_user_id: None,
            intro_markdown: None,
            short_description: None,
            starters: Vec::new(),
            exposures_suspended: false,
            exposed: false,
            usage: None,
            ..crate::storage::row_to_agent(r, capabilities)
        }
    }
}
