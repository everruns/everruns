// In-memory storage: Sessions, Pinned Sessions

use super::super::models::*;
use super::InMemoryDatabase;
use super::matches_search_tokens;
use crate::kernel_imports::{
    everruns_provider::typed_id::AgentId, everruns_provider::typed_id::EventId,
    everruns_provider::typed_id::HarnessId, everruns_provider::typed_id::PrincipalId,
    everruns_provider::typed_id::SessionId,
};
use anyhow::Result;
use chrono::{DateTime, Utc};
use everruns_platform::{SessionActivity, SessionStatus};

use uuid::Uuid;

/// Facet dimension whose own selection is excluded when counting it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FacetDimension {
    None,
    Activity,
    Source,
    Agent,
}

fn session_activity(row: &SessionRow) -> SessionActivity {
    SessionActivity::derive(
        &SessionStatus::from(row.status.as_str()),
        row.last_turn_status.as_deref(),
    )
}

fn count_by(values: impl Iterator<Item = String>) -> Vec<SessionFacetBucket> {
    let mut counts: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    for value in values {
        *counts.entry(value).or_default() += 1;
    }
    counts
        .into_iter()
        .map(|(value, count)| SessionFacetBucket { value, count })
        .collect()
}

impl InMemoryDatabase {
    // ============================================
    // Sessions
    // ============================================

    pub async fn create_session(&self, input: CreateSessionRow) -> Result<SessionRow> {
        let now = Self::now();
        let id = SessionId::new();

        // THREAT[TM-TENANT-014]: Resolve the explicit detached budget root
        // under the creating org.
        // Canonicalizing through the referenced row preserves the origin root
        // across detached chains and rejects cross-org linkage.
        let root_session_id = if let Some(budget_root) = input.budget_root_session_id {
            let sessions = self.sessions.read();
            let referenced = sessions
                .get(&budget_root)
                .filter(|row| row.org_id == input.org_id)
                .ok_or_else(|| anyhow::anyhow!("budget root session not found in organization"))?;
            referenced.root_session_id.unwrap_or(budget_root)
        } else {
            match input.parent_session_id {
                Some(parent) => self
                    .sessions
                    .read()
                    .get(&parent)
                    .and_then(|p| p.root_session_id)
                    .unwrap_or(parent),
                None => id,
            }
        };

        // Attach to an existing workspace when requested (validated by the
        // service before reaching storage); otherwise auto-create a default
        // workspace whose UUID equals the session id — matches the Postgres
        // path and the migration's equality invariant (see knowledge/runtime-resources/workspace.md).
        let session_uuid = id.uuid();
        let workspace_id = input.workspace_id.unwrap_or(session_uuid);
        if input.workspace_id.is_none() {
            let ws_id_hex = session_uuid.simple().to_string();
            let ws_name = format!("session-{ws_id_hex}");
            let public_id = format!("wsp_{ws_id_hex}");
            self.workspaces.write().insert(
                session_uuid,
                crate::storage::models::WorkspaceRow {
                    id: session_uuid,
                    org_id: input.org_id,
                    public_id,
                    name: ws_name.clone(),
                    description: Some(format!("Default workspace for session {session_uuid}")),
                    owner_principal_id: None,
                    resolved_owner_user_id: input.resolved_owner_user_id,
                    status: "active".to_string(),
                    created_at: now,
                    updated_at: now,
                    archived_at: None,
                    deleted_at: None,
                },
            );
        }
        let row = SessionRow {
            id,
            org_id: input.org_id,
            workspace_id,
            app_id: input.app_id,
            harness_id: input.harness_id,
            agent_id: input.agent_id,
            agent_version_id: input.agent_version_id,
            agent_config_hash: input.agent_config_hash,
            agent_identity_id: input.agent_identity_id,
            owner_principal_id: input.owner_principal_id,
            resolved_owner_user_id: input.resolved_owner_user_id,
            title: input.title,
            goal: None,
            locale: input.locale,
            tags: input.tags,
            model_id: input.model_id,
            capabilities: input.capabilities,
            tools: input.tools,
            mcp_servers: input.mcp_servers,
            system_prompt: input.system_prompt,
            initial_files: input.initial_files,
            hints: input.hints,
            network_access: input.network_access,
            max_iterations: input.max_iterations,
            parallel_tool_calls: input.parallel_tool_calls,
            status: "started".to_string(),
            source: input.source.as_str().to_string(),
            last_turn_status: None,
            last_turn_at: None,
            run_summary: None,
            run_summary_turn_sequence: None,
            created_at: now,
            updated_at: now,
            started_at: None,
            finished_at: None,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            turn_count: 0,
            tool_call_count: 0,
            total_actual_cost_usd: 0.0,
            total_estimated_cost_usd: 0.0,
            total_cost_usd: 0.0,
            parent_session_id: input.parent_session_id,
            root_session_id: Some(root_session_id),
            forked_from_session_id: None,
            forked_from_sequence: None,
            blueprint_id: input.blueprint_id,
            blueprint_config: input.blueprint_config,
            archived_at: None,
        };
        self.sessions.write().insert(id, row.clone());
        self.insert_initial_session_participants(&row).await?;
        self.enqueue_reporting_outbox(
            row.org_id,
            "session",
            &row.id.uuid().to_string(),
            Some(&row.updated_at.to_rfc3339()),
            "session_snapshot",
        )
        .await?;
        Ok(row)
    }

    /// Record fork provenance on an already-created session
    /// (knowledge/runtime-resources/forking-sessions.md). No-op if the session id is unknown.
    pub async fn set_session_fork_lineage(
        &self,
        session_id: SessionId,
        forked_from_session_id: SessionId,
        forked_from_sequence: Option<i32>,
    ) -> Result<()> {
        if let Some(row) = self.sessions.write().get_mut(&session_id) {
            row.forked_from_session_id = Some(forked_from_session_id);
            row.forked_from_sequence = forked_from_sequence;
            row.updated_at = Self::now();
        }
        Ok(())
    }

    /// Get session, validating org ownership directly
    pub async fn get_session(&self, org_id: i64, id: SessionId) -> Result<Option<SessionRow>> {
        let sessions = self.sessions.read();
        if let Some(session) = sessions.get(&id) {
            // Validate that the session belongs to the org
            if session.org_id == org_id {
                return Ok(Some(session.clone()));
            }
        }
        Ok(None)
    }

    /// Get session without org scoping. For internal system use only (e.g. usage tracking).
    pub async fn get_session_unscoped(&self, id: SessionId) -> Result<Option<SessionRow>> {
        let sessions = self.sessions.read();
        Ok(sessions.get(&id).cloned())
    }

    /// List sessions for an agent with pagination, validating org ownership.
    /// Returns (sessions, total_count).
    /// Rows matching every filter except the dimensions named in `skip`, so the
    /// facet rail can count a dimension with its own selection excluded.
    fn filtered_sessions(
        &self,
        org_id: i64,
        filters: &SessionListFilters,
        skip: FacetDimension,
    ) -> Vec<SessionRow> {
        let sessions = self.sessions.read();
        sessions
            .values()
            .filter(|s| s.org_id == org_id)
            .filter(|s| {
                skip == FacetDimension::Agent
                    || filters.agent_id.is_none_or(|aid| s.agent_id == Some(aid))
            })
            .filter(|s| {
                matches_search_tokens(
                    filters.search.as_deref(),
                    &[s.title.as_deref().unwrap_or("")],
                )
            })
            .filter(|s| {
                filters
                    .owner_user_id
                    .is_none_or(|uid| s.resolved_owner_user_id == Some(uid))
            })
            .filter(|s| filters.include_archived || s.archived_at.is_none())
            .filter(|s| filters.created_after.is_none_or(|t| s.created_at >= t))
            .filter(|s| filters.created_before.is_none_or(|t| s.created_at < t))
            .filter(|s| {
                skip == FacetDimension::Source
                    || filters.sources.is_empty()
                    || filters
                        .sources
                        .iter()
                        .any(|src| src.as_str() == s.source.as_str())
            })
            .filter(|s| {
                skip == FacetDimension::Activity
                    || filters.activities.is_empty()
                    || filters.activities.contains(&session_activity(s))
            })
            .cloned()
            .collect()
    }

    pub async fn list_sessions(
        &self,
        org_id: i64,
        filters: &SessionListFilters,
        pagination: crate::api::common::Pagination,
    ) -> Result<(Vec<SessionRow>, u32)> {
        // If agent_id is provided, validate it belongs to the org
        if let Some(aid) = filters.agent_id {
            let agents = self.agents.read();
            if !agents
                .get(&aid)
                .map(|a| a.org_id == org_id)
                .unwrap_or(false)
            {
                return Ok((vec![], 0));
            }
        }

        let mut result = self.filtered_sessions(org_id, filters, FacetDimension::None);
        match filters.order {
            SessionListOrder::CreatedAt => {
                result.sort_by_key(|session| std::cmp::Reverse(session.created_at))
            }
            SessionListOrder::LastActivity => {
                result.sort_by_key(|session| std::cmp::Reverse(session.updated_at))
            }
        }

        let total = result.len() as u32;
        let offset = pagination.offset as usize;
        let limit = pagination.limit as usize;

        // Apply pagination
        let paginated = result.into_iter().skip(offset).take(limit).collect();

        Ok((paginated, total))
    }

    pub async fn session_facets(
        &self,
        org_id: i64,
        filters: &SessionListFilters,
    ) -> Result<SessionFacetsRow> {
        if let Some(aid) = filters.agent_id
            && !self
                .agents
                .read()
                .get(&aid)
                .map(|a| a.org_id == org_id)
                .unwrap_or(false)
        {
            return Ok(SessionFacetsRow::default());
        }

        let matched = self.filtered_sessions(org_id, filters, FacetDimension::None);
        let today = Utc::now().date_naive();

        let mut durations: Vec<i64> = matched
            .iter()
            .map(|s| {
                let end = s.finished_at.or(s.last_turn_at).unwrap_or(s.updated_at);
                let start = s.started_at.unwrap_or(s.created_at);
                (end - start).num_milliseconds().max(0)
            })
            .collect();
        durations.sort_unstable();
        // Matches PostgreSQL's `percentile_cont` at the discrete boundary well
        // enough for the in-memory backend's test-fixture scale.
        let p95_duration_ms = if durations.is_empty() {
            0
        } else {
            let idx = (((durations.len() - 1) as f64) * 0.95).round() as usize;
            durations[idx]
        };

        Ok(SessionFacetsRow {
            total: matched.len() as i64,
            by_activity: count_by(
                self.filtered_sessions(org_id, filters, FacetDimension::Activity)
                    .iter()
                    .map(|s| session_activity(s).as_str().to_string()),
            ),
            by_source: count_by(
                self.filtered_sessions(org_id, filters, FacetDimension::Source)
                    .iter()
                    .map(|s| s.source.clone()),
            ),
            by_agent: {
                let agents = self.agents.read();
                count_by(
                    self.filtered_sessions(org_id, filters, FacetDimension::Agent)
                        .iter()
                        .filter_map(|s| {
                            s.agent_id
                                .and_then(|agent_id| agents.get(&agent_id))
                                .map(|agent| agent.public_id.clone())
                        }),
                )
            },
            active_now: matched
                .iter()
                .filter(|s| matches!(s.status.as_str(), "active" | "waiting_for_tool_results"))
                .count() as i64,
            failed_today: matched
                .iter()
                .filter(|s| {
                    matches!(
                        s.last_turn_status.as_deref(),
                        Some("failed") | Some("cancelled")
                    ) && s.last_turn_at.is_some_and(|t| t.date_naive() == today)
                })
                .count() as i64,
            p95_duration_ms,
            tokens_today: matched
                .iter()
                .filter(|s| s.created_at.date_naive() == today)
                .map(|s| {
                    s.total_input_tokens
                        + s.total_output_tokens
                        + s.total_cache_read_tokens
                        + s.total_cache_creation_tokens
                })
                .sum(),
        })
    }

    pub async fn count_sessions_for_agent(&self, org_id: i64, agent_id: AgentId) -> Result<u64> {
        Ok(self
            .sessions
            .read()
            .values()
            .filter(|s| s.org_id == org_id && s.agent_id == Some(agent_id))
            .count() as u64)
    }

    pub async fn count_sessions_for_harness(
        &self,
        org_id: i64,
        harness_id: HarnessId,
    ) -> Result<u64> {
        Ok(self
            .sessions
            .read()
            .values()
            .filter(|s| s.org_id == org_id && s.harness_id == Some(harness_id))
            .count() as u64)
    }

    pub async fn count_sessions_for_harnesses(
        &self,
        org_id: i64,
        harness_ids: &[HarnessId],
    ) -> Result<Vec<(HarnessId, i64)>> {
        let mut counts = std::collections::HashMap::<HarnessId, i64>::new();
        for session in self.sessions.read().values() {
            if session.org_id == org_id
                && let Some(harness_id) = session.harness_id
                && harness_ids.contains(&harness_id)
            {
                *counts.entry(harness_id).or_default() += 1;
            }
        }
        Ok(counts.into_iter().collect())
    }

    /// Count sessions in an org (for resource limits). Sessions are hard-deleted
    /// (no soft-delete status), so every stored row counts toward the cap.
    pub async fn count_sessions_for_org(&self, org_id: i64) -> Result<i64> {
        Ok(self
            .sessions
            .read()
            .values()
            .filter(|s| s.org_id == org_id)
            .count() as i64)
    }

    /// List child sessions (subagents) for a parent session.
    pub async fn list_child_sessions(
        &self,
        parent_session_id: SessionId,
    ) -> Result<Vec<SessionRow>> {
        let sessions = self.sessions.read();
        let mut result: Vec<_> = sessions
            .values()
            .filter(|s| s.parent_session_id == Some(parent_session_id))
            .cloned()
            .collect();
        result.sort_by_key(|message| message.created_at);
        Ok(result)
    }

    /// Count sessions grouped by status for an organization.
    pub async fn count_sessions_by_status(&self, org_id: i64) -> Result<Vec<(String, i64)>> {
        let sessions = self.sessions.read();
        let mut counts: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
        for s in sessions.values() {
            if s.org_id == org_id {
                *counts.entry(s.status.clone()).or_default() += 1;
            }
        }
        Ok(counts.into_iter().collect())
    }

    /// Count non-finished sessions for an org (EVE-508 concurrent session cap).
    pub async fn count_active_sessions_for_org(&self, org_id: i64) -> Result<i64> {
        let sessions = self.sessions.read();
        let count = sessions
            .values()
            .filter(|s| {
                s.org_id == org_id
                    && matches!(
                        s.status.as_str(),
                        "active" | "idle" | "started" | "waiting_for_tool_results" | "paused"
                    )
            })
            .count();
        Ok(count as i64)
    }

    /// Count sessions currently executing a turn for an org (EVE-508 active turn cap).
    pub async fn count_active_turns_for_org(&self, org_id: i64) -> Result<i64> {
        let sessions = self.sessions.read();
        let count = sessions
            .values()
            .filter(|s| s.org_id == org_id && s.status == "active")
            .count();
        Ok(count as i64)
    }

    /// Atomically reserve active-turn capacity by marking the accepted
    /// session active before the user message is persisted.
    pub async fn reserve_active_turn_slot_for_org(
        &self,
        org_id: i64,
        session_id: SessionId,
        max_active_turns: i64,
    ) -> Result<ReserveActiveTurnSlotResult> {
        let mut sessions = self.sessions.write();

        // Existence/ownership before capacity (mirror Postgres): capture the
        // prior status for release, and report a missing/foreign session as
        // SessionNotFound rather than AtCapacity.
        let previous_status = match sessions.get(&session_id) {
            Some(session) if session.org_id == org_id => session.status.clone(),
            _ => return Ok(ReserveActiveTurnSlotResult::SessionNotFound),
        };

        let active_turns = sessions
            .values()
            .filter(|s| s.org_id == org_id && s.status == "active")
            .count() as i64;

        if active_turns >= max_active_turns {
            return Ok(ReserveActiveTurnSlotResult::AtCapacity { active_turns });
        }

        let session = sessions
            .get_mut(&session_id)
            .expect("session presence checked above");
        session.status = "active".to_string();
        session.updated_at = Self::now();
        Ok(ReserveActiveTurnSlotResult::Reserved { previous_status })
    }

    /// Release a previously reserved active-turn slot by restoring the prior
    /// status. Best-effort: only reverts a session still in `active`.
    pub async fn release_active_turn_slot_for_org(
        &self,
        org_id: i64,
        session_id: SessionId,
        previous_status: &str,
    ) -> Result<()> {
        let mut sessions = self.sessions.write();
        if let Some(session) = sessions.get_mut(&session_id)
            && session.org_id == org_id
            && session.status == "active"
        {
            session.status = previous_status.to_string();
            session.updated_at = Self::now();
        }
        Ok(())
    }

    /// Aggregate session and turn execution stats for an optional agent or harness scope.
    pub async fn session_aggregate_stats(
        &self,
        org_id: i64,
        agent_id: Option<AgentId>,
        harness_id: Option<HarnessId>,
    ) -> Result<SessionAggregateStatsRow> {
        let sessions = self.sessions.read();
        let events = self.events.read();
        let mut stats = SessionAggregateStatsRow::default();
        let mut session_ids = std::collections::HashSet::new();

        for session in sessions.values().filter(|session| {
            session.org_id == org_id
                && agent_id.is_none_or(|id| session.agent_id == Some(id))
                && harness_id.is_none_or(|id| session.harness_id == Some(id))
        }) {
            stats.session_count += 1;
            match session.status.as_str() {
                "active" => stats.active_session_count += 1,
                "idle" => stats.idle_session_count += 1,
                "started" => stats.started_session_count += 1,
                "waiting_for_tool_results" => stats.waiting_for_tool_results_session_count += 1,
                _ => {}
            }

            let start = session.started_at.unwrap_or(session.created_at);
            let end = session.finished_at.unwrap_or(session.updated_at);
            let duration_ms = end.signed_duration_since(start).num_milliseconds().max(0);
            stats.total_session_duration_ms += duration_ms;
            stats.total_input_tokens += session.total_input_tokens;
            stats.total_output_tokens += session.total_output_tokens;
            stats.total_cache_read_tokens += session.total_cache_read_tokens;
            stats.total_cache_creation_tokens += session.total_cache_creation_tokens;
            stats.total_actual_cost_usd += session.total_actual_cost_usd;
            stats.total_estimated_cost_usd += session.total_estimated_cost_usd;
            stats.total_cost_usd += session.total_cost_usd;
            stats.first_session_at = Some(
                stats
                    .first_session_at
                    .map_or(session.created_at, |current| {
                        current.min(session.created_at)
                    }),
            );
            stats.last_session_at =
                Some(stats.last_session_at.map_or(session.created_at, |current| {
                    current.max(session.created_at)
                }));
            session_ids.insert(session.id);
        }

        for event in events.values().filter(|event| {
            session_ids.contains(&event.session_id) && event.event_type == "turn.started"
        }) {
            stats.execution_count += 1;
            stats.last_execution_at = Some(
                stats
                    .last_execution_at
                    .map_or(event.ts, |current| current.max(event.ts)),
            );
        }

        Ok(stats)
    }

    /// Find active sessions owned by apps with Slack channel configuration.
    pub async fn find_active_slack_sessions(&self) -> Result<Vec<SessionRow>> {
        let sessions = self.sessions.read();
        let apps = self.apps.read();
        let result: Vec<_> = sessions
            .values()
            .filter(|s| {
                s.status == "active"
                    && s.app_id
                        .and_then(|app_id| apps.get(&app_id))
                        .is_some_and(|app| {
                            app.channel_type.as_deref() == Some("slack") && app.status != "deleted"
                        })
            })
            .cloned()
            .collect();
        Ok(result)
    }

    /// Find sessions in `waiting_for_tool_results` with updated_at before cutoff.
    pub async fn list_sessions_waiting_tool_results_before(
        &self,
        cutoff: DateTime<Utc>,
    ) -> Result<Vec<(SessionId, i64)>> {
        let sessions = self.sessions.read();
        let result: Vec<_> = sessions
            .values()
            .filter(|s| s.status == "waiting_for_tool_results" && s.updated_at < cutoff)
            .map(|s| (s.id, s.org_id))
            .collect();
        Ok(result)
    }

    /// Find a single session matching ALL given tags within an org.
    pub async fn find_session_by_tags(
        &self,
        org_id: i64,
        tags: &[String],
    ) -> Result<Option<SessionRow>> {
        let sessions = self.sessions.read();
        let mut result: Vec<_> = sessions
            .values()
            .filter(|s| s.org_id == org_id && tags.iter().all(|tag| s.tags.contains(tag)))
            .cloned()
            .collect();
        result.sort_by_key(|session| session.created_at);
        Ok(result.into_iter().next())
    }

    /// Find a single app-owned session matching ALL given tags within an org.
    pub async fn find_app_session_by_tags(
        &self,
        org_id: i64,
        app_id: Uuid,
        tags: &[String],
    ) -> Result<Option<SessionRow>> {
        let sessions = self.sessions.read();
        let mut result: Vec<_> = sessions
            .values()
            .filter(|s| {
                s.org_id == org_id
                    && s.app_id == Some(app_id)
                    && tags.iter().all(|tag| s.tags.contains(tag))
            })
            .cloned()
            .collect();
        result.sort_by_key(|session| session.created_at);
        Ok(result.into_iter().next())
    }

    /// Find a single session matching ALL given tags + owner within an org.
    pub async fn find_session_by_tags_and_owner(
        &self,
        org_id: i64,
        owner_principal_id: PrincipalId,
        tags: &[String],
    ) -> Result<Option<SessionRow>> {
        let sessions = self.sessions.read();
        let mut result: Vec<_> = sessions
            .values()
            .filter(|s| {
                s.org_id == org_id
                    && s.owner_principal_id == owner_principal_id
                    && tags.iter().all(|tag| s.tags.contains(tag))
            })
            .cloned()
            .collect();
        result.sort_by_key(|session| session.created_at);
        Ok(result.into_iter().next())
    }

    /// Find a single app-owned session matching ALL given tags + owner within an org.
    pub async fn find_app_session_by_tags_and_owner(
        &self,
        org_id: i64,
        app_id: Uuid,
        owner_principal_id: PrincipalId,
        tags: &[String],
    ) -> Result<Option<SessionRow>> {
        let sessions = self.sessions.read();
        let mut result: Vec<_> = sessions
            .values()
            .filter(|s| {
                s.org_id == org_id
                    && s.app_id == Some(app_id)
                    && s.owner_principal_id == owner_principal_id
                    && tags.iter().all(|tag| s.tags.contains(tag))
            })
            .cloned()
            .collect();
        result.sort_by_key(|session| session.created_at);
        Ok(result.into_iter().next())
    }

    /// Update session, validating org ownership directly
    /// In-memory twin of the fenced run-summary write (EVE-867). Holds the
    /// write lock across the comparison so the sequence guard is as atomic here
    /// as the Postgres `WHERE` clause is there.
    pub async fn set_session_run_summary(
        &self,
        org_id: i64,
        id: SessionId,
        summary: &str,
        turn_sequence: i64,
    ) -> Result<bool> {
        let mut sessions = self.sessions.write();
        let Some(session) = sessions.get_mut(&id) else {
            return Ok(false);
        };
        if session.org_id != org_id {
            return Ok(false);
        }
        if session
            .run_summary_turn_sequence
            .is_some_and(|stored| stored >= turn_sequence)
        {
            return Ok(false);
        }
        session.run_summary = Some(summary.to_string());
        session.run_summary_turn_sequence = Some(turn_sequence);
        Ok(true)
    }

    pub async fn update_session(
        &self,
        org_id: i64,
        id: SessionId,
        input: UpdateSession,
    ) -> Result<Option<SessionRow>> {
        // First validate org ownership
        {
            let sessions = self.sessions.read();
            if let Some(session) = sessions.get(&id) {
                if session.org_id != org_id {
                    return Ok(None);
                }
            } else {
                return Ok(None);
            }
        }

        let mut sessions = self.sessions.write();
        if let Some(session) = sessions.get_mut(&id) {
            if let Some(harness_id) = input.harness_id {
                session.harness_id = Some(harness_id);
            }
            if let Some(agent_version_id) = input.agent_version_id {
                session.agent_version_id = Some(agent_version_id);
            }
            if let Some(agent_config_hash) = input.agent_config_hash {
                session.agent_config_hash = Some(agent_config_hash);
            }
            if let Some(title) = input.title {
                session.title = Some(title);
            }
            if let Some(goal) = input.goal {
                session.goal = Some(goal);
            }
            input
                .agent_identity_id
                .apply(&mut session.agent_identity_id);
            if let Some(owner_principal_id) = input.owner_principal_id {
                session.owner_principal_id = owner_principal_id;
            }
            input
                .resolved_owner_user_id
                .apply(&mut session.resolved_owner_user_id);
            if let Some(locale) = input.locale {
                session.locale = Some(locale);
            }
            if let Some(tags) = input.tags {
                session.tags = tags;
            }
            if let Some(status) = input.status {
                session.status = status;
            }
            if let Some(started_at) = input.started_at {
                session.started_at = Some(started_at);
            }
            if input.finished_at.is_some() {
                session.finished_at = input.finished_at;
            }
            // Update updated_at on every update (mimics DB trigger)
            session.updated_at = Self::now();
            return Ok(Some(session.clone()));
        }
        Ok(None)
    }

    /// Archive or unarchive a session. Mirrors
    /// `Database::set_session_archived`: idempotent, and archiving keeps the
    /// original `archived_at`.
    pub async fn set_session_archived(
        &self,
        org_id: i64,
        id: SessionId,
        archived: bool,
    ) -> Result<Option<SessionRow>> {
        let mut sessions = self.sessions.write();
        let Some(session) = sessions.get_mut(&id) else {
            return Ok(None);
        };
        if session.org_id != org_id {
            return Ok(None);
        }
        session.archived_at = if archived {
            session.archived_at.or_else(|| Some(Self::now()))
        } else {
            None
        };
        session.updated_at = Self::now();
        Ok(Some(session.clone()))
    }

    /// Delete session, validating org ownership directly
    pub async fn delete_session(&self, org_id: i64, id: SessionId) -> Result<bool> {
        // First validate org ownership
        {
            let sessions = self.sessions.read();
            if let Some(session) = sessions.get(&id) {
                if session.org_id != org_id {
                    return Ok(false);
                }
            } else {
                return Ok(false);
            }
        }

        // Delete events first
        {
            let mut events = self.events.write();
            let to_remove: Vec<EventId> = events
                .iter()
                .filter(|(_, e)| e.session_id == id)
                .map(|(eid, _)| *eid)
                .collect();
            for eid in to_remove {
                events.remove(&eid);
            }
        }
        // Delete session files
        {
            let mut files = self.session_files.write();
            let to_remove: Vec<Uuid> = files
                .iter()
                .filter(|(_, f)| f.session_id == id)
                .map(|(fid, _)| *fid)
                .collect();
            for fid in to_remove {
                files.remove(&fid);
            }
        }
        Ok(self.sessions.write().remove(&id).is_some())
    }

    // ============================================
    // Pinned Sessions
    // ============================================

    pub async fn pin_session(
        &self,
        user_id: Uuid,
        session_id: SessionId,
        org_id: i64,
    ) -> Result<()> {
        let mut pins = self.pinned_sessions.write();
        pins.entry((user_id, session_id))
            .or_insert((org_id, Self::now()));
        Ok(())
    }

    pub async fn unpin_session(
        &self,
        user_id: Uuid,
        session_id: SessionId,
        org_id: i64,
    ) -> Result<bool> {
        let mut pins = self.pinned_sessions.write();
        let key = (user_id, session_id);
        let Some((pinned_org_id, _)) = pins.get(&key) else {
            return Ok(false);
        };
        if *pinned_org_id != org_id {
            return Ok(false);
        }
        Ok(pins.remove(&key).is_some())
    }

    pub async fn list_pinned_session_ids(
        &self,
        user_id: Uuid,
        org_id: i64,
    ) -> Result<Vec<SessionId>> {
        let pins = self.pinned_sessions.read();
        let mut entries: Vec<_> = pins
            .iter()
            .filter(|((uid, _), (oid, _))| *uid == user_id && *oid == org_id)
            .map(|((_, sid), (_, pinned_at))| (*sid, *pinned_at))
            .collect();
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.1)); // Most recently pinned first
        Ok(entries.into_iter().map(|(sid, _)| sid).collect())
    }
}
