use super::queries as q;
use crate::domains::common::*;
use crate::domains::sessions::{SESSION_MANAGE, SESSION_VIEW};
use everruns_core::SessionTask;
use everruns_core::session_task::{
    NewTaskMessage, SessionTaskFilter, SessionTaskRegistry, SessionTaskState,
    TASK_KIND_AGENT_HANDOFF, TASK_KIND_SUBAGENT, TaskMessage, TaskMessagePart, find_task_executor,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

fn registry_err(e: everruns_provider::error::AgentLoopError) -> CommandError {
    CommandError::internal(anyhow::anyhow!(e))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListSessionTasks {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Optional state filter (queued, running, awaiting_input, succeeded, failed, canceled).
    #[serde(default)]
    pub state: Option<String>,
    /// Optional kind filter (subagent, external_agent, background_tool, ...).
    #[serde(default)]
    pub kind: Option<String>,
}

impl Command for ListSessionTasks {
    type Output = Vec<SessionTask>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_session_tasks",
            category: "session_tasks",
            description: "List background tasks owned by a session.",
            method: "GET",
            path: "/v1/sessions/{session_id}/tasks",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("session_id")
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&SESSION_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<SessionTask>, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        if !q::session_in_org(&ctx.db, ctx.org_id(), session_id)
            .await
            .map_err(classify_anyhow)?
        {
            return Err(CommandError::not_found("Session"));
        }
        let state = match self.state.as_deref().filter(|s| !s.is_empty()) {
            Some(raw) => Some(SessionTaskState::parse(raw).ok_or_else(|| {
                CommandError::bad_request(format!(
                    "Unknown state filter \"{raw}\". Valid states: queued, running, \
                     awaiting_input, succeeded, failed, canceled."
                ))
            })?),
            None => None,
        };
        let filter = SessionTaskFilter {
            kind: self.kind,
            state,
        };
        q::registry_for_ctx(ctx)
            .list(session_id, Some(&filter))
            .await
            .map_err(registry_err)
    }
}

inventory::submit! { CommandDescriptor::of::<ListSessionTasks>() }

/// List background tasks across every session in the caller's org.
///
/// Org-scoped observability query (EVE-583): unlike `ListSessionTasks`, this is
/// not bound to a single session. The org is taken from the authenticated
/// caller (`ctx.org_id()`), never from input, so the result is always scoped to
/// the caller's tenant.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ListOrgTasks {
    /// Optional state filter (queued, running, awaiting_input, succeeded, failed, canceled).
    #[serde(default)]
    pub state: Option<String>,
    /// Optional kind filter (subagent, external_agent, background_tool, monitor, ...).
    #[serde(default)]
    pub kind: Option<String>,
    /// Optional age filter: only tasks created at or after this RFC3339 timestamp.
    #[serde(default)]
    pub created_after: Option<String>,
    /// Optional delegation-tree filter (EVE-680): only tasks whose owning
    /// session's root is this session's prefixed public id. Returns a whole
    /// subagent tree's work in one query.
    #[serde(default)]
    pub root_session_id: Option<String>,
    /// Max tasks to return, newest first. Defaults to 100, capped at 500.
    #[serde(default)]
    pub limit: Option<u32>,
}

impl Command for ListOrgTasks {
    type Output = Vec<SessionTask>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_org_tasks",
            category: "session_tasks",
            description: "List background tasks across every session in the org.",
            method: "GET",
            path: "/v1/tasks",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&SESSION_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<SessionTask>, CommandError> {
        const DEFAULT_LIMIT: u32 = 100;
        const MAX_LIMIT: u32 = 500;

        let state = match self.state.as_deref().filter(|s| !s.is_empty()) {
            Some(raw) => Some(SessionTaskState::parse(raw).ok_or_else(|| {
                CommandError::bad_request(format!(
                    "Unknown state filter \"{raw}\". Valid states: queued, running, \
                     awaiting_input, succeeded, failed, canceled."
                ))
            })?),
            None => None,
        };
        let kind = self.kind.filter(|k| !k.is_empty());
        let created_after = match self.created_after.as_deref().filter(|s| !s.is_empty()) {
            Some(raw) => Some(
                chrono::DateTime::parse_from_rfc3339(raw)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .map_err(|e| {
                        CommandError::bad_request(format!(
                            "Invalid created_after timestamp \"{raw}\" (expected RFC3339): {e}"
                        ))
                    })?,
            ),
            None => None,
        };
        let root_session_id = match self.root_session_id.as_deref().filter(|s| !s.is_empty()) {
            Some(raw) => Some(q::parse_session_id(raw)?),
            None => None,
        };
        let limit = self.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT) as i64;
        let state = state.map(|s| s.to_string());

        let rows = ctx
            .db
            .list_org_session_tasks(
                ctx.org_id(),
                kind.as_deref(),
                state.as_deref(),
                created_after,
                root_session_id,
                limit,
            )
            .await
            .map_err(classify_anyhow)?;
        rows.iter()
            .map(|r| {
                r.to_task().map_err(|e| {
                    CommandError::internal(anyhow::anyhow!("Invalid session task row: {e}"))
                })
            })
            .collect()
    }
}

inventory::submit! { CommandDescriptor::of::<ListOrgTasks>() }

/// Task snapshot plus the recent message thread.
#[derive(Debug, Serialize, ToSchema)]
pub struct SessionTaskDetail {
    pub task: SessionTask,
    pub messages: Vec<TaskMessage>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetSessionTask {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Task's prefixed public identifier.
    pub task_id: String,
    /// Return only messages newer than this message ID (exclusive cursor).
    /// When omitted, the most recent `limit` messages are returned.
    #[serde(default)]
    pub after_id: Option<String>,
    /// Maximum number of messages to return. Defaults to 50.
    #[serde(default)]
    pub limit: Option<u32>,
}

impl Command for GetSessionTask {
    type Output = SessionTaskDetail;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_session_task",
            category: "session_tasks",
            description: "Get one session task with its recent message thread.",
            method: "GET",
            path: "/v1/sessions/{session_id}/tasks/{task_id}",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&SESSION_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<SessionTaskDetail, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        let task = q::get_task_in_org(ctx, ctx.org_id(), session_id, &self.task_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Session task"))?;
        const DEFAULT_LIMIT: u32 = 50;
        const MAX_LIMIT: u32 = 500;
        let limit = Some(self.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT));
        let messages = q::registry_for_ctx(ctx)
            .list_messages(session_id, &self.task_id, limit, self.after_id.as_deref())
            .await
            .map_err(registry_err)?;
        Ok(SessionTaskDetail { task, messages })
    }
}

inventory::submit! { CommandDescriptor::of::<GetSessionTask>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct PostSessionTaskMessage {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Task's prefixed public identifier.
    pub task_id: String,
    /// Plain-text message (alternative to `content`).
    #[serde(default)]
    pub text: Option<String>,
    /// Structured message parts (alternative to `text`).
    #[serde(default)]
    pub content: Option<Vec<TaskMessagePart>>,
    /// Input request ID this message answers, when applicable.
    #[serde(default)]
    pub in_reply_to: Option<String>,
}

impl Command for PostSessionTaskMessage {
    type Output = TaskMessage;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "post_session_task_message",
            category: "session_tasks",
            description: "Send an inbound message to a session task.",
            method: "POST",
            path: "/v1/sessions/{session_id}/tasks/{task_id}/messages",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&SESSION_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<TaskMessage, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        let task = q::get_task_in_org(ctx, ctx.org_id(), session_id, &self.task_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Session task"))?;

        if task.kind == TASK_KIND_SUBAGENT || task.kind == TASK_KIND_AGENT_HANDOFF {
            return Err(CommandError::bad_request(
                "Subagent and agent-handoff tasks are steered by their parent agent via the \
                 message_task tool; HTTP message delivery is not supported for this task kind.",
            ));
        }

        let content = match (self.content, self.text) {
            (Some(content), _) if !content.is_empty() => content,
            (_, Some(text)) if !text.trim().is_empty() => vec![TaskMessagePart::text(text)],
            _ => {
                return Err(CommandError::bad_request(
                    "Message requires non-empty `content` parts or `text`",
                ));
            }
        };
        let task_id = self.task_id.clone();
        let mut message = NewTaskMessage {
            direction: everruns_core::session_task::TaskMessageDirection::Inbound,
            content,
            in_reply_to: self.in_reply_to,
            // API writers are unfenced: user messages apply regardless of
            // executor attempt.
            expected_attempt: None,
        };
        message.in_reply_to = message.in_reply_to.filter(|s| !s.is_empty());

        let recorded = q::registry_for_ctx(ctx)
            .record_message(session_id, &task_id, message)
            .await
            .map_err(registry_err)?;

        // Best-effort executor delivery: re-fetch the task (it may have
        // transitioned to running if the message answered an input request),
        // then call deliver so the running work receives the message.
        // The message is durably recorded regardless — a delivery error (or
        // a re-fetch error) is logged but never fails the HTTP call.
        let refetch = q::registry_for_ctx(ctx).get(session_id, &task_id).await;
        match refetch {
            Ok(Some(task)) if find_task_executor(&task.kind).is_some() => {
                let executor = find_task_executor(&task.kind).unwrap();
                match q::tool_context_for_ctx(ctx, session_id).await {
                    Ok(tool_ctx) => {
                        if let Err(e) = executor.deliver(&task, &recorded, &tool_ctx).await {
                            tracing::warn!(
                                task_id = %task_id,
                                kind = %task.kind,
                                error = %e,
                                "PostSessionTaskMessage: executor deliver failed (best-effort; message is recorded)"
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            task_id = %task_id,
                            error = %e,
                            "PostSessionTaskMessage: could not build ToolContext; skipping executor deliver (message is recorded)"
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    task_id = %task_id,
                    error = %e,
                    "PostSessionTaskMessage: re-fetch after record failed; skipping executor deliver (message is recorded)"
                );
            }
            _ => {}
        }

        Ok(recorded)
    }
}

inventory::submit! { CommandDescriptor::of::<PostSessionTaskMessage>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct CancelSessionTask {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Task's prefixed public identifier.
    pub task_id: String,
}

impl Command for CancelSessionTask {
    type Output = SessionTask;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "cancel_session_task",
            category: "session_tasks",
            description: "Request cooperative cancellation of a session task.",
            method: "POST",
            path: "/v1/sessions/{session_id}/tasks/{task_id}/cancel",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&SESSION_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<SessionTask, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        if !q::session_in_org(&ctx.db, ctx.org_id(), session_id)
            .await
            .map_err(classify_anyhow)?
        {
            return Err(CommandError::not_found("Session"));
        }
        let task = q::registry_for_ctx(ctx)
            .request_cancel(session_id, &self.task_id)
            .await
            .map_err(registry_err)?
            .ok_or_else(|| CommandError::not_found("Session task"))?;

        // Best-effort executor cancel: skip for already-terminal tasks to
        // avoid side effects (e.g. disabling a schedule for a task that
        // already succeeded). For non-terminal tasks, invoke the kind-specific
        // cancel so active executors (notably MonitorTaskExecutor) can act
        // immediately. MonitorTaskExecutor.cancel disables the linked schedule
        // AND transitions the task to Canceled in the registry — so we
        // re-fetch the task afterwards to return the freshest snapshot.
        if !task.state.is_terminal()
            && let Some(executor) = find_task_executor(&task.kind)
        {
            match q::tool_context_for_ctx(ctx, session_id).await {
                Ok(tool_ctx) => {
                    if let Err(e) = executor.cancel(&task, &tool_ctx).await {
                        tracing::warn!(
                            task_id = %task.id,
                            kind = %task.kind,
                            error = %e,
                            "CancelSessionTask: executor cancel failed (best-effort)"
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        task_id = %task.id,
                        error = %e,
                        "CancelSessionTask: could not build ToolContext; skipping executor cancel (cancel intent is recorded)"
                    );
                }
            }
        }

        // Re-fetch to return the freshest snapshot: the executor may have
        // transitioned the task to Canceled (e.g. MonitorTaskExecutor does).
        // On re-fetch error, return the previously fetched snapshot rather
        // than failing the HTTP call (cancel intent is already recorded).
        let fresh = match q::registry_for_ctx(ctx)
            .get(session_id, &self.task_id)
            .await
        {
            Ok(Some(t)) => t,
            Ok(None) => task,
            Err(e) => {
                tracing::warn!(
                    task_id = %self.task_id,
                    error = %e,
                    "CancelSessionTask: re-fetch after cancel failed; returning snapshot (cancel intent is recorded)"
                );
                task
            }
        };

        Ok(fresh)
    }
}

inventory::submit! { CommandDescriptor::of::<CancelSessionTask>() }

// ============================================================================
// Per-task push-notification configs (EVE-682)
// ============================================================================

/// Valid `event_filter` members. `terminal` is the default when none supplied.
const VALID_EVENT_FILTERS: [&str; 3] = ["terminal", "awaiting_input", "message"];

fn generate_push_config_public_id() -> String {
    format!("tpc_{}", uuid::Uuid::new_v4().simple())
}

/// Normalize + validate a requested event filter. Empty / absent defaults to
/// terminal-only (matching org-webhook behavior), duplicates are removed, and
/// unknown members are rejected.
fn normalize_event_filter(input: Option<Vec<String>>) -> Result<Vec<String>, CommandError> {
    let raw = input.unwrap_or_default();
    if raw.is_empty() {
        return Ok(vec!["terminal".to_string()]);
    }
    let mut out: Vec<String> = Vec::new();
    for member in raw {
        if !VALID_EVENT_FILTERS.contains(&member.as_str()) {
            return Err(CommandError::bad_request(format!(
                "Unknown event_filter \"{member}\". Valid values: {}.",
                VALID_EVENT_FILTERS.join(", ")
            )));
        }
        if !out.contains(&member) {
            out.push(member);
        }
    }
    Ok(out)
}

/// Public view of a per-task push config. The stored secret is NEVER returned —
/// only `has_secret` signals whether one is configured.
#[derive(Debug, Serialize, ToSchema)]
pub struct TaskPushConfig {
    /// Public identifier (tpc_<32-hex-chars>).
    #[schema(example = "tpc_9f8c2b1a4e7d4c3b8a1f2e3d4c5b6a7f")]
    pub id: String,
    /// Target URL that receives POST deliveries.
    #[schema(example = "https://hooks.example.com/everruns/tasks")]
    pub url: String,
    /// Whether a signing secret is configured (the secret itself is never returned).
    #[schema(example = true)]
    pub has_secret: bool,
    /// Events that trigger delivery (terminal, awaiting_input, message).
    pub event_filter: Vec<String>,
    /// When the config was created (RFC 3339).
    #[schema(example = "2026-07-11T00:00:00Z")]
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// When the config was last updated (RFC 3339).
    #[schema(example = "2026-07-11T00:00:00Z")]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl TaskPushConfig {
    fn from_row(row: crate::storage::models::SessionTaskPushConfigRow) -> Self {
        Self {
            id: row.public_id,
            url: row.url,
            has_secret: row.secret.is_some(),
            event_filter: row.event_filter,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateTaskPushConfig {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Task's prefixed public identifier.
    pub task_id: String,
    /// URL to POST task events to.
    pub url: String,
    /// Optional HMAC-SHA256 signing secret. When set, every delivery includes
    /// an `X-Everruns-Signature: sha256=<hex>` header. Never returned once set.
    #[serde(default)]
    pub secret: Option<String>,
    /// Events that trigger delivery. Defaults to `["terminal"]`.
    #[serde(default)]
    pub event_filter: Option<Vec<String>>,
}

impl Command for CreateTaskPushConfig {
    type Output = TaskPushConfig;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_task_push_config",
            category: "session_tasks",
            description: "Create a per-task push-notification config.",
            method: "POST",
            path: "/v1/sessions/{session_id}/tasks/{task_id}/push-configs",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&SESSION_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<TaskPushConfig, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        // Authorize + existence check: the task must belong to a session in the
        // caller's org before we persist any target for it (tenant isolation).
        q::get_task_in_org(ctx, ctx.org_id(), session_id, &self.task_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Session task"))?;

        // SSRF: validate the URL against private/internal ranges before persist.
        // Delivery additionally pins DNS (see build_task_webhook_request).
        everruns_provider::url_validation::validate_safe_url(&self.url)
            .map_err(|e| CommandError::bad_request(format!("Invalid webhook URL: {e}")))?;

        let event_filter = normalize_event_filter(self.event_filter)?;
        let input = crate::storage::models::CreateSessionTaskPushConfig {
            public_id: generate_push_config_public_id(),
            session_id,
            task_id: self.task_id,
            url: self.url,
            secret: self.secret.filter(|s| !s.is_empty()),
            event_filter,
        };
        let row = ctx
            .db
            .create_task_push_config(input)
            .await
            .map_err(classify_anyhow)?;
        Ok(TaskPushConfig::from_row(row))
    }
}

inventory::submit! { CommandDescriptor::of::<CreateTaskPushConfig>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListTaskPushConfigs {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Task's prefixed public identifier.
    pub task_id: String,
}

impl Command for ListTaskPushConfigs {
    type Output = Vec<TaskPushConfig>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_task_push_configs",
            category: "session_tasks",
            description: "List per-task push-notification configs.",
            method: "GET",
            path: "/v1/sessions/{session_id}/tasks/{task_id}/push-configs",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&SESSION_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<TaskPushConfig>, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        q::get_task_in_org(ctx, ctx.org_id(), session_id, &self.task_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Session task"))?;
        let rows = ctx
            .db
            .list_task_push_configs(session_id, &self.task_id)
            .await
            .map_err(classify_anyhow)?;
        Ok(rows.into_iter().map(TaskPushConfig::from_row).collect())
    }
}

inventory::submit! { CommandDescriptor::of::<ListTaskPushConfigs>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteTaskPushConfig {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Task's prefixed public identifier.
    pub task_id: String,
    /// Push config public id (tpc_...).
    pub config_id: String,
}

impl Command for DeleteTaskPushConfig {
    type Output = ();

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_task_push_config",
            category: "session_tasks",
            description: "Delete a per-task push-notification config.",
            method: "DELETE",
            path: "/v1/sessions/{session_id}/tasks/{task_id}/push-configs/{config_id}",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&SESSION_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<(), CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        q::get_task_in_org(ctx, ctx.org_id(), session_id, &self.task_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Session task"))?;
        let deleted = ctx
            .db
            .delete_task_push_config(session_id, &self.task_id, &self.config_id)
            .await
            .map_err(classify_anyhow)?;
        if deleted {
            Ok(())
        } else {
            Err(CommandError::not_found("Push config"))
        }
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteTaskPushConfig>() }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::models::CreateHarnessRow;
    use crate::storage::{CreateSessionRow, StorageBackend};
    use everruns_core::network_access::NetworkAccessList;
    use everruns_core::session_task::{
        CreateSessionTask, NewTaskMessage, SessionTaskRegistry, SessionTaskState,
        TASK_KIND_AGENT_HANDOFF, TASK_KIND_BACKGROUND_TOOL, TASK_KIND_MONITOR, TASK_KIND_SESSION,
        TASK_KIND_SUBAGENT, TaskLinks, TaskMessagePart, TaskWakePolicy,
    };
    use everruns_core::{Caller, DEFAULT_ORG_ID};
    use everruns_provider::typed_id::{HarnessId, PrincipalId};
    use std::sync::Arc;

    // -------------------------------------------------------------------------
    // Helpers
    // -------------------------------------------------------------------------

    /// Build a minimal test Ctx backed by an in-memory StorageBackend.
    fn test_ctx(db: Arc<StorageBackend>) -> Ctx {
        Ctx::minimal_for_test(Caller::internal(DEFAULT_ORG_ID), db, None)
    }

    async fn ensure_base_harness(
        db: &Arc<StorageBackend>,
        network_access: Option<NetworkAccessList>,
    ) -> HarnessId {
        if let Some(existing) = db
            .get_harness_by_name(DEFAULT_ORG_ID, "base")
            .await
            .unwrap()
        {
            // `org_init::base_harness_id` resolves the *built-in* base harness,
            // so the helper must guarantee that same row — otherwise a stray
            // user-created "base" harness would make these tests diverge from
            // production resolution.
            assert!(
                existing.is_built_in,
                "expected the built-in base harness, found a non-built-in one"
            );
            return existing.id;
        }

        db.create_harness(
            DEFAULT_ORG_ID,
            CreateHarnessRow {
                name: "base".to_string(),
                display_name: Some("Base".to_string()),
                description: None,
                system_prompt: Some(String::new()),
                parent_harness_id: None,
                default_model_id: None,
                tags: vec![],
                initial_files: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: network_access.map(|acl| serde_json::to_value(&acl).unwrap()),
                is_built_in: true,
                embedder_metadata: Default::default(),
            },
        )
        .await
        .unwrap()
        .id
    }

    /// Create a session in the in-memory database, returning its ID.
    async fn create_session(db: &Arc<StorageBackend>) -> everruns_provider::typed_id::SessionId {
        ensure_base_harness(db, None).await;

        db.create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            org_id: DEFAULT_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: None,
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            title: Some("test session".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::json!([]),
            hints: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            blueprint_id: None,
            blueprint_config: None,
            parent_session_id: None,
            budget_root_session_id: None,
            workspace_id: None,
        })
        .await
        .unwrap()
        .id
    }

    /// Create a session owned by an arbitrary org (for cross-tenant tests).
    async fn create_session_in_org(
        db: &Arc<StorageBackend>,
        org_id: i64,
    ) -> everruns_provider::typed_id::SessionId {
        db.create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            org_id,
            app_id: None,
            harness_id: None,
            agent_id: None,
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            title: Some("other-org session".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::json!([]),
            hints: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            blueprint_id: None,
            blueprint_config: None,
            parent_session_id: None,
            budget_root_session_id: None,
            workspace_id: None,
        })
        .await
        .unwrap()
        .id
    }

    /// Create a subagent child session whose `parent_session_id` is `parent`,
    /// returning its ID. Used to build a delegation tree in tests.
    async fn create_child_session(
        db: &Arc<StorageBackend>,
        parent: everruns_provider::typed_id::SessionId,
    ) -> everruns_provider::typed_id::SessionId {
        db.create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            org_id: DEFAULT_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: None,
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            title: Some("child session".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::json!([]),
            hints: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            blueprint_id: None,
            blueprint_config: None,
            parent_session_id: Some(parent),
            budget_root_session_id: None,
            workspace_id: None,
        })
        .await
        .unwrap()
        .id
    }

    // -------------------------------------------------------------------------
    // ListOrgTasks — cross-session, org-scoped listing (EVE-583)
    // -------------------------------------------------------------------------

    /// EVE-680: a subagent child inherits its parent's `root_session_id`, and
    /// `ListOrgTasks { root_session_id }` returns exactly one delegation tree's
    /// tasks — the root's own plus every descendant's — and nothing from a
    /// sibling tree.
    #[tokio::test]
    async fn list_org_tasks_filters_by_root_session() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = test_ctx(db.clone());
        let registry = q::registry_for_ctx(&ctx);

        // Tree 1: root A → child B (a subagent). B must inherit A as its root.
        let sess_a = create_session(&db).await;
        let sess_b = create_child_session(&db, sess_a).await;
        let child = db
            .get_session(DEFAULT_ORG_ID, sess_b)
            .await
            .unwrap()
            .expect("child session exists");
        assert_eq!(
            child.root_session_id,
            Some(sess_a),
            "subagent child must inherit its parent's tree root"
        );
        let root = db
            .get_session(DEFAULT_ORG_ID, sess_a)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            root.root_session_id,
            Some(sess_a),
            "a top-level session is its own root"
        );

        // Tree 2: an independent root C — its tasks must not appear when
        // filtering on tree 1's root.
        let sess_c = create_session(&db).await;

        for (sess, name) in [(sess_a, "A-root"), (sess_b, "B-child"), (sess_c, "C-root")] {
            registry
                .create(CreateSessionTask {
                    session_id: sess,
                    id: None,
                    kind: TASK_KIND_SUBAGENT.to_string(),
                    display_name: name.to_string(),
                    spec: serde_json::json!({}),
                    state: SessionTaskState::Running,
                    links: TaskLinks::default(),
                    wake_policy: TaskWakePolicy::Silent,
                })
                .await
                .unwrap();
        }

        // Filter on tree 1's root: the root's task and the child's task, never C.
        let tree1 = ListOrgTasks {
            root_session_id: Some(sess_a.to_string()),
            ..Default::default()
        }
        .execute(&ctx)
        .await
        .unwrap();
        let names: std::collections::HashSet<_> =
            tree1.iter().map(|t| t.display_name.clone()).collect();
        assert_eq!(
            names,
            ["A-root".to_string(), "B-child".to_string()]
                .into_iter()
                .collect(),
            "root filter must return the whole tree (root + descendants) and only that tree"
        );

        // An invalid session id is a bad request, mirroring other id filters.
        let bad = ListOrgTasks {
            root_session_id: Some("not-a-session".to_string()),
            ..Default::default()
        }
        .execute(&ctx)
        .await;
        assert!(
            matches!(bad.unwrap_err().kind, CommandErrorKind::BadRequest(_)),
            "invalid root_session_id must be a bad request"
        );
    }

    /// Org-scoped listing must return every task across the caller's org while
    /// never leaking tasks owned by another org, and must honor kind/state/limit
    /// filters.
    #[tokio::test]
    async fn list_org_tasks_scopes_to_org_and_filters() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = test_ctx(db.clone()); // DEFAULT_ORG_ID
        let registry = q::registry_for_ctx(&ctx);

        // Org A (the caller's org): one subagent (running), one background_tool (queued).
        let sess_a = create_session(&db).await;
        registry
            .create(CreateSessionTask {
                session_id: sess_a,
                id: None,
                kind: TASK_KIND_SUBAGENT.to_string(),
                display_name: "A-sub".to_string(),
                spec: serde_json::json!({}),
                state: SessionTaskState::Running,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();
        registry
            .create(CreateSessionTask {
                session_id: sess_a,
                id: None,
                kind: TASK_KIND_BACKGROUND_TOOL.to_string(),
                display_name: "A-bg".to_string(),
                spec: serde_json::json!({}),
                state: SessionTaskState::Queued,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();

        // Org B (a different tenant): a task that must never surface for org A.
        let other_org = DEFAULT_ORG_ID + 12_345;
        let sess_b = create_session_in_org(&db, other_org).await;
        registry
            .create(CreateSessionTask {
                session_id: sess_b,
                id: None,
                kind: TASK_KIND_SUBAGENT.to_string(),
                display_name: "B-sub".to_string(),
                spec: serde_json::json!({}),
                state: SessionTaskState::Running,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();

        // Unfiltered: only org A's two tasks, never org B's.
        let all = ListOrgTasks::default().execute(&ctx).await.unwrap();
        assert_eq!(all.len(), 2, "must list exactly org A's two tasks");
        assert!(
            all.iter().all(|t| t.session_id == sess_a),
            "must never leak another org's tasks"
        );

        // Kind filter.
        let subs = ListOrgTasks {
            kind: Some("subagent".to_string()),
            ..Default::default()
        }
        .execute(&ctx)
        .await
        .unwrap();
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].kind, TASK_KIND_SUBAGENT);

        // State filter.
        let queued = ListOrgTasks {
            state: Some("queued".to_string()),
            ..Default::default()
        }
        .execute(&ctx)
        .await
        .unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].state, SessionTaskState::Queued);

        // Limit caps the result set.
        let one = ListOrgTasks {
            limit: Some(1),
            ..Default::default()
        }
        .execute(&ctx)
        .await
        .unwrap();
        assert_eq!(one.len(), 1, "limit must bound the result set");

        // Age filter: a future cutoff excludes everything; bad input is rejected.
        let future = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
        let none = ListOrgTasks {
            created_after: Some(future),
            ..Default::default()
        }
        .execute(&ctx)
        .await
        .unwrap();
        assert!(none.is_empty(), "future created_after must exclude all");

        let bad = ListOrgTasks {
            created_after: Some("not-a-timestamp".to_string()),
            ..Default::default()
        }
        .execute(&ctx)
        .await;
        assert!(
            matches!(bad.unwrap_err().kind, CommandErrorKind::BadRequest(_)),
            "invalid created_after must be a bad request"
        );
    }

    // -------------------------------------------------------------------------
    // CancelSessionTask — monitor task
    // -------------------------------------------------------------------------

    /// API cancel of a monitor task must disable the linked schedule via
    /// MonitorTaskExecutor and return the task in Canceled state.
    #[tokio::test]
    async fn cancel_monitor_task_cancels_linked_schedule() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_id = create_session(&db).await;
        let ctx = test_ctx(db.clone());

        // Create a schedule directly in storage so we have a real schedule_id.
        let schedule = db
            .create_session_schedule(crate::storage::CreateSessionScheduleRow {
                org_id: DEFAULT_ORG_ID,
                session_id,
                owner_principal_id: PrincipalId::from_seed(1),
                resolved_owner_user_id: None,
                description: "test monitor schedule".to_string(),
                cron_expression: Some("0 * * * *".to_string()),
                scheduled_at: None,
                timezone: "UTC".to_string(),
                next_trigger_at: None,
            })
            .await
            .unwrap();
        assert!(schedule.enabled, "schedule must start enabled");
        let schedule_id = schedule.id;

        // Create a monitor task with the schedule_id in spec.
        let registry = q::registry_for_ctx(&ctx);
        let task = registry
            .create(CreateSessionTask {
                session_id,
                id: None,
                kind: TASK_KIND_MONITOR.to_string(),
                display_name: "Test Monitor".to_string(),
                spec: serde_json::json!({ "schedule_id": schedule_id.to_string() }),
                state: SessionTaskState::Running,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();

        // Call the cancel command via execute (bypassing policy).
        let result = CancelSessionTask {
            session_id: session_id.to_string(),
            task_id: task.id.clone(),
        }
        .execute(&ctx)
        .await
        .unwrap();

        // Task must be Canceled (MonitorTaskExecutor transitions it).
        assert_eq!(
            result.state,
            SessionTaskState::Canceled,
            "monitor task must be Canceled after API cancel"
        );

        // Schedule must be disabled (MonitorTaskExecutor called cancel_schedule).
        let updated_schedule = db
            .get_session_schedule(DEFAULT_ORG_ID, schedule_id)
            .await
            .unwrap()
            .expect("schedule must still exist");
        assert!(
            !updated_schedule.enabled,
            "schedule must be disabled after monitor task cancel"
        );
    }

    /// API cancellation must not claim a detached peer was canceled when the
    /// command context cannot deliver the cooperative stop message.
    #[tokio::test]
    async fn cancel_detached_session_task_without_platform_store_fails_closed() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_id = create_session(&db).await;
        let ctx = test_ctx(db.clone());
        let registry = q::registry_for_ctx(&ctx);
        let task = registry
            .create(CreateSessionTask {
                session_id,
                id: None,
                kind: TASK_KIND_SESSION.to_string(),
                display_name: "Detached peer".to_string(),
                spec: serde_json::json!({}),
                state: SessionTaskState::Running,
                links: TaskLinks {
                    child_session_id: Some(everruns_provider::typed_id::SessionId::new()),
                    ..Default::default()
                },
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();

        let result = CancelSessionTask {
            session_id: session_id.to_string(),
            task_id: task.id,
        }
        .execute(&ctx)
        .await
        .unwrap();

        assert_eq!(result.state, SessionTaskState::Running);
        assert!(result.cancel_requested_at.is_some());
        assert!(result.summary.is_none());
    }

    // -------------------------------------------------------------------------
    // PostSessionTaskMessage — subagent kind (must be rejected)
    // -------------------------------------------------------------------------

    /// Posting a message to a subagent task must return 400; subagent steering
    /// is internal-channel only (parent agent's message_task tool).
    #[tokio::test]
    async fn post_message_to_subagent_task_returns_bad_request() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_id = create_session(&db).await;
        let ctx = test_ctx(db.clone());

        let registry = q::registry_for_ctx(&ctx);
        let task = registry
            .create(CreateSessionTask {
                session_id,
                id: None,
                kind: TASK_KIND_SUBAGENT.to_string(),
                display_name: "Subagent Task".to_string(),
                spec: serde_json::json!({}),
                state: SessionTaskState::Running,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();

        let result = PostSessionTaskMessage {
            session_id: session_id.to_string(),
            task_id: task.id.clone(),
            text: Some("steer me".to_string()),
            content: None,
            in_reply_to: None,
        }
        .execute(&ctx)
        .await;

        assert!(result.is_err(), "subagent task message must be rejected");
        let err = result.unwrap_err();
        assert!(
            matches!(err.kind, CommandErrorKind::BadRequest(_)),
            "must be a BadRequest error, got: {:?}",
            err.kind
        );

        // No message must have been recorded.
        let messages = registry
            .list_messages(session_id, &task.id, Some(10), None)
            .await
            .unwrap();
        assert!(
            messages.is_empty(),
            "no message must be persisted for subagent tasks"
        );
    }

    /// Handoff tasks are also parent-steered through generic task tools, so the
    /// HTTP message endpoint must reject them exactly like subagent tasks.
    #[tokio::test]
    async fn post_message_to_agent_handoff_task_returns_bad_request() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_id = create_session(&db).await;
        let ctx = test_ctx(db.clone());

        let registry = q::registry_for_ctx(&ctx);
        let task = registry
            .create(CreateSessionTask {
                session_id,
                id: None,
                kind: TASK_KIND_AGENT_HANDOFF.to_string(),
                display_name: "AWS Operator".to_string(),
                spec: serde_json::json!({ "target_id": "aws", "external_agent_id": "agent_aws" }),
                state: SessionTaskState::Running,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();

        let result = PostSessionTaskMessage {
            session_id: session_id.to_string(),
            task_id: task.id.clone(),
            text: Some("steer me".to_string()),
            content: None,
            in_reply_to: None,
        }
        .execute(&ctx)
        .await;

        assert!(result.is_err(), "handoff task message must be rejected");
        let err = result.unwrap_err();
        assert!(
            matches!(err.kind, CommandErrorKind::BadRequest(_)),
            "must be a BadRequest error, got: {:?}",
            err.kind
        );

        let messages = registry
            .list_messages(session_id, &task.id, Some(10), None)
            .await
            .unwrap();
        assert!(
            messages.is_empty(),
            "no message must be persisted for agent-handoff tasks"
        );
    }

    // -------------------------------------------------------------------------
    // PostSessionTaskMessage — unknown/no-executor kind
    // -------------------------------------------------------------------------

    /// Posting a message to a task whose kind has no executor registered
    /// must still return 200 and a recorded TaskMessage (no executor → no-op
    /// delivery, message is durably stored).
    #[tokio::test]
    async fn post_message_to_no_executor_kind_returns_recorded_message() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_id = create_session(&db).await;
        let ctx = test_ctx(db.clone());

        // Create a task with a kind that has no registered executor.
        let registry = q::registry_for_ctx(&ctx);
        let task = registry
            .create(CreateSessionTask {
                session_id,
                id: None,
                kind: "unknown_test_kind".to_string(),
                display_name: "Unknown Kind Task".to_string(),
                spec: serde_json::json!({}),
                state: SessionTaskState::Running,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();

        let result = PostSessionTaskMessage {
            session_id: session_id.to_string(),
            task_id: task.id.clone(),
            text: Some("hello from API".to_string()),
            content: None,
            in_reply_to: None,
        }
        .execute(&ctx)
        .await
        .unwrap();

        // The returned message must be the recorded inbound message.
        assert_eq!(
            result.task_id, task.id,
            "recorded message must belong to the task"
        );
        assert_eq!(
            result.direction,
            everruns_core::session_task::TaskMessageDirection::Inbound
        );

        // The message thread must contain the message.
        let messages = registry
            .list_messages(session_id, &task.id, Some(10), None)
            .await
            .unwrap();
        assert_eq!(messages.len(), 1, "message must be persisted");
    }

    // -------------------------------------------------------------------------
    // PostSessionTaskMessage — background_tool kind (executor registered, deliver
    // returns unsupported — best-effort: HTTP call still succeeds)
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn post_message_to_background_tool_still_returns_200() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_id = create_session(&db).await;
        let ctx = test_ctx(db.clone());

        let registry = q::registry_for_ctx(&ctx);
        let task = registry
            .create(CreateSessionTask {
                session_id,
                id: None,
                kind: TASK_KIND_BACKGROUND_TOOL.to_string(),
                display_name: "Background Tool Task".to_string(),
                spec: serde_json::json!({}),
                state: SessionTaskState::Running,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();

        // BackgroundToolTaskExecutor.deliver returns an error (unsupported).
        // The command must still return Ok with the recorded message.
        let result = PostSessionTaskMessage {
            session_id: session_id.to_string(),
            task_id: task.id.clone(),
            text: Some("steer the tool".to_string()),
            content: None,
            in_reply_to: None,
        }
        .execute(&ctx)
        .await
        .unwrap();

        assert_eq!(result.task_id, task.id);
        let messages = registry
            .list_messages(session_id, &task.id, Some(10), None)
            .await
            .unwrap();
        assert_eq!(messages.len(), 1);
    }

    // -------------------------------------------------------------------------
    // GetSessionTask — cursor pagination
    // -------------------------------------------------------------------------

    /// `after_id` returns only messages newer than the cursor, oldest-first.
    #[tokio::test]
    async fn get_task_after_id_returns_messages_after_cursor() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_id = create_session(&db).await;
        let ctx = test_ctx(db.clone());

        let registry = q::registry_for_ctx(&ctx);
        let task = registry
            .create(CreateSessionTask {
                session_id,
                id: None,
                kind: "unknown_kind".to_string(),
                display_name: "Cursor Test".to_string(),
                spec: serde_json::json!({}),
                state: SessionTaskState::Running,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();

        // Record 5 messages.
        let mut ids = Vec::new();
        for i in 0..5u32 {
            let m = registry
                .record_message(
                    session_id,
                    &task.id,
                    NewTaskMessage {
                        direction: everruns_core::session_task::TaskMessageDirection::Inbound,
                        content: vec![TaskMessagePart::text(format!("msg{i}"))],
                        in_reply_to: None,
                        expected_attempt: None,
                    },
                )
                .await
                .unwrap();
            ids.push(m.id);
        }

        // after_id = ids[1] → should return msgs 2, 3, 4
        let result = GetSessionTask {
            session_id: session_id.to_string(),
            task_id: task.id.clone(),
            after_id: Some(ids[1].clone()),
            limit: None,
        }
        .execute(&ctx)
        .await
        .unwrap();

        assert_eq!(
            result.messages.len(),
            3,
            "should have 3 messages after cursor"
        );
        assert_eq!(result.messages[0].id, ids[2]);
        assert_eq!(result.messages[2].id, ids[4]);

        // after_id + limit = 1 → should return only msg 2
        let result_limited = GetSessionTask {
            session_id: session_id.to_string(),
            task_id: task.id.clone(),
            after_id: Some(ids[1].clone()),
            limit: Some(1),
        }
        .execute(&ctx)
        .await
        .unwrap();

        assert_eq!(result_limited.messages.len(), 1);
        assert_eq!(result_limited.messages[0].id, ids[2]);
    }

    // -------------------------------------------------------------------------
    // CancelSessionTask — terminal task must not invoke executor
    // -------------------------------------------------------------------------

    /// API cancel of an already-terminal (succeeded) monitor task must NOT
    /// invoke the executor. The linked schedule must remain enabled because
    /// MonitorTaskExecutor.cancel was never called.
    #[tokio::test]
    async fn cancel_terminal_monitor_task_skips_executor() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_id = create_session(&db).await;
        let ctx = test_ctx(db.clone());

        // Create a schedule that must remain enabled.
        let schedule = db
            .create_session_schedule(crate::storage::CreateSessionScheduleRow {
                org_id: DEFAULT_ORG_ID,
                session_id,
                owner_principal_id: PrincipalId::from_seed(1),
                resolved_owner_user_id: None,
                description: "terminal monitor schedule".to_string(),
                cron_expression: Some("0 * * * *".to_string()),
                scheduled_at: None,
                timezone: "UTC".to_string(),
                next_trigger_at: None,
            })
            .await
            .unwrap();
        let schedule_id = schedule.id;
        assert!(schedule.enabled, "schedule must start enabled");

        // Create the monitor task already in Succeeded state.
        let registry = q::registry_for_ctx(&ctx);
        let task = registry
            .create(CreateSessionTask {
                session_id,
                id: None,
                kind: TASK_KIND_MONITOR.to_string(),
                display_name: "Succeeded Monitor".to_string(),
                spec: serde_json::json!({ "schedule_id": schedule_id.to_string() }),
                state: SessionTaskState::Succeeded,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();

        // API cancel on a terminal task.
        let result = CancelSessionTask {
            session_id: session_id.to_string(),
            task_id: task.id.clone(),
        }
        .execute(&ctx)
        .await
        .unwrap();

        // request_cancel records cancel_requested_at but does NOT change state
        // for terminal tasks (registry invariant); the returned task reflects
        // the terminal state.
        assert_eq!(
            result.state,
            SessionTaskState::Succeeded,
            "terminal task state must not regress after API cancel"
        );

        // Schedule must still be enabled — MonitorTaskExecutor.cancel was NOT called.
        let updated_schedule = db
            .get_session_schedule(DEFAULT_ORG_ID, schedule_id)
            .await
            .unwrap()
            .expect("schedule must still exist");
        assert!(
            updated_schedule.enabled,
            "schedule must remain enabled when cancel skips executor for terminal task"
        );
    }

    // -------------------------------------------------------------------------
    // Per-task push configs (EVE-682)
    // -------------------------------------------------------------------------

    async fn make_task(
        db: &Arc<StorageBackend>,
        session_id: everruns_provider::typed_id::SessionId,
    ) -> String {
        let ctx = test_ctx(db.clone());
        q::registry_for_ctx(&ctx)
            .create(CreateSessionTask {
                session_id,
                id: None,
                kind: TASK_KIND_BACKGROUND_TOOL.to_string(),
                display_name: "push cfg task".to_string(),
                spec: serde_json::json!({}),
                state: SessionTaskState::Running,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap()
            .id
    }

    /// Create → list → delete round-trip. The stored secret is never echoed
    /// (only `has_secret`), and event_filter defaults to terminal-only.
    #[tokio::test]
    async fn push_config_crud_never_returns_secret() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_id = create_session(&db).await;
        let task_id = make_task(&db, session_id).await;
        let ctx = test_ctx(db.clone());

        // Default filter is terminal-only; secret is accepted but never echoed.
        let created = CreateTaskPushConfig {
            session_id: session_id.to_string(),
            task_id: task_id.clone(),
            url: "https://hooks.example.com/notify".to_string(),
            secret: Some("s3cret".to_string()),
            event_filter: None,
        }
        .execute(&ctx)
        .await
        .unwrap();
        assert!(created.id.starts_with("tpc_"));
        assert!(
            created.has_secret,
            "has_secret must reflect the stored secret"
        );
        assert_eq!(created.event_filter, vec!["terminal".to_string()]);
        // The response type has no secret field — verify the serialized form too.
        let json = serde_json::to_value(&created).unwrap();
        assert!(
            json.get("secret").is_none(),
            "serialized push config must never contain the secret"
        );

        let listed = ListTaskPushConfigs {
            session_id: session_id.to_string(),
            task_id: task_id.clone(),
        }
        .execute(&ctx)
        .await
        .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, created.id);

        let del = DeleteTaskPushConfig {
            session_id: session_id.to_string(),
            task_id: task_id.clone(),
            config_id: created.id.clone(),
        }
        .execute(&ctx)
        .await;
        assert!(del.is_ok());

        // Deleting again is a not-found.
        let again = DeleteTaskPushConfig {
            session_id: session_id.to_string(),
            task_id,
            config_id: created.id,
        }
        .execute(&ctx)
        .await;
        assert!(matches!(
            again.unwrap_err().kind,
            CommandErrorKind::NotFound(_)
        ));
    }

    /// SSRF guard: a private/internal URL is rejected before any row is written.
    #[tokio::test]
    async fn push_config_rejects_unsafe_url() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_id = create_session(&db).await;
        let task_id = make_task(&db, session_id).await;
        let ctx = test_ctx(db.clone());

        let err = CreateTaskPushConfig {
            session_id: session_id.to_string(),
            task_id,
            url: "http://169.254.169.254/latest/meta-data".to_string(),
            secret: None,
            event_filter: None,
        }
        .execute(&ctx)
        .await
        .unwrap_err();
        assert!(matches!(err.kind, CommandErrorKind::BadRequest(_)));
    }

    /// An unknown event_filter member is rejected.
    #[tokio::test]
    async fn push_config_rejects_unknown_event_filter() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_id = create_session(&db).await;
        let task_id = make_task(&db, session_id).await;
        let ctx = test_ctx(db.clone());

        let err = CreateTaskPushConfig {
            session_id: session_id.to_string(),
            task_id,
            url: "https://hooks.example.com/notify".to_string(),
            secret: None,
            event_filter: Some(vec!["bogus".to_string()]),
        }
        .execute(&ctx)
        .await
        .unwrap_err();
        assert!(matches!(err.kind, CommandErrorKind::BadRequest(_)));
    }

    /// Tenant isolation: the caller's org (DEFAULT) must not create a push
    /// config on a task owned by a session in another org — it reads as
    /// not-found, never leaking existence.
    #[tokio::test]
    async fn push_config_cross_org_is_not_found() {
        let db = Arc::new(StorageBackend::in_memory());
        let other_org = DEFAULT_ORG_ID + 999;
        let other_session = create_session_in_org(&db, other_org).await;
        let task_id = make_task(&db, other_session).await;

        // ctx is DEFAULT_ORG_ID — a different tenant than the task's owner.
        let ctx = test_ctx(db.clone());
        let err = CreateTaskPushConfig {
            session_id: other_session.to_string(),
            task_id,
            url: "https://hooks.example.com/notify".to_string(),
            secret: None,
            event_filter: None,
        }
        .execute(&ctx)
        .await
        .unwrap_err();
        assert!(matches!(err.kind, CommandErrorKind::NotFound(_)));
    }

    // -------------------------------------------------------------------------
    // tool_context_for_ctx — network_access is populated from folded overlays
    // -------------------------------------------------------------------------

    /// The factory must derive the effective network ACL by folding harness →
    /// agent → session overlays and set it on the returned ToolContext.
    ///
    /// Test setup:
    ///   harness allows [a.example.com, b.example.com]
    ///   session allows [b.example.com]
    ///   expected intersection: [b.example.com]  (a.example.com is narrowed out)
    #[tokio::test]
    async fn tool_context_for_ctx_populates_network_access() {
        let db = Arc::new(StorageBackend::in_memory());

        // Create a harness with a network_access list.
        let harness_network_access =
            NetworkAccessList::allow_only(["a.example.com", "b.example.com"]);
        let harness = db
            .create_harness(
                DEFAULT_ORG_ID,
                CreateHarnessRow {
                    name: "acl-test-harness".to_string(),
                    display_name: None,
                    description: None,
                    system_prompt: Some(String::new()),
                    parent_harness_id: None,
                    default_model_id: None,
                    tags: vec![],
                    initial_files: serde_json::json!([]),
                    mcp_servers: serde_json::json!({}),
                    network_access: Some(serde_json::to_value(&harness_network_access).unwrap()),
                    embedder_metadata: serde_json::json!({}),
                    is_built_in: false,
                },
            )
            .await
            .unwrap();

        // Create a session with a narrower network_access list, linked to the harness.
        let session_network_access = NetworkAccessList::allow_only(["b.example.com"]);
        let session_id = db
            .create_session(CreateSessionRow {
                source: everruns_platform::SessionSource::Api,
                org_id: DEFAULT_ORG_ID,
                app_id: None,
                harness_id: Some(harness.id),
                agent_id: None,
                agent_version_id: None,
                agent_config_hash: None,
                agent_identity_id: None,
                owner_principal_id: PrincipalId::from_seed(1),
                resolved_owner_user_id: None,
                title: Some("acl test session".to_string()),
                locale: None,
                tags: vec![],
                model_id: None,
                capabilities: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                system_prompt: None,
                initial_files: serde_json::json!([]),
                hints: None,
                network_access: Some(serde_json::to_value(&session_network_access).unwrap()),
                max_iterations: None,
                parallel_tool_calls: None,
                blueprint_id: None,
                blueprint_config: None,
                parent_session_id: None,
                budget_root_session_id: None,
                workspace_id: None,
            })
            .await
            .unwrap()
            .id;

        let ctx = test_ctx(db.clone());
        let tool_ctx = q::tool_context_for_ctx(&ctx, session_id)
            .await
            .expect("tool_context_for_ctx must succeed");

        let acl = tool_ctx
            .network_access
            .expect("network_access must be populated");

        // b.example.com is in both layers → allowed after intersection.
        assert!(
            acl.is_url_allowed("https://b.example.com/ok"),
            "b.example.com must be allowed (in both harness and session)"
        );
        // a.example.com is only in harness, not session → blocked by intersection.
        assert!(
            !acl.is_url_allowed("https://a.example.com/ok"),
            "a.example.com must be blocked (not in session allow list)"
        );
        // Unrelated host must be blocked.
        assert!(
            !acl.is_url_allowed("https://other.example.com/ok"),
            "other.example.com must be blocked"
        );
    }
    /// Legacy sessions can have NULL harness_id. The ToolContext builder must
    /// resolve the normal base-harness fallback so executor ACLs remain scoped.
    #[tokio::test]
    async fn tool_context_for_ctx_uses_base_harness_for_null_session_harness() {
        let db = Arc::new(StorageBackend::in_memory());
        let base_network_access = NetworkAccessList::allow_only(["base.example.com"]);
        ensure_base_harness(&db, Some(base_network_access)).await;
        let session_id = create_session(&db).await;
        let ctx = test_ctx(db.clone());

        let tool_ctx = q::tool_context_for_ctx(&ctx, session_id)
            .await
            .expect("tool_context_for_ctx must resolve the base harness");

        let acl = tool_ctx
            .network_access
            .expect("base harness network_access must be applied");
        assert!(
            acl.is_url_allowed("https://base.example.com/ok"),
            "base harness allowlist must permit listed hosts"
        );
        assert!(
            !acl.is_url_allowed("https://other.example.com/blocked"),
            "base harness allowlist must block unlisted hosts"
        );
    }

    /// A session that references a deleted/stale harness must fail closed rather
    /// than building an unrestricted ToolContext for executor network calls.
    #[tokio::test]
    async fn tool_context_for_ctx_fails_closed_for_missing_session_harness() {
        let db = Arc::new(StorageBackend::in_memory());
        let stale_harness_id = HarnessId::new();
        let session_id = db
            .create_session(CreateSessionRow {
                source: everruns_platform::SessionSource::Api,
                org_id: DEFAULT_ORG_ID,
                app_id: None,
                harness_id: Some(stale_harness_id),
                agent_id: None,
                agent_version_id: None,
                agent_config_hash: None,
                agent_identity_id: None,
                owner_principal_id: PrincipalId::from_seed(1),
                resolved_owner_user_id: None,
                title: Some("stale harness session".to_string()),
                locale: None,
                tags: vec![],
                model_id: None,
                capabilities: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                system_prompt: None,
                initial_files: serde_json::json!([]),
                hints: None,
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
                blueprint_id: None,
                blueprint_config: None,
                parent_session_id: None,
                budget_root_session_id: None,
                workspace_id: None,
            })
            .await
            .unwrap()
            .id;
        let ctx = test_ctx(db.clone());

        let err = q::tool_context_for_ctx(&ctx, session_id)
            .await
            .expect_err("missing harness must fail closed");
        assert!(
            err.to_string().contains("Leaf harness not found"),
            "unexpected error: {err}"
        );
    }
}
