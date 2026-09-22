//! Installations, schedules, leased resources, session tasks, and apps.

use super::*;

impl StorageBackend {
    pub async fn get_user_id_by_installation_id(
        &self,
        provider: &str,
        installation_id: i64,
    ) -> Result<Option<Uuid>> {
        dispatch!(
            self,
            get_user_id_by_installation_id,
            provider,
            installation_id
        )
    }

    // ============================================
    // Agent Identity Connections
    // ============================================

    pub async fn upsert_agent_identity_connection(
        &self,
        input: CreateAgentIdentityConnectionRow,
    ) -> Result<AgentIdentityConnectionRow> {
        dispatch!(self, upsert_agent_identity_connection, input)
    }

    pub async fn upsert_agent_identity_connection_for_active_agent(
        &self,
        org_id: i64,
        agent_id: AgentId,
        input: CreateAgentIdentityConnectionRow,
    ) -> Result<Option<AgentIdentityConnectionRow>> {
        dispatch!(
            self,
            upsert_agent_identity_connection_for_active_agent,
            org_id,
            agent_id,
            input
        )
    }

    pub async fn get_agent_identity_connection(
        &self,
        identity_id: AgentIdentityId,
        provider: &str,
    ) -> Result<Option<AgentIdentityConnectionRow>> {
        dispatch!(self, get_agent_identity_connection, identity_id, provider)
    }

    pub async fn list_agent_identity_connections(
        &self,
        identity_id: AgentIdentityId,
    ) -> Result<Vec<AgentIdentityConnectionRow>> {
        dispatch!(self, list_agent_identity_connections, identity_id)
    }
    pub async fn update_agent_identity_connection_oauth_tokens(
        &self,
        input: UpdateOAuthConnectionTokens,
    ) -> Result<Option<AgentIdentityConnectionRow>> {
        dispatch!(self, update_agent_identity_connection_oauth_tokens, input)
    }

    pub async fn delete_all_agent_identity_connections(
        &self,
        identity_id: AgentIdentityId,
    ) -> Result<u64> {
        dispatch!(self, delete_all_agent_identity_connections, identity_id)
    }

    pub async fn delete_agent_identity_connection(
        &self,
        identity_id: AgentIdentityId,
        provider: &str,
    ) -> Result<bool> {
        dispatch!(
            self,
            delete_agent_identity_connection,
            identity_id,
            provider
        )
    }

    pub async fn invalidate_mcp_service_connection_if_access_token_matches(
        &self,
        identity_id: AgentIdentityId,
        provider: &str,
        expected_access_token_encrypted: &[u8],
        org_id: i64,
        mcp_server_id: Uuid,
        agent_id: Uuid,
    ) -> Result<bool> {
        dispatch!(
            self,
            invalidate_mcp_service_connection_if_access_token_matches,
            identity_id,
            provider,
            expected_access_token_encrypted,
            org_id,
            mcp_server_id,
            agent_id
        )
    }

    // ============================================
    // Session Schedules
    // ============================================

    pub async fn create_session_schedule(
        &self,
        input: CreateSessionScheduleRow,
    ) -> Result<SessionScheduleRow> {
        dispatch!(self, create_session_schedule, input)
    }

    pub async fn get_session_schedule(
        &self,
        org_id: i64,
        schedule_id: ScheduleId,
    ) -> Result<Option<SessionScheduleRow>> {
        dispatch!(self, get_session_schedule, org_id, schedule_id)
    }

    pub async fn list_session_schedules(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Result<Vec<SessionScheduleRow>> {
        dispatch!(self, list_session_schedules, org_id, session_id)
    }

    pub async fn update_session_schedule(
        &self,
        org_id: i64,
        schedule_id: ScheduleId,
        input: UpdateSessionScheduleRow,
    ) -> Result<Option<SessionScheduleRow>> {
        dispatch!(self, update_session_schedule, org_id, schedule_id, input)
    }

    pub async fn delete_session_schedule(
        &self,
        org_id: i64,
        schedule_id: ScheduleId,
    ) -> Result<bool> {
        dispatch!(self, delete_session_schedule, org_id, schedule_id)
    }

    pub async fn create_session_schedule_with_limits(
        &self,
        input: CreateSessionScheduleRow,
        max_per_session: u32,
        max_per_org: i64,
    ) -> Result<Option<SessionScheduleRow>> {
        dispatch!(
            self,
            create_session_schedule_with_limits,
            input,
            max_per_session,
            max_per_org
        )
    }

    pub async fn count_active_session_schedules(&self, session_id: SessionId) -> Result<u32> {
        dispatch!(self, count_active_session_schedules, session_id)
    }

    pub async fn count_active_org_session_schedules(&self, org_id: i64) -> Result<u32> {
        dispatch!(self, count_active_org_session_schedules, org_id)
    }

    pub async fn claim_due_session_schedules(
        &self,
        scheduler_id: &str,
        limit: i32,
    ) -> Result<Vec<SessionScheduleRow>> {
        dispatch!(self, claim_due_session_schedules, scheduler_id, limit)
    }

    // ============================================
    // Leased Resources
    // ============================================

    pub async fn get_session_organization_id(&self, session_id: SessionId) -> Result<Option<i64>> {
        dispatch!(self, get_session_organization_id, session_id)
    }

    // ============================================
    // Cross-Org Resource Resolution
    //
    // Lookup-by-public_id helpers that return the owning org without requiring
    // the caller to know it. Used only by the authenticated
    // GET /v1/resolve-org endpoint, which gates the result by the caller's
    // org memberships to preserve the 404-vs-403 enumeration guarantee.
    // See knowledge/security/multitenancy.md (Cross-Org Resource Resolution).
    // ============================================

    pub async fn get_agent_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        dispatch!(self, get_agent_organization_id, public_id)
    }

    pub async fn get_harness_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        dispatch!(self, get_harness_organization_id, public_id)
    }

    pub async fn get_app_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        dispatch!(self, get_app_organization_id, public_id)
    }

    pub async fn get_skill_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        dispatch!(self, get_skill_organization_id, public_id)
    }

    pub async fn get_mcp_server_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        dispatch!(self, get_mcp_server_organization_id, public_id)
    }

    pub async fn get_agent_identity_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        dispatch!(self, get_agent_identity_organization_id, public_id)
    }

    pub async fn get_eval_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        dispatch!(self, get_eval_organization_id, public_id)
    }

    pub async fn get_memory_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        dispatch!(self, get_memory_organization_id, public_id)
    }

    pub async fn upsert_leased_resource(
        &self,
        input: UpsertLeasedResourceRow,
    ) -> Result<LeasedResourceRow> {
        dispatch!(self, upsert_leased_resource, input)
    }

    pub async fn release_leased_resource(
        &self,
        input: ReleaseLeasedResourceRow,
    ) -> Result<Option<LeasedResourceRow>> {
        dispatch!(self, release_leased_resource, input)
    }

    pub async fn list_session_leased_resources(
        &self,
        session_id: SessionId,
    ) -> Result<Vec<LeasedResourceRow>> {
        dispatch!(self, list_session_leased_resources, session_id)
    }

    pub async fn claim_due_leased_resources(
        &self,
        limit: i32,
        stale_after_seconds: i32,
    ) -> Result<Vec<LeasedResourceRow>> {
        dispatch!(self, claim_due_leased_resources, limit, stale_after_seconds)
    }

    pub async fn mark_leased_resource_released(
        &self,
        resource_id: LeasedResourceId,
        expected_cleanup_started_at: DateTime<Utc>,
    ) -> Result<Option<LeasedResourceRow>> {
        dispatch!(
            self,
            mark_leased_resource_released,
            resource_id,
            expected_cleanup_started_at
        )
    }

    pub async fn mark_leased_resource_cleanup_failed(
        &self,
        resource_id: LeasedResourceId,
        expected_cleanup_started_at: DateTime<Utc>,
        retry_after_seconds: i32,
        error: &str,
    ) -> Result<Option<LeasedResourceRow>> {
        dispatch!(
            self,
            mark_leased_resource_cleanup_failed,
            resource_id,
            expected_cleanup_started_at,
            retry_after_seconds,
            error
        )
    }

    // ============================================
    // Session Resource Registry
    // ============================================

    pub async fn upsert_session_resource(
        &self,
        input: UpsertSessionResourceRow,
    ) -> Result<SessionResourceRow> {
        dispatch!(self, upsert_session_resource, input)
    }

    pub async fn update_session_resource_status(
        &self,
        session_id: SessionId,
        resource_id: &str,
        status: &str,
    ) -> Result<Option<SessionResourceRow>> {
        dispatch!(
            self,
            update_session_resource_status,
            session_id,
            resource_id,
            status
        )
    }

    pub async fn get_session_resource(
        &self,
        session_id: SessionId,
        resource_id: &str,
    ) -> Result<Option<SessionResourceRow>> {
        dispatch!(self, get_session_resource, session_id, resource_id)
    }

    pub async fn list_session_resources(
        &self,
        session_id: SessionId,
        kind: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<SessionResourceRow>> {
        dispatch!(self, list_session_resources, session_id, kind, status)
    }

    pub async fn delete_session_resource(
        &self,
        session_id: SessionId,
        resource_id: &str,
    ) -> Result<bool> {
        dispatch!(self, delete_session_resource, session_id, resource_id)
    }

    // ============================================
    // Session Tasks
    // ============================================

    /// Insert a task. Idempotent on `id`; the `bool` is true when inserted.
    pub async fn create_session_task(
        &self,
        task: &everruns_core::SessionTask,
    ) -> Result<(SessionTaskRow, bool)> {
        dispatch!(self, create_session_task, task)
    }

    pub async fn get_session_task(
        &self,
        session_id: SessionId,
        task_id: &str,
    ) -> Result<Option<SessionTaskRow>> {
        dispatch!(self, get_session_task, session_id, task_id)
    }

    pub async fn list_session_tasks(
        &self,
        session_id: SessionId,
        kind: Option<&str>,
        state: Option<&str>,
    ) -> Result<Vec<SessionTaskRow>> {
        dispatch!(self, list_session_tasks, session_id, kind, state)
    }

    /// List tasks across every session owned by `org_id`, newest-first, with
    /// optional kind/state/age filters and a bounded limit. Org scoping is the
    /// authoritative multitenancy boundary (a semijoin on `sessions.org_id`):
    /// a task is only returned when its owning session belongs to the org.
    #[allow(clippy::too_many_arguments)]
    pub async fn list_org_session_tasks(
        &self,
        org_id: i64,
        kind: Option<&str>,
        state: Option<&str>,
        created_after: Option<DateTime<Utc>>,
        root_session_id: Option<SessionId>,
        limit: i64,
    ) -> Result<Vec<SessionTaskRow>> {
        dispatch!(
            self,
            list_org_session_tasks,
            org_id,
            kind,
            state,
            created_after,
            root_session_id,
            limit
        )
    }

    pub async fn update_session_task(
        &self,
        session_id: SessionId,
        task_id: &str,
        update: everruns_core::SessionTaskUpdate,
    ) -> Result<Option<SessionTaskRow>> {
        dispatch!(self, update_session_task, session_id, task_id, update)
    }

    pub async fn request_cancel_session_task(
        &self,
        session_id: SessionId,
        task_id: &str,
    ) -> Result<Option<(SessionTaskRow, bool)>> {
        dispatch!(self, request_cancel_session_task, session_id, task_id)
    }

    pub async fn insert_session_task_message(
        &self,
        input: NewSessionTaskMessageRow,
    ) -> Result<SessionTaskMessageRow> {
        dispatch!(self, insert_session_task_message, input)
    }

    // Per-task push-notification configs (EVE-682).

    pub async fn create_task_push_config(
        &self,
        input: crate::storage::models::CreateSessionTaskPushConfig,
    ) -> Result<crate::storage::models::SessionTaskPushConfigRow> {
        dispatch!(self, create_task_push_config, input)
    }

    pub async fn list_task_push_configs(
        &self,
        session_id: SessionId,
        task_id: &str,
    ) -> Result<Vec<crate::storage::models::SessionTaskPushConfigRow>> {
        dispatch!(self, list_task_push_configs, session_id, task_id)
    }

    pub async fn delete_task_push_config(
        &self,
        session_id: SessionId,
        task_id: &str,
        public_id: &str,
    ) -> Result<bool> {
        dispatch!(
            self,
            delete_task_push_config,
            session_id,
            task_id,
            public_id
        )
    }

    pub async fn list_session_task_messages(
        &self,
        session_id: SessionId,
        task_id: &str,
        limit: Option<u32>,
        after_id: Option<&str>,
    ) -> Result<Vec<SessionTaskMessageRow>> {
        dispatch!(
            self,
            list_session_task_messages,
            session_id,
            task_id,
            limit,
            after_id
        )
    }

    /// Return (session_id, task_id, schedule_id) triples for running monitor
    /// tasks whose linked schedule is inactive (missing or enabled=false).
    pub async fn list_monitor_tasks_with_inactive_schedules(
        &self,
        limit: i64,
    ) -> Result<Vec<(SessionId, String, String)>> {
        dispatch!(self, list_monitor_tasks_with_inactive_schedules, limit)
    }

    /// Return (session_id, task_id) pairs for tasks with a stale heartbeat.
    /// See individual backend impls for locking semantics.
    pub async fn list_orphaned_session_task_ids(
        &self,
        stale_after: chrono::Duration,
        limit: i64,
    ) -> Result<Vec<(SessionId, String)>> {
        dispatch!(self, list_orphaned_session_task_ids, stale_after, limit)
    }

    /// Prune a bounded batch of terminal session tasks older than `cutoff`,
    /// returning the `(session_id, task_id, result_path)` triples removed so
    /// the caller can delete their artifacts (EVE-580). Messages are deleted
    /// in both backends (PG via FK cascade, in-memory explicitly).
    pub async fn prune_terminal_session_tasks(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> Result<Vec<(SessionId, String, Option<String>)>> {
        dispatch!(self, prune_terminal_session_tasks, cutoff, limit)
    }

    /// Full retention prune (EVE-580): delete a bounded batch of terminal
    /// session tasks older than `now - ttl` (rows + messages), then remove
    /// each pruned task's recorded internal artifact subtree through the
    /// existing session-file deletion seam (which clears backing blobs for the
    /// object-storage backend). Returns the number of tasks pruned.
    ///
    /// Ordering: rows commit first, artifacts after, so a crash leaks at worst
    /// a dangling blob (reclaimed by blob GC) rather than a row pointing at a
    /// deleted artifact. Artifact deletion is best-effort and never fails the
    /// prune. Shared by the in-process Direct worker adapter and the gRPC
    /// `PruneTerminalSessionTasks` server handler.
    pub async fn prune_terminal_session_tasks_with_artifacts(
        &self,
        ttl: chrono::Duration,
        limit: i64,
    ) -> Result<usize> {
        // Defensive bound on a destructive query. Postgres treats `LIMIT <= 0`
        // (a negative value) as unbounded (`LIMIT ALL`), so a misconfigured or
        // legacy caller passing `limit <= 0` could turn this bounded retention
        // pass into an unlimited delete. Clamp to a positive, capped batch here
        // — the single chokepoint every caller (Direct adapter + gRPC handler)
        // funnels through — regardless of the caller's input. EVE-580 review.
        let limit = limit.clamp(1, MAX_RETENTION_PRUNE_LIMIT);
        let cutoff = chrono::Utc::now() - ttl;
        let pruned = self.prune_terminal_session_tasks(cutoff, limit).await?;

        for (session_id, task_id, result_path) in &pruned {
            let Some(result_path) = result_path.as_deref() else {
                continue;
            };
            let Some(dir) = task_artifact_delete_root(result_path) else {
                tracing::warn!(
                    session_id = %session_id,
                    task_id = %task_id,
                    result_path = %result_path,
                    "Retention prune: skipped non-task artifact result path"
                );
                continue;
            };
            if let Err(e) = self
                .delete_session_file_recursive(session_id.uuid(), dir)
                .await
            {
                tracing::warn!(
                    session_id = %session_id,
                    task_id = %task_id,
                    result_path = %result_path,
                    error = %e,
                    "Retention prune: failed to delete task artifacts (best-effort; blob GC will reclaim)"
                );
            }
        }

        Ok(pruned.len())
    }

    // ============================================
    // Audit Logs (TM-OBS-007, EVE-226)
    // ============================================

    pub async fn create_audit_log(&self, input: CreateAuditLogRow) -> Result<AuditLogRow> {
        dispatch!(self, create_audit_log, input)
    }

    pub async fn list_audit_logs(&self, query: AuditLogQuery<'_>) -> Result<Vec<AuditLogRow>> {
        dispatch!(self, list_audit_logs, query)
    }

    pub async fn delete_audit_logs_before(&self, before: DateTime<Utc>) -> Result<u64> {
        dispatch!(self, delete_audit_logs_before, before)
    }

    // ============================================
    // App CRUD
    // ============================================

    pub async fn create_app(&self, org_id: i64, input: CreateAppRow) -> Result<AppRow> {
        dispatch!(self, create_app, org_id, input)
    }

    pub async fn get_app_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<AppRow>> {
        dispatch!(self, get_app_by_public_id, org_id, public_id)
    }

    pub async fn get_app_by_id(&self, org_id: i64, id: Uuid) -> Result<Option<AppRow>> {
        dispatch!(self, get_app_by_id, org_id, id)
    }

    /// Lookup app by public_id without org scoping (for unauthenticated webhooks).
    pub async fn get_app_by_public_id_unscoped(&self, public_id: &str) -> Result<Option<AppRow>> {
        dispatch!(self, get_app_by_public_id_unscoped, public_id)
    }

    /// Lookup an app through a globally unique channel public ID.
    pub async fn get_app_by_channel_public_id_unscoped(
        &self,
        channel_public_id: &str,
    ) -> Result<Option<AppRow>> {
        dispatch!(
            self,
            get_app_by_channel_public_id_unscoped,
            channel_public_id
        )
    }

    pub async fn list_apps(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<AppRow>> {
        dispatch!(self, list_apps, org_id, search, include_archived)
    }

    pub async fn count_apps_for_agent(&self, org_id: i64, agent_id: AgentId) -> Result<u64> {
        dispatch!(self, count_apps_for_agent, org_id, agent_id)
    }

    pub async fn count_apps_for_harness(&self, org_id: i64, harness_id: HarnessId) -> Result<u64> {
        dispatch!(self, count_apps_for_harness, org_id, harness_id)
    }

    pub async fn count_apps_for_harnesses(
        &self,
        org_id: i64,
        harness_ids: &[HarnessId],
    ) -> Result<Vec<(HarnessId, i64)>> {
        dispatch!(self, count_apps_for_harnesses, org_id, harness_ids)
    }

    pub async fn update_app(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateApp,
    ) -> Result<Option<AppRow>> {
        dispatch!(self, update_app, org_id, id, input)
    }

    pub async fn delete_app(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_app, org_id, id)
    }

    pub async fn destroy_app(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, destroy_app, org_id, id)
    }

    // ============================================
    // App Channel CRUD
    // ============================================

    pub async fn create_app_channel(
        &self,
        app_id: Uuid,
        input: CreateAppChannelRow,
    ) -> Result<AppChannelRow> {
        dispatch!(self, create_app_channel, app_id, input)
    }

    pub async fn create_app_channel_enforcing_schedule_cap(
        &self,
        org_id: i64,
        app_id: Uuid,
        input: CreateAppChannelRow,
        max_enabled_schedule_channels: i64,
    ) -> Result<AppChannelRow> {
        dispatch!(
            self,
            create_app_channel_enforcing_schedule_cap,
            org_id,
            app_id,
            input,
            max_enabled_schedule_channels
        )
    }

    pub async fn list_app_channels(&self, app_id: Uuid) -> Result<Vec<AppChannelRow>> {
        dispatch!(self, list_app_channels, app_id)
    }

    pub async fn set_app_endpoint_publish(&self, app_id: Uuid, published: bool) -> Result<u64> {
        dispatch!(self, set_app_endpoint_publish, app_id, published)
    }

    pub async fn agents_with_live_endpoints(
        &self,
        agent_ids: &[Uuid],
    ) -> Result<std::collections::HashSet<Uuid>> {
        dispatch!(self, agents_with_live_endpoints, agent_ids)
    }

    pub async fn app_has_channels(&self, app_id: Uuid) -> Result<bool> {
        dispatch!(self, app_has_channels, app_id)
    }

    pub async fn get_app_channel_by_public_id(
        &self,
        public_id: &str,
    ) -> Result<Option<AppChannelRow>> {
        dispatch!(self, get_app_channel_by_public_id, public_id)
    }

    pub async fn get_ingress_endpoint_by_public_id(
        &self,
        public_id: &str,
    ) -> Result<Option<IngressEndpointRow>> {
        dispatch!(self, get_ingress_endpoint_by_public_id, public_id)
    }

    pub async fn list_agent_endpoints(
        &self,
        org_id: i64,
        agent_id: Uuid,
    ) -> Result<Vec<IngressEndpointRow>> {
        dispatch!(self, list_agent_endpoints, org_id, agent_id)
    }

    pub async fn get_agent_endpoint(
        &self,
        org_id: i64,
        agent_id: Uuid,
        public_id: &str,
    ) -> Result<Option<IngressEndpointRow>> {
        dispatch!(self, get_agent_endpoint, org_id, agent_id, public_id)
    }

    pub async fn create_agent_endpoint(
        &self,
        org_id: i64,
        input: CreateAgentEndpointRow,
    ) -> Result<IngressEndpointRow> {
        dispatch!(self, create_agent_endpoint, org_id, input)
    }

    pub async fn update_agent_endpoint(
        &self,
        org_id: i64,
        agent_id: Uuid,
        public_id: &str,
        input: UpdateAgentEndpointRow,
    ) -> Result<Option<IngressEndpointRow>> {
        dispatch!(
            self,
            update_agent_endpoint,
            org_id,
            agent_id,
            public_id,
            input
        )
    }

    pub async fn delete_agent_endpoint(
        &self,
        org_id: i64,
        agent_id: Uuid,
        public_id: &str,
    ) -> Result<bool> {
        dispatch!(self, delete_agent_endpoint, org_id, agent_id, public_id)
    }

    pub async fn list_ingress_endpoints_by_legacy_alias(
        &self,
        legacy_app_public_id: &str,
        channel_type: &str,
    ) -> Result<Vec<IngressEndpointRow>> {
        dispatch!(
            self,
            list_ingress_endpoints_by_legacy_alias,
            legacy_app_public_id,
            channel_type
        )
    }

    pub async fn get_agent_endpoint_public_id(
        &self,
        org_id: i64,
        endpoint_id: Uuid,
    ) -> Result<Option<String>> {
        dispatch!(self, get_agent_endpoint_public_id, org_id, endpoint_id)
    }

    pub async fn update_app_channel(
        &self,
        id: Uuid,
        input: UpdateAppChannel,
    ) -> Result<Option<AppChannelRow>> {
        dispatch!(self, update_app_channel, id, input)
    }

    pub async fn update_app_channel_enforcing_schedule_cap(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateAppChannel,
        max_enabled_schedule_channels: i64,
    ) -> Result<Option<AppChannelRow>> {
        dispatch!(
            self,
            update_app_channel_enforcing_schedule_cap,
            org_id,
            id,
            input,
            max_enabled_schedule_channels
        )
    }

    pub async fn delete_app_channel(&self, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_app_channel, id)
    }

    pub async fn count_enabled_schedule_channels_for_org(&self, org_id: i64) -> Result<i64> {
        dispatch!(self, count_enabled_schedule_channels_for_org, org_id)
    }

    // ============================================
    // Observers (online scoring — knowledge/evaluation/online-evals.md)
    // ============================================

    pub async fn create_observer(
        &self,
        org_id: i64,
        input: CreateObserverRow,
    ) -> Result<ObserverRow> {
        dispatch!(self, create_observer, org_id, input)
    }
}
