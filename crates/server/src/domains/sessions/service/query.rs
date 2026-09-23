//! Reading sessions: get, list, facets, stats, and row hydration.

use super::*;

impl SessionService {
    pub async fn get(
        &self,
        caller: &Caller,
        id: Uuid,
        user_id: Option<Uuid>,
    ) -> Result<Option<Session>> {
        let row = self
            .db
            .get_session(caller.org_id, SessionId::from_uuid(id))
            .await?;
        // A session in another project reads as missing, like a cross-org one.
        // Internal callers (worker, reconcilers) stay org-wide.
        let row = row.filter(|r| caller.is_internal || r.project_id == caller.project_id);
        match row {
            Some(r) => {
                let fallback = if r.harness_id.is_none() {
                    Some(org_init::base_harness_id(&self.db, caller.org_id).await?)
                } else {
                    None
                };
                let mut session = Self::row_to_session(r, &caller.org_public_id, fallback);
                self.hydrate_ownership(caller.org_id, &mut session).await?;
                // Populate features before resolving agent_id (needs internal UUID)
                self.populate_features(caller.org_id, &mut session).await?;
                self.resolve_session_agent_id(caller.org_id, &mut session)
                    .await?;
                // Populate is_pinned if user context available
                if let Some(uid) = user_id {
                    let pinned = self.db.list_pinned_session_ids(uid, caller.org_id).await?;
                    session.is_pinned = Some(pinned.iter().any(|s| s.uuid() == id));
                }
                Ok(Some(session))
            }
            None => Ok(None),
        }
    }

    /// Resolve the model used by turns without a per-message override.
    ///
    /// Follows the runtime precedence: session, agent, harness, organization.
    pub async fn resolved_model_id(
        &self,
        org_id: i64,
        session: &Session,
    ) -> Result<Option<ModelId>> {
        if let Some(model_id) = session.model_id {
            return Ok(self
                .db
                .get_model(org_id, model_id.uuid())
                .await?
                .map(|model| model.id));
        }

        if let Some(agent_id) = session.agent_id {
            let agent = match self
                .db
                .get_agent_by_public_id(org_id, None, &agent_id.to_string())
                .await?
            {
                Some(agent) => Some(agent),
                None => self.db.get_agent(org_id, agent_id).await?,
            };
            if let Some(model_id) = agent.and_then(|agent| agent.default_model_id) {
                return Ok(Some(model_id));
            }
        }

        if let Some(harness) = self
            .resolve_effective_harness(org_id, session.harness_id)
            .await?
            && let Some(model_id) = harness.default_model_id
        {
            return Ok(Some(model_id));
        }

        Ok(self
            .db
            .get_default_model(org_id)
            .await?
            .map(|model| model.id))
    }

    /// Get session counts grouped by status for an organization.
    pub async fn stats(&self, caller: &Caller) -> Result<SessionStats> {
        let counts = self.db.count_sessions_by_status(caller.org_id).await?;
        let mut stats = SessionStats::default();
        for (status, count) in counts {
            let count = count as u32;
            stats.total += count;
            match status.as_str() {
                "active" => stats.active = count,
                "idle" => stats.idle = count,
                "started" => stats.started = count,
                "waiting_for_tool_results" => stats.waiting_for_tool_results = count,
                _ => {} // ignore unknown statuses
            }
        }
        Ok(stats)
    }

    /// List sessions for an organization with optional agent filter.
    /// Returns (sessions, total_count).
    /// Sessions include preview text from first user message and last assistant response.
    pub async fn list(
        &self,
        caller: &Caller,
        user_id: Option<Uuid>,
        filters: &SessionListFilters,
        pagination: Pagination,
    ) -> Result<(Vec<Session>, u32)> {
        let org_id = caller.org_id;
        let org_public_id = &caller.org_public_id;
        let (rows, total) = self.db.list_sessions(org_id, filters, pagination).await?;
        let fallback = if rows.iter().any(|r| r.harness_id.is_none()) {
            Some(org_init::base_harness_id(&self.db, org_id).await?)
        } else {
            None
        };
        let mut sessions: Vec<Session> = rows
            .into_iter()
            .map(|r| Self::row_to_session(r, org_public_id, fallback))
            .collect();

        if sessions.is_empty() {
            return Ok((sessions, total));
        }

        let hydration = self.load_session_list_hydration(org_id, &sessions).await?;
        self.apply_session_list_hydration(&mut sessions, &hydration);

        // Fetch previews for all sessions in batch queries
        let session_ids: Vec<Uuid> = sessions.iter().map(|s| s.id.uuid()).collect();
        let input_previews = self.db.get_session_previews(&session_ids).await?;
        let output_previews = self.db.get_session_output_previews(&session_ids).await?;

        // Populate previews for each session
        for session in &mut sessions {
            if let Some(preview) = input_previews.get(&session.id.uuid()) {
                session.preview = Some(preview.clone());
            }
            if let Some(preview) = output_previews.get(&session.id.uuid()) {
                session.output_preview = Some(preview.clone());
            }
        }

        // Populate is_pinned if user context available
        if let Some(uid) = user_id {
            let pinned_ids = self.db.list_pinned_session_ids(uid, org_id).await?;
            let pinned_set: std::collections::HashSet<Uuid> =
                pinned_ids.iter().map(|id| id.uuid()).collect();
            for session in &mut sessions {
                session.is_pinned = Some(pinned_set.contains(&session.id.uuid()));
            }
        }

        Ok((sessions, total))
    }

    /// Facet-rail counts and masthead metrics for the sessions surface
    /// (EVE-852), aggregated over the same predicate as [`Self::list`].
    ///
    /// Agent buckets are returned keyed by the agent's public id so the caller
    /// never has to expose or resolve internal UUIDs.
    pub async fn facets(
        &self,
        caller: &Caller,
        filters: &SessionListFilters,
    ) -> Result<SessionFacetsResponse> {
        let row = self.db.session_facets(caller.org_id, filters).await?;

        let bucket = |buckets: Vec<crate::storage::SessionFacetBucket>| {
            buckets
                .into_iter()
                .map(|b| SessionFacetCount {
                    value: b.value,
                    count: b.count as u64,
                })
                .collect::<Vec<_>>()
        };

        Ok(SessionFacetsResponse {
            total: row.total as u64,
            by_activity: bucket(row.by_activity),
            by_source: bucket(row.by_source),
            by_agent: bucket(row.by_agent),
            active_now: row.active_now as u64,
            failed_today: row.failed_today as u64,
            p95_duration_ms: row.p95_duration_ms as u64,
            tokens_today: row.tokens_today as u64,
        })
    }

    pub(crate) async fn load_session_list_hydration(
        &self,
        org_id: i64,
        sessions: &[Session],
    ) -> Result<SessionListHydration> {
        // THREAT[TM-TENANT-001]: every batch loader receives the caller's org_id;
        // capability-table reads additionally join through their org-scoped owner.
        let principal_ids: Vec<PrincipalId> = sessions
            .iter()
            .map(|session| session.owner_principal_id)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let resolved_user_ids: Vec<Uuid> = sessions
            .iter()
            .filter_map(|session| session.resolved_owner_user_id)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let principal_rows = self
            .db
            .get_principals_for_session_list(org_id, &principal_ids, &resolved_user_ids)
            .await?;

        let mut hydration = SessionListHydration::default();
        for row in principal_rows {
            let principal = row_to_principal(row);
            if principal.status != everruns_platform::PrincipalStatus::Deleted {
                hydration.owners.insert(principal.id, principal.summary());
            }
            if principal.kind == everruns_core::PrincipalKind::User
                && let Some(user_id) = principal.subject_id
            {
                hydration
                    .effective_owners
                    .insert(user_id, principal.summary());
            }
        }

        let agent_ids: Vec<AgentId> = sessions
            .iter()
            .filter_map(|session| session.agent_id)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        if !agent_ids.is_empty() {
            let agent_rows = self.db.get_agents_by_ids(org_id, &agent_ids).await?;
            let existing_agent_ids: Vec<AgentId> = agent_rows.iter().map(|row| row.id).collect();
            for row in agent_rows {
                if let Ok(public_id) = row.public_id.parse::<AgentId>() {
                    hydration.agent_public_ids.insert(row.id, public_id);
                }
            }
            for row in self
                .db
                .get_agent_capabilities_by_agent_ids(org_id, &existing_agent_ids)
                .await?
            {
                let capability_ids = hydration
                    .agent_capability_ids
                    .entry(row.agent_id)
                    .or_default();
                if !capability_ids.contains(&row.capability_id) {
                    capability_ids.push(row.capability_id);
                }
            }
        }

        let harness_ids: HashSet<HarnessId> =
            sessions.iter().map(|session| session.harness_id).collect();
        hydration.harness_capability_ids = self
            .load_session_list_harness_capability_ids(org_id, harness_ids)
            .await?;

        Ok(hydration)
    }

    pub(crate) async fn load_session_list_harness_capability_ids(
        &self,
        org_id: i64,
        root_ids: HashSet<HarnessId>,
    ) -> Result<HashMap<HarnessId, Vec<String>>> {
        let root_id_list: Vec<HarnessId> = root_ids.iter().copied().collect();
        let rows_by_id: HashMap<HarnessId, _> = self
            .db
            .get_harness_ancestry_by_ids(org_id, &root_id_list)
            .await?
            .into_iter()
            .map(|row| (row.id, row))
            .collect();

        let loaded_ids: Vec<HarnessId> = rows_by_id.keys().copied().collect();
        let mut layer_capability_ids: HashMap<HarnessId, Vec<String>> = HashMap::new();
        for row in self
            .db
            .get_harness_capabilities_by_harness_ids(org_id, &loaded_ids)
            .await?
        {
            layer_capability_ids
                .entry(row.harness_id)
                .or_default()
                .push(row.capability_id);
        }

        let mut effective_by_root = HashMap::new();
        for root_id in root_ids {
            if !rows_by_id.contains_key(&root_id) {
                continue;
            }
            let mut chain = Vec::new();
            let mut visited = HashSet::new();
            let mut cursor = Some(root_id);
            while let Some(id) = cursor {
                if !visited.insert(id) {
                    anyhow::bail!("Harness inheritance cycle detected");
                }
                let row = rows_by_id
                    .get(&id)
                    .ok_or_else(|| ResourceNotFoundError::new("Parent harness"))?;
                chain.push(id);
                cursor = row.parent_harness_id;
            }

            let mut capability_ids = Vec::new();
            for id in chain.into_iter().rev() {
                for capability_id in layer_capability_ids.get(&id).into_iter().flatten() {
                    if !capability_ids.contains(capability_id) {
                        capability_ids.push(capability_id.clone());
                    }
                }
            }
            effective_by_root.insert(root_id, capability_ids);
        }

        Ok(effective_by_root)
    }

    pub(crate) fn apply_session_list_hydration(
        &self,
        sessions: &mut [Session],
        hydration: &SessionListHydration,
    ) {
        for session in sessions {
            session.owner = hydration.owners.get(&session.owner_principal_id).cloned();
            session.effective_owner = session
                .resolved_owner_user_id
                .and_then(|id| hydration.effective_owners.get(&id).cloned());

            let agent_internal_id = session.agent_id;
            let mut capability_ids = hydration
                .harness_capability_ids
                .get(&session.harness_id)
                .cloned()
                .unwrap_or_default();
            if let Some(agent_id) = agent_internal_id
                && let Some(agent_capability_ids) = hydration.agent_capability_ids.get(&agent_id)
            {
                for capability_id in agent_capability_ids {
                    if !capability_ids.contains(capability_id) {
                        capability_ids.push(capability_id.clone());
                    }
                }
            }
            for capability in &session.capabilities {
                let capability_id = capability.capability_id().to_string();
                if !capability_ids.contains(&capability_id) {
                    capability_ids.push(capability_id);
                }
            }
            session.features = compute_features(&capability_ids, &self.capability_registry);

            if let Some(agent_id) = agent_internal_id
                && let Some(public_id) = hydration.agent_public_ids.get(&agent_id)
            {
                session.agent_id = Some(*public_id);
            }
        }
    }

    pub fn row_to_session(
        row: crate::storage::SessionRow,
        org_public_id: &str,
        fallback_harness: Option<HarnessId>,
    ) -> Session {
        // Convert database usage columns to TokenUsage. Actual and estimated cost
        // totals are tracked separately; the aggregate carries each so consumers
        // can prefer actual and reconcile drift.
        let usage = if row.total_input_tokens > 0 || row.total_output_tokens > 0 {
            Some(
                TokenUsage::with_cache(
                    row.total_input_tokens as u32,
                    row.total_output_tokens as u32,
                    if row.total_cache_read_tokens > 0 {
                        Some(row.total_cache_read_tokens as u32)
                    } else {
                        None
                    },
                    if row.total_cache_creation_tokens > 0 {
                        Some(row.total_cache_creation_tokens as u32)
                    } else {
                        None
                    },
                )
                .with_cost(
                    (row.total_actual_cost_usd > 0.0).then_some(row.total_actual_cost_usd),
                    (row.total_estimated_cost_usd > 0.0).then_some(row.total_estimated_cost_usd),
                )
                .with_effective_cost((row.total_cost_usd > 0.0).then_some(row.total_cost_usd)),
            )
        } else {
            None
        };

        // Parse capabilities from JSON
        let capabilities: Vec<AgentCapabilityConfig> =
            serde_json::from_value(row.capabilities).unwrap_or_default();

        Session {
            id: row.id,
            organization_id: org_public_id.to_string(),
            workspace_id: WorkspaceId::from_uuid(row.workspace_id),
            harness_id: row.harness_id.or(fallback_harness).unwrap_or_else(|| {
                panic!(
                    "session {} has no harness_id and no fallback was provided; \
                     ensure the org has a built-in 'base' harness provisioned",
                    row.id
                )
            }),
            agent_id: row.agent_id,
            agent_version_id: row.agent_version_id,
            agent_identity_id: row.agent_identity_id,
            owner_principal_id: row.owner_principal_id,
            resolved_owner_user_id: row.resolved_owner_user_id,
            owner: None,
            effective_owner: None,
            title: row.title,
            goal: row.goal,
            locale: row.locale,
            preview: None,        // Populated separately in list()
            output_preview: None, // Populated separately in list()
            tags: row.tags,
            model_id: row.model_id,
            capabilities,
            tools: serde_json::from_value(row.tools).unwrap_or_default(),
            mcp_servers: serde_json::from_value(row.mcp_servers).unwrap_or_default(),
            system_prompt: row.system_prompt,
            initial_files: serde_json::from_value(row.initial_files).unwrap_or_default(),
            network_access: row
                .network_access
                .and_then(|v| serde_json::from_value(v).ok()),
            hints: row.hints.and_then(|v| serde_json::from_value(v).ok()),
            max_iterations: max_iterations::from_db(row.max_iterations),
            parallel_tool_calls: row.parallel_tool_calls,
            status: SessionStatus::from(row.status.as_str()),
            source: SessionSource::from(row.source.as_str()),
            activity: SessionActivity::derive(
                &SessionStatus::from(row.status.as_str()),
                row.last_turn_status.as_deref(),
            ),
            run_summary: row.run_summary.clone(),
            created_at: row.created_at,
            updated_at: row.updated_at,
            started_at: row.started_at,
            finished_at: row.finished_at,
            usage,
            is_pinned: None, // Populated by caller with user context
            archived_at: row.archived_at,
            active_schedule_count: None, // Populated by caller
            // Denormalized tab counts (EVE-868). Projections that do not select
            // the counters decode them as 0, which is also the "nothing behind
            // this tab" value — so zero is reported as absent, and an empty tab
            // renders with no badge rather than a `0`.
            event_count: (row.event_count > 0).then_some(row.event_count as u32),
            task_count: (row.task_count > 0).then_some(row.task_count as u32),
            file_count: (row.workspace_file_count > 0).then_some(row.workspace_file_count as u32),
            features: vec![], // Populated by caller via populate_features()
            parent_session_id: row.parent_session_id,
            forked_from_session_id: row.forked_from_session_id,
            forked_from_sequence: row.forked_from_sequence,
            blueprint_id: row.blueprint_id,
            blueprint_config: row.blueprint_config,
        }
    }

    /// Populate the `features` field on a session by aggregating features from
    /// all active capabilities (harness + agent + session-level).
    ///
    /// Must be called BEFORE `resolve_session_agent_id()` because the session's
    /// agent_id at that point is still the internal UUID needed for DB lookups.
    pub(crate) async fn populate_features(&self, org_id: i64, session: &mut Session) -> Result<()> {
        let harness_id = session.harness_id.uuid();
        let agent_internal_id = session.agent_id.map(|a| a.uuid());

        let capability_ids = self
            .collect_session_capability_ids(
                org_id,
                harness_id,
                agent_internal_id,
                &session.capabilities,
            )
            .await?;

        session.features = compute_features(&capability_ids, &self.capability_registry);
        Ok(())
    }

    pub(crate) async fn hydrate_ownership(&self, org_id: i64, session: &mut Session) -> Result<()> {
        session.owner = self
            .principal_service
            .get_summary(org_id, session.owner_principal_id)
            .await?;
        session.effective_owner = self
            .principal_service
            .effective_owner_summary(org_id, session.resolved_owner_user_id)
            .await?;
        Ok(())
    }

    pub(crate) async fn resolve_effective_harness(
        &self,
        org_id: i64,
        harness_id: HarnessId,
    ) -> Result<Option<everruns_platform::Harness>> {
        resolve_effective_harness(self.db.as_ref(), org_id, harness_id).await
    }
}
