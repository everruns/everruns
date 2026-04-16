// In-memory storage: Sessions, Pinned Sessions

use super::super::models::*;
use super::InMemoryDatabase;
use super::matches_search_tokens;
use anyhow::Result;
use chrono::{DateTime, Utc};
use everruns_core::{AgentId, EventId, SessionId};
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
            title: input.title,
            locale: input.locale,
            tags: input.tags,
            model_id: input.model_id,
            capabilities: input.capabilities,
            tools: input.tools,
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
            if let Some(title) = input.title {
                session.title = Some(title);
            }
            input
                .agent_identity_id
                .apply(&mut session.agent_identity_id);
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

    pub async fn unpin_session(&self, user_id: Uuid, session_id: SessionId) -> Result<bool> {
        let mut pins = self.pinned_sessions.write();
        Ok(pins.remove(&(user_id, session_id)).is_some())
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
