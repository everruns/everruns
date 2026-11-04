// Stored definition loading stays in the server; callers project portable execution views.
use super::*;

impl DirectWorkerAdapters {
    pub(super) async fn configure_agent_for_org(
        &self,
        org_id: i64,
        agent: &mut Agent,
    ) -> Result<()> {
        self.filter_dreaming(org_id, &mut agent.capabilities)
            .await?;
        crate::platform_chat_agent::configure_page_tools(&self.db, org_id, agent)
            .await
            .map_err(|error| store_error(format!("Failed to resolve page tools: {error}")))
    }

    /// Resolve adoption on every load; no replica-local cache may delay revocation.
    pub(super) async fn filter_dreaming(
        &self,
        org_id: i64,
        capabilities: &mut Vec<AgentCapabilityConfig>,
    ) -> Result<()> {
        if !capabilities.iter().any(|c| c.capability_id() == "dreaming") {
            return Ok(());
        }
        let flags = crate::services::org_feature_flags::resolve_org_feature_flags(
            &self.db,
            org_id,
            &crate::records::FeatureFlagPolicy::current(),
        )
        .await
        .map_err(|error| store_error(error.to_string()))?;
        capabilities.retain(|c| c.capability_id() != "dreaming" || flags.dreaming);
        Ok(())
    }

    /// Load the stored platform Session record (server-internal; EVE-882).
    ///
    /// `WorkerAdapters::get_session` projects this into the portable
    /// `ExecutionSession`; internal paths that need platform-only fields
    /// (scoped-MCP wiring) read the record here.
    pub(crate) async fn get_stored_session(
        &self,
        org_id: i64,
        session_id: Uuid,
    ) -> Result<Option<Session>> {
        let session_id_typed = SessionId::from_uuid(session_id);
        let row = self
            .db
            .get_session(org_id, session_id_typed)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Failed to get session");
                store_error("Failed to get session")
            })?;

        let Some(mut r) = row else {
            return Ok(None);
        };
        if let Some(message) = self.input_message_id {
            if !self
                .db
                .runtime_invocation_exists(r.id, message)
                .await
                .map_err(|e| store_error(e.to_string()))?
            {
                return Err(store_error("Unknown invocation"));
            }
            let responder = self
                .db
                .runtime_invocation_responder(r.id, message)
                .await
                .map_err(|e| store_error(e.to_string()))?
                .map(AgentId::from_uuid);

            if r.agent_id != responder {
                r.agent_revision = None;
            }

            r.agent_id = responder;
        }
        if r.locale.is_none() {
            let profile = match self.input_message_id {
                Some(message) => self
                    .db
                    .runtime_invocation_subject(r.id, message)
                    .await
                    .map_err(|e| store_error(e.to_string()))?,
                None => None,
            };
            if let Some(id) = profile.or(r.virtual_user_id)
                && let Some(profile) = self
                    .db
                    .get_virtual_user(org_id, id)
                    .await
                    .map_err(|e| store_error(e.to_string()))?
            {
                r.locale = profile.locale;
            }
        }
        let capabilities = serde_json::from_value(r.capabilities).unwrap_or_default();
        let mut capabilities =
            crate::domains::capabilities::queries::hydrate_declarative_capability_configs(
                &self.db,
                org_id,
                capabilities,
            )
            .await?;

        self.filter_dreaming(org_id, &mut capabilities).await?;

        Ok(Some({
            // Parse capabilities from JSON
            Session {
                source: crate::domains::sessions::record::SessionSource::from(r.source.as_str()),
                run_summary: r.run_summary.clone(),
                last_turn_status: r.last_turn_status.clone(),
                last_message_at: None,
                activity: crate::domains::sessions::record::SessionActivity::derive(
                    &crate::domains::sessions::record::SessionStatus::from(r.status.as_str()),
                    r.last_turn_status.as_deref(),
                ),
                id: r.id,
                organization_id: everruns_core::org_public_id_from_internal(org_id),
                workspace_id: everruns_contracts::typed_id::WorkspaceId::from_uuid(r.workspace_id),
                harness_id: r.harness_id.unwrap_or_else(|| HarnessId::from_seed(1)),
                agent_id: r.agent_id,
                agent_revision: r.agent_revision,
                virtual_user_id: r.virtual_user_id,
                playground_user_id: r.playground_user_id,
                owner_principal_id: r.owner_principal_id,
                resolved_owner_user_id: r.resolved_owner_user_id,
                owner: None,
                effective_owner: None,
                title: r.title,
                goal: r.goal,
                locale: r.locale,
                preview: None,
                output_preview: None,
                tags: r.tags,
                model_id: r.model_id,
                capabilities,
                tools: serde_json::from_value(r.tools).unwrap_or_default(),
                mcp_servers: serde_json::from_value(r.mcp_servers).unwrap_or_default(),
                system_prompt: r.system_prompt,
                initial_files: serde_json::from_value(r.initial_files).unwrap_or_default(),
                network_access: r
                    .network_access
                    .and_then(|v| serde_json::from_value(v).ok()),
                hints: r.hints.and_then(|v| serde_json::from_value(v).ok()),
                max_iterations: max_iterations::from_db(r.max_iterations),
                parallel_tool_calls: r.parallel_tool_calls,
                status: match r.status.as_str() {
                    "started" => SessionStatus::Started,
                    "active" => SessionStatus::Active,
                    "idle" => SessionStatus::Idle,
                    "running" => SessionStatus::Active,
                    _ => SessionStatus::Started,
                },
                created_at: r.created_at,
                updated_at: r.updated_at,
                started_at: r.started_at,
                finished_at: r.finished_at,
                usage: None,
                is_pinned: None,
                archived_at: None,
                active_schedule_count: None,
                event_count: None,
                task_count: None,
                file_count: None,
                features: vec![],
                parent_session_id: r.parent_session_id,
                forked_from_session_id: r.forked_from_session_id,
                forked_from_sequence: r.forked_from_sequence,
                blueprint_id: r.blueprint_id,
                blueprint_config: r.blueprint_config,
            }
        }))
    }

    /// Build MCP tool definitions from pre-loaded capability rows, through the
    /// same mapping as the gRPC worker path (annotations and saved labels).
    pub(super) async fn build_mcp_tool_definitions_with_capabilities(
        &self,
        org_id: i64,
        capability_rows: &[AgentCapabilityRow],
    ) -> Result<Vec<ToolDefinition>> {
        Ok(
            crate::domains::mcp_servers::tool_labels::org_mcp_tool_definitions(
                &self.mcp_server_service,
                &self.db,
                org_id,
                capability_rows.iter().map(|row| row.capability_id.as_str()),
            )
            .await,
        )
    }

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
                let mut agent = Self::row_to_agent(r, capabilities);
                self.configure_agent_for_org(org_id, &mut agent).await?;
                Ok(Some(agent))
            }
            None => Ok(None),
        }
    }

    pub(crate) async fn get_harness_impl(
        &self,
        org_id: i64,
        harness_id: Uuid,
    ) -> Result<Option<Harness>> {
        let mut harness =
            crate::harness_chain::resolve_effective_harness(&self.db, org_id, harness_id).await?;
        if let Some(harness) = harness.as_mut() {
            self.filter_dreaming(org_id, &mut harness.capabilities)
                .await?;
        }
        Ok(harness)
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
