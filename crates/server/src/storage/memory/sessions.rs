// In-memory storage: Sessions, Pinned Sessions

use super::super::models::*;
use super::InMemoryDatabase;
use super::matches_search_tokens;
use anyhow::Result;
use chrono::{DateTime, Utc};
use everruns_core::{AgentId, EventId, HarnessId, PrincipalId, SessionId};
use uuid::Uuid;

impl InMemoryDatabase {
    // ============================================
    // Sessions
    // ============================================

    pub async fn create_session(&self, input: CreateSessionRow) -> Result<SessionRow> {
        let now = Self::now();
        let id = SessionId::new();
        let row = SessionRow {
            id,
            org_id: input.org_id,
            harness_id: input.harness_id,
            agent_id: input.agent_id,
            agent_identity_id: input.agent_identity_id,
            owner_principal_id: input.owner_principal_id,
            resolved_owner_user_id: input.resolved_owner_user_id,
            title: input.title,
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
            status: "pending".to_string(), // Default status for new sessions
            created_at: now,
            updated_at: now,
            started_at: None,
            finished_at: None,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            parent_session_id: None,
            subagent_name: None,
            subagent_task: None,
            subagent_status: None,
            blueprint_id: input.blueprint_id,
            blueprint_config: input.blueprint_config,
        };
        self.sessions.write().insert(id, row.clone());
        Ok(row)
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

    pub async fn set_subagent_metadata(
        &self,
        org_id: i64,
        id: SessionId,
        parent_session_id: SessionId,
        subagent_name: &str,
        subagent_task: &str,
        subagent_status: &str,
    ) -> Result<Option<SessionRow>> {
        let mut sessions = self.sessions.write();
        let Some(session) = sessions.get_mut(&id) else {
            return Ok(None);
        };
        if session.org_id != org_id {
            return Ok(None);
        }
        session.parent_session_id = Some(parent_session_id);
        session.subagent_name = Some(subagent_name.to_string());
        session.subagent_task = Some(subagent_task.to_string());
        session.subagent_status = Some(subagent_status.to_string());
        Ok(Some(session.clone()))
    }

    /// Get session without org scoping. For internal system use only (e.g. usage tracking).
    pub async fn get_session_unscoped(&self, id: SessionId) -> Result<Option<SessionRow>> {
        let sessions = self.sessions.read();
        Ok(sessions.get(&id).cloned())
    }

    /// List sessions for an agent with pagination, validating org ownership.
    /// Returns (sessions, total_count).
    pub async fn list_sessions(
        &self,
        org_id: i64,
        agent_id: Option<AgentId>,
        search: Option<&str>,
        pagination: crate::api::common::Pagination,
    ) -> Result<(Vec<SessionRow>, u32)> {
        // If agent_id is provided, validate it belongs to the org
        if let Some(aid) = agent_id {
            let agents = self.agents.read();
            if !agents
                .get(&aid)
                .map(|a| a.org_id == org_id)
                .unwrap_or(false)
            {
                return Ok((vec![], 0));
            }
        }

        let sessions = self.sessions.read();
        let mut result: Vec<_> = sessions
            .values()
            .filter(|s| s.org_id == org_id && agent_id.is_none_or(|aid| s.agent_id == Some(aid)))
            .filter(|s| matches_search_tokens(search, &[s.title.as_deref().unwrap_or("")]))
            .cloned()
            .collect();
        result.sort_by_key(|session| std::cmp::Reverse(session.created_at));

        let total = result.len() as u32;
        let offset = pagination.offset as usize;
        let limit = pagination.limit as usize;

        // Apply pagination
        let paginated = result.into_iter().skip(offset).take(limit).collect();

        Ok((paginated, total))
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

    /// Find active sessions with Slack tags (for startup recovery).
    pub async fn find_active_slack_sessions(&self) -> Result<Vec<SessionRow>> {
        let sessions = self.sessions.read();
        let result: Vec<_> = sessions
            .values()
            .filter(|s| s.status == "active" && s.tags.iter().any(|t| t.starts_with("slack:app:")))
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

    /// Update session, validating org ownership directly
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
            if let Some(title) = input.title {
                session.title = Some(title);
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
