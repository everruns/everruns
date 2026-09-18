//! Harnesses, agent triggers, sessions, workspaces, and memory files.

use super::*;

impl StorageBackend {
    pub async fn create_harness(&self, org_id: i64, input: CreateHarnessRow) -> Result<HarnessRow> {
        dispatch!(self, create_harness, org_id, input)
    }

    pub async fn create_harness_with_id(
        &self,
        org_id: i64,
        id: HarnessId,
        input: CreateHarnessRow,
    ) -> Result<Option<HarnessRow>> {
        dispatch!(self, create_harness_with_id, org_id, id, input)
    }

    pub async fn get_harness(&self, org_id: i64, id: HarnessId) -> Result<Option<HarnessRow>> {
        #[cfg(test)]
        self.fail_if_forced("get_harness")?;
        #[cfg(test)]
        self.record_session_list_lookup().await;
        dispatch!(self, get_harness, org_id, id)
    }

    pub async fn get_harness_ancestry_by_ids(
        &self,
        org_id: i64,
        ids: &[HarnessId],
    ) -> Result<Vec<HarnessRow>> {
        #[cfg(test)]
        self.record_session_list_lookup().await;
        dispatch!(self, get_harness_ancestry_by_ids, org_id, ids)
    }

    pub async fn get_harness_by_name(&self, org_id: i64, name: &str) -> Result<Option<HarnessRow>> {
        dispatch!(self, get_harness_by_name, org_id, name)
    }

    pub async fn list_harnesses(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<HarnessRow>> {
        dispatch!(self, list_harnesses, org_id, search, include_archived)
    }

    pub async fn count_sessions_for_harness(
        &self,
        org_id: i64,
        harness_id: HarnessId,
    ) -> Result<u64> {
        dispatch!(self, count_sessions_for_harness, org_id, harness_id)
    }

    pub async fn count_sessions_for_harnesses(
        &self,
        org_id: i64,
        harness_ids: &[HarnessId],
    ) -> Result<Vec<(HarnessId, i64)>> {
        dispatch!(self, count_sessions_for_harnesses, org_id, harness_ids)
    }

    /// Count user-created harnesses in an org (for resource limits); excludes
    /// soft-deleted rows and system-seeded built-in harnesses.
    pub async fn count_harnesses_for_org(&self, org_id: i64) -> Result<i64> {
        dispatch!(self, count_harnesses_for_org, org_id)
    }

    /// Count non-deleted, non-built-in agents in an org (for resource limits).
    pub async fn count_agents_for_org(&self, org_id: i64) -> Result<i64> {
        dispatch!(self, count_agents_for_org, org_id)
    }

    /// Flag an agent as platform-supplied.
    ///
    /// Reserved for org bootstrap reconciliation, which must be able to adopt a
    /// row that already exists — an org seeded before built-in agents shipped
    /// has the agent but not the flag. No command path calls this.
    pub async fn mark_agent_built_in(&self, org_id: i64, id: AgentId) -> Result<()> {
        dispatch!(self, mark_agent_built_in, org_id, id)
    }

    /// Count sessions in an org (for resource limits).
    pub async fn count_sessions_for_org(&self, org_id: i64) -> Result<i64> {
        dispatch!(self, count_sessions_for_org, org_id)
    }

    pub async fn update_harness(
        &self,
        org_id: i64,
        id: HarnessId,
        input: UpdateHarness,
    ) -> Result<Option<HarnessRow>> {
        dispatch!(self, update_harness, org_id, id, input)
    }

    pub async fn list_child_harnesses(
        &self,
        org_id: i64,
        parent_id: HarnessId,
    ) -> Result<Vec<HarnessRow>> {
        dispatch!(self, list_child_harnesses, org_id, parent_id)
    }

    pub async fn release_built_in_harness(&self, org_id: i64, name: &str) -> Result<bool> {
        dispatch!(self, release_built_in_harness, org_id, name)
    }

    pub async fn delete_harness(&self, org_id: i64, id: HarnessId) -> Result<bool> {
        dispatch!(self, delete_harness, org_id, id)
    }

    pub async fn destroy_harness(&self, org_id: i64, id: HarnessId) -> Result<bool> {
        dispatch!(self, destroy_harness, org_id, id)
    }

    // ============================================
    // Agent identities
    // ============================================

    pub async fn create_agent_identity(
        &self,
        input: CreateAgentIdentityRow,
    ) -> Result<AgentIdentityRow> {
        dispatch!(self, create_agent_identity, input)
    }

    pub async fn get_agent_identity(
        &self,
        org_id: i64,
        id: AgentIdentityId,
    ) -> Result<Option<AgentIdentityRow>> {
        dispatch!(self, get_agent_identity, org_id, id)
    }

    pub async fn list_agent_identities(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<AgentIdentityRow>> {
        dispatch!(
            self,
            list_agent_identities,
            org_id,
            search,
            include_archived
        )
    }

    pub async fn update_agent_identity(
        &self,
        org_id: i64,
        id: AgentIdentityId,
        input: UpdateAgentIdentity,
    ) -> Result<Option<AgentIdentityRow>> {
        dispatch!(self, update_agent_identity, org_id, id, input)
    }

    pub async fn delete_agent_identity(&self, org_id: i64, id: AgentIdentityId) -> Result<bool> {
        dispatch!(self, delete_agent_identity, org_id, id)
    }

    pub async fn destroy_agent_identity(&self, org_id: i64, id: AgentIdentityId) -> Result<bool> {
        dispatch!(self, destroy_agent_identity, org_id, id)
    }

    // ============================================
    // Agent triggers
    // ============================================

    pub async fn create_agent_trigger(
        &self,
        input: CreateAgentTriggerRow,
    ) -> Result<AgentTriggerRow> {
        dispatch!(self, create_agent_trigger, input)
    }

    pub async fn get_agent_trigger(
        &self,
        org_id: i64,
        id: TriggerId,
    ) -> Result<Option<AgentTriggerRow>> {
        dispatch!(self, get_agent_trigger, org_id, id)
    }

    pub async fn list_agent_triggers(
        &self,
        org_id: i64,
        agent_id: Option<AgentId>,
        include_archived: bool,
    ) -> Result<Vec<AgentTriggerRow>> {
        dispatch!(
            self,
            list_agent_triggers,
            org_id,
            agent_id,
            include_archived
        )
    }

    pub async fn update_agent_trigger(
        &self,
        org_id: i64,
        id: TriggerId,
        input: UpdateAgentTrigger,
    ) -> Result<Option<AgentTriggerRow>> {
        dispatch!(self, update_agent_trigger, org_id, id, input)
    }

    pub async fn set_agent_trigger_durable_schedule_id(
        &self,
        org_id: i64,
        id: TriggerId,
        durable_schedule_id: Option<Uuid>,
    ) -> Result<Option<AgentTriggerRow>> {
        dispatch!(
            self,
            set_agent_trigger_durable_schedule_id,
            org_id,
            id,
            durable_schedule_id
        )
    }

    pub async fn delete_agent_trigger(&self, org_id: i64, id: TriggerId) -> Result<bool> {
        dispatch!(self, delete_agent_trigger, org_id, id)
    }

    // ============================================
    // Sessions
    // ============================================

    pub async fn create_session(&self, input: CreateSessionRow) -> Result<SessionRow> {
        dispatch!(self, create_session, input)
    }

    pub async fn create_session_participant(
        &self,
        input: CreateSessionParticipantRow,
    ) -> Result<SessionParticipantRow> {
        dispatch!(self, create_session_participant, input)
    }

    pub async fn ensure_active_user_session_participant(
        &self,
        input: CreateSessionParticipantRow,
    ) -> Result<SessionParticipantRow> {
        dispatch!(self, ensure_active_user_session_participant, input)
    }

    pub async fn list_session_participants(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Result<Vec<SessionParticipantRow>> {
        dispatch!(self, list_session_participants, org_id, session_id)
    }

    pub async fn leave_session_participant(
        &self,
        org_id: i64,
        session_id: SessionId,
        participant_id: SessionParticipantId,
    ) -> Result<Option<SessionParticipantRow>> {
        dispatch!(
            self,
            leave_session_participant,
            org_id,
            session_id,
            participant_id
        )
    }

    pub async fn list_reporting_outbox(
        &self,
        org_id: i64,
        source_type: &str,
        source_id: &str,
        reason: &str,
    ) -> Result<Vec<ReportingOutboxRow>> {
        dispatch!(
            self,
            list_reporting_outbox,
            org_id,
            source_type,
            source_id,
            reason
        )
    }

    /// Record fork provenance on an already-created session
    /// (knowledge/runtime-resources/forking-sessions.md).
    pub async fn set_session_fork_lineage(
        &self,
        session_id: SessionId,
        forked_from_session_id: SessionId,
        forked_from_sequence: Option<i32>,
    ) -> Result<()> {
        dispatch!(
            self,
            set_session_fork_lineage,
            session_id,
            forked_from_session_id,
            forked_from_sequence
        )
    }

    pub async fn get_session(&self, org_id: i64, id: SessionId) -> Result<Option<SessionRow>> {
        #[cfg(test)]
        self.fail_if_forced("get_session")?;
        dispatch!(self, get_session, org_id, id)
    }

    /// Get session without org scoping. For internal system use only (e.g. usage tracking).
    pub async fn get_session_unscoped(&self, id: SessionId) -> Result<Option<SessionRow>> {
        dispatch!(self, get_session_unscoped, id)
    }

    /// List sessions for an agent with pagination, validating org ownership.
    /// Returns (sessions, total_count).
    pub async fn list_sessions(
        &self,
        org_id: i64,
        filters: &SessionListFilters,
        pagination: Pagination,
    ) -> Result<(Vec<SessionRow>, u32)> {
        #[cfg(test)]
        self.record_session_list_lookup().await;
        dispatch!(self, list_sessions, org_id, filters, pagination)
    }

    /// Facet-rail counts and masthead metrics over the same predicate as
    /// [`Self::list_sessions`] (EVE-852).
    pub async fn session_facets(
        &self,
        org_id: i64,
        filters: &SessionListFilters,
    ) -> Result<SessionFacetsRow> {
        dispatch!(self, session_facets, org_id, filters)
    }

    /// List child sessions (subagents) for a parent session.
    pub async fn list_child_sessions(
        &self,
        parent_session_id: SessionId,
    ) -> Result<Vec<SessionRow>> {
        dispatch!(self, list_child_sessions, parent_session_id)
    }

    /// Count sessions grouped by status for an organization.
    pub async fn count_sessions_by_status(&self, org_id: i64) -> Result<Vec<(String, i64)>> {
        dispatch!(self, count_sessions_by_status, org_id)
    }

    pub async fn count_active_sessions_for_org(&self, org_id: i64) -> Result<i64> {
        dispatch!(self, count_active_sessions_for_org, org_id)
    }

    pub async fn count_active_turns_for_org(&self, org_id: i64) -> Result<i64> {
        dispatch!(self, count_active_turns_for_org, org_id)
    }

    pub async fn reserve_active_turn_slot_for_org(
        &self,
        org_id: i64,
        session_id: SessionId,
        max_active_turns: i64,
    ) -> Result<ReserveActiveTurnSlotResult> {
        dispatch!(
            self,
            reserve_active_turn_slot_for_org,
            org_id,
            session_id,
            max_active_turns
        )
    }

    /// Release a previously reserved active-turn slot, restoring the session's
    /// status captured at reservation time (best-effort; only reverts a session
    /// still `active`).
    pub async fn release_active_turn_slot_for_org(
        &self,
        org_id: i64,
        session_id: SessionId,
        previous_status: &str,
    ) -> Result<()> {
        dispatch!(
            self,
            release_active_turn_slot_for_org,
            org_id,
            session_id,
            previous_status
        )
    }

    /// Aggregate session and execution stats for an optional agent or harness scope.
    pub async fn session_aggregate_stats(
        &self,
        org_id: i64,
        agent_id: Option<AgentId>,
        harness_id: Option<HarnessId>,
    ) -> Result<SessionAggregateStatsRow> {
        dispatch!(self, session_aggregate_stats, org_id, agent_id, harness_id)
    }

    /// Find active sessions with Slack tags (for startup recovery).
    pub async fn find_active_slack_sessions(&self) -> Result<Vec<SessionRow>> {
        dispatch!(self, find_active_slack_sessions)
    }

    /// Find sessions in `waiting_for_tool_results` with updated_at before the
    /// given cutoff. Returns `(session_id, org_id)` pairs.
    pub async fn list_sessions_waiting_tool_results_before(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<(SessionId, i64)>> {
        dispatch!(self, list_sessions_waiting_tool_results_before, cutoff)
    }

    /// Find a single session matching ALL given tags within an org.
    pub async fn find_session_by_tags(
        &self,
        org_id: i64,
        tags: &[String],
    ) -> Result<Option<SessionRow>> {
        dispatch!(self, find_session_by_tags, org_id, tags)
    }

    /// Find a single app-owned session matching ALL given tags within an org.
    pub async fn find_app_session_by_tags(
        &self,
        org_id: i64,
        app_id: Uuid,
        tags: &[String],
    ) -> Result<Option<SessionRow>> {
        dispatch!(self, find_app_session_by_tags, org_id, app_id, tags)
    }

    /// Find a single session matching ALL given tags + owner within an org.
    pub async fn find_session_by_tags_and_owner(
        &self,
        org_id: i64,
        owner_principal_id: PrincipalId,
        tags: &[String],
    ) -> Result<Option<SessionRow>> {
        dispatch!(
            self,
            find_session_by_tags_and_owner,
            org_id,
            owner_principal_id,
            tags
        )
    }

    /// Find a single app-owned session matching ALL given tags + owner within an org.
    pub async fn find_app_session_by_tags_and_owner(
        &self,
        org_id: i64,
        app_id: Uuid,
        owner_principal_id: PrincipalId,
        tags: &[String],
    ) -> Result<Option<SessionRow>> {
        dispatch!(
            self,
            find_app_session_by_tags_and_owner,
            org_id,
            app_id,
            owner_principal_id,
            tags
        )
    }

    pub async fn update_session(
        &self,
        org_id: i64,
        id: SessionId,
        input: UpdateSession,
    ) -> Result<Option<SessionRow>> {
        dispatch!(self, update_session, org_id, id, input)
    }

    /// Store a generated run summary, fenced on the terminal turn it describes
    /// so a late out-of-band write cannot overwrite a newer one (EVE-867).
    pub async fn set_session_run_summary(
        &self,
        org_id: i64,
        id: SessionId,
        summary: &str,
        turn_sequence: i64,
    ) -> Result<bool> {
        dispatch!(
            self,
            set_session_run_summary,
            org_id,
            id,
            summary,
            turn_sequence
        )
    }

    pub async fn set_session_archived(
        &self,
        org_id: i64,
        id: SessionId,
        archived: bool,
    ) -> Result<Option<SessionRow>> {
        dispatch!(self, set_session_archived, org_id, id, archived)
    }

    pub async fn delete_session(&self, org_id: i64, id: SessionId) -> Result<bool> {
        dispatch!(self, delete_session, org_id, id)
    }

    // ============================================
    // Workspaces (see knowledge/runtime-resources/workspace.md)
    // ============================================

    pub async fn create_workspace(
        &self,
        org_id: i64,
        input: CreateWorkspaceRow,
    ) -> Result<WorkspaceRow> {
        dispatch!(self, create_workspace, org_id, input)
    }

    pub async fn get_workspace(
        &self,
        org_id: i64,
        workspace_id: WorkspaceId,
    ) -> Result<Option<WorkspaceRow>> {
        dispatch!(
            self,
            get_workspace_by_public_id,
            org_id,
            &workspace_id.to_string()
        )
    }

    pub async fn get_workspace_by_id(&self, org_id: i64, id: Uuid) -> Result<Option<WorkspaceRow>> {
        dispatch!(self, get_workspace_by_id, org_id, id)
    }

    pub async fn get_workspace_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        dispatch!(self, get_workspace_organization_id, public_id)
    }

    pub async fn list_workspaces(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<WorkspaceRow>> {
        dispatch!(self, list_workspaces, org_id, search, include_archived)
    }

    pub async fn update_workspace(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateWorkspace,
    ) -> Result<Option<WorkspaceRow>> {
        dispatch!(self, update_workspace, org_id, id, input)
    }

    pub async fn archive_workspace(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, archive_workspace, org_id, id)
    }

    // ============================================
    // Memories
    // ============================================

    pub async fn create_memory(&self, org_id: i64, input: CreateMemoryRow) -> Result<MemoryRow> {
        dispatch!(self, create_memory, org_id, input)
    }

    pub async fn get_memory(&self, org_id: i64, memory_id: MemoryId) -> Result<Option<MemoryRow>> {
        dispatch!(
            self,
            get_memory_by_public_id,
            org_id,
            &memory_id.to_string()
        )
    }

    pub async fn get_memory_by_id(&self, org_id: i64, id: Uuid) -> Result<Option<MemoryRow>> {
        dispatch!(self, get_memory_by_id, org_id, id)
    }

    pub async fn get_memory_by_scope_owner(
        &self,
        org_id: i64,
        scope: &str,
        owner_agent_id: Option<AgentId>,
        owner_user_id: Option<Uuid>,
    ) -> Result<Option<MemoryRow>> {
        dispatch!(
            self,
            get_memory_by_scope_owner,
            org_id,
            scope,
            owner_agent_id,
            owner_user_id
        )
    }

    pub async fn list_memories(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<MemoryRow>> {
        dispatch!(self, list_memories, org_id, search, include_archived)
    }

    pub async fn update_memory(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateMemory,
    ) -> Result<Option<MemoryRow>> {
        dispatch!(self, update_memory, org_id, id, input)
    }

    pub async fn archive_memory(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, archive_memory, org_id, id)
    }

    pub async fn claim_next_memory_sync(&self) -> Result<Option<MemoryRow>> {
        dispatch!(self, claim_next_memory_sync)
    }

    pub async fn complete_memory_sync(
        &self,
        memory_id: Uuid,
        claimed_at: DateTime<Utc>,
        files: Vec<CreateMemoryFileRow>,
    ) -> Result<Option<MemoryRow>> {
        dispatch!(self, complete_memory_sync, memory_id, claimed_at, files)
    }

    pub async fn fail_memory_sync(
        &self,
        memory_id: Uuid,
        claimed_at: DateTime<Utc>,
        error: &str,
    ) -> Result<Option<MemoryRow>> {
        dispatch!(self, fail_memory_sync, memory_id, claimed_at, error)
    }

    pub async fn list_all_memory_files(&self, memory_id: Uuid) -> Result<Vec<MemoryFileRow>> {
        dispatch!(self, list_all_memory_files, memory_id)
    }

    pub async fn create_memory_file(
        &self,
        memory_id: Uuid,
        input: CreateMemoryFileRow,
    ) -> Result<MemoryFileRow> {
        dispatch!(self, create_memory_file, memory_id, input)
    }

    pub async fn get_memory_file(
        &self,
        memory_id: Uuid,
        path: &str,
    ) -> Result<Option<MemoryFileRow>> {
        dispatch!(self, get_memory_file, memory_id, path)
    }

    pub async fn get_memory_file_info(
        &self,
        memory_id: Uuid,
        path: &str,
    ) -> Result<Option<MemoryFileInfoRow>> {
        dispatch!(self, get_memory_file_info, memory_id, path)
    }

    pub async fn list_memory_files(
        &self,
        memory_id: Uuid,
        parent_path: &str,
    ) -> Result<Vec<MemoryFileInfoRow>> {
        dispatch!(self, list_memory_files, memory_id, parent_path)
    }

    pub async fn update_memory_file(
        &self,
        memory_id: Uuid,
        path: &str,
        input: UpdateMemoryFile,
    ) -> Result<Option<MemoryFileRow>> {
        dispatch!(self, update_memory_file, memory_id, path, input)
    }

    pub async fn delete_memory_file(&self, memory_id: Uuid, path: &str) -> Result<bool> {
        dispatch!(self, delete_memory_file, memory_id, path)
    }

    pub async fn delete_memory_file_recursive(&self, memory_id: Uuid, path: &str) -> Result<u64> {
        dispatch!(self, delete_memory_file_recursive, memory_id, path)
    }
}
