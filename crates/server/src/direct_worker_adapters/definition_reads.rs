// Stored definition loading stays in the server; callers project portable execution views.
use super::*;

impl DirectWorkerAdapters {
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
                tracing::error!("Failed to get agent: {}", e);
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
    pub(super) fn row_to_agent(r: AgentRow, capabilities: Vec<AgentCapabilityConfig>) -> Agent {
        Agent {
            service_virtual_user_id: None,

            public_id: r
                .public_id
                .parse()
                .unwrap_or_else(|_| AgentId::from_uuid(r.id.uuid())),
            internal_id: r.id.uuid(),
            name: r.name,
            display_name: r.display_name,
            description: r.description,
            intro_markdown: None,
            short_description: None,
            starters: Vec::new(),
            system_prompt: r.system_prompt,
            default_model_id: r.default_model_id,
            harness_id: r.harness_id,
            default_version_id: r.default_version_id,
            forked_from_agent_id: r.forked_from_agent_id,
            forked_from_version_id: r.forked_from_version_id,
            root_agent_id: r.root_agent_id,
            tags: r.tags,
            capabilities,
            environments: r
                .environments
                .and_then(|value| serde_json::from_value(value).ok()),
            initial_files: serde_json::from_value(r.initial_files).unwrap_or_default(),
            mcp_servers: serde_json::from_value(r.mcp_servers).unwrap_or_default(),
            network_access: r
                .network_access
                .and_then(|v| serde_json::from_value(v).ok()),
            max_iterations: max_iterations::from_db(r.max_iterations),
            parallel_tool_calls: r.parallel_tool_calls,
            tools: serde_json::from_value(r.tools).unwrap_or_default(),
            status: match r.status.as_str() {
                "active" => AgentStatus::Active,
                "archived" => AgentStatus::Archived,
                "deleted" => AgentStatus::Deleted,
                _ => AgentStatus::Active,
            },
            // Execution-side adapter: exposure state is a control-plane concern.
            exposures_suspended: false,
            exposed: false,
            created_at: r.created_at,
            updated_at: r.updated_at,
            archived_at: r.archived_at,
            deleted_at: r.deleted_at,
            usage: None,
        }
    }
}
