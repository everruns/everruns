use super::queries as q;
use crate::domains::common::*;
use crate::domains::sessions::{SESSION_MANAGE, SESSION_VIEW};
use everruns_core::SessionTask;
use everruns_core::session_task::{
    NewTaskMessage, SessionTaskFilter, SessionTaskRegistry, SessionTaskState,
    TASK_KIND_AGENT_HANDOFF, TASK_KIND_ASSIGNMENT, TASK_KIND_SUBAGENT, TaskMessage,
    TaskMessagePart, find_task_executor,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

fn registry_err(e: everruns_contracts::error::AgentLoopError) -> CommandError {
    CommandError::internal(anyhow::anyhow!(e))
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
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

#[command(
    name = "list_session_tasks",
    category = "session_tasks",
    description = "List background tasks owned by a session.",
    method = "GET",
    path = "/v1/sessions/{session_id}/tasks",
    policy = SESSION_VIEW,
    positional = "session_id",
)]
impl Command for ListSessionTasks {
    type Output = Vec<SessionTask>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<SessionTask>, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        if !q::session_in_org(&ctx.db, ctx.org_id(), session_id).await? {
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

/// List background tasks across every session in the caller's org.
///
/// Org-scoped observability query (EVE-583): unlike `ListSessionTasks`, this is
/// not bound to a single session. The org is taken from the authenticated
/// caller (`ctx.org_id()`), never from input, so the result is always scoped to
/// the caller's tenant.
#[derive(Debug, Default, Deserialize, ToSchema, serde::Serialize)]
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

#[command(
    name = "list_org_tasks",
    category = "session_tasks",
    description = "List background tasks across every session in the org.",
    method = "GET",
    path = "/v1/tasks",
    policy = SESSION_VIEW,
)]
impl Command for ListOrgTasks {
    type Output = Vec<SessionTask>;

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
            .await?;
        rows.iter()
            .map(|r| {
                r.to_task().map_err(|e| {
                    CommandError::internal(anyhow::anyhow!("Invalid session task row: {e}"))
                })
            })
            .collect()
    }
}

/// Task snapshot plus the recent message thread.
#[derive(Debug, Serialize, ToSchema)]
pub struct SessionTaskDetail {
    pub task: SessionTask,
    pub messages: Vec<TaskMessage>,
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
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

#[command(
    name = "get_session_task",
    category = "session_tasks",
    description = "Get one session task with its recent message thread.",
    method = "GET",
    path = "/v1/sessions/{session_id}/tasks/{task_id}",
    policy = SESSION_VIEW,
)]
impl Command for GetSessionTask {
    type Output = SessionTaskDetail;

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

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
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

#[command(
    name = "post_session_task_message",
    category = "session_tasks",
    description = "Send an inbound message to a session task.",
    method = "POST",
    path = "/v1/sessions/{session_id}/tasks/{task_id}/messages",
    policy = SESSION_MANAGE,
)]
impl Command for PostSessionTaskMessage {
    type Output = TaskMessage;

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
        if task.kind == TASK_KIND_ASSIGNMENT {
            return Err(CommandError::bad_request(
                "Assignments belong to a thread: send the message to the thread session \
                 itself, or ask the coordinator to relay it.",
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

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct CancelSessionTask {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Task's prefixed public identifier.
    pub task_id: String,
}

#[command(
    name = "cancel_session_task",
    category = "session_tasks",
    description = "Request cooperative cancellation of a session task.",
    method = "POST",
    path = "/v1/sessions/{session_id}/tasks/{task_id}/cancel",
    policy = SESSION_MANAGE,
)]
impl Command for CancelSessionTask {
    type Output = SessionTask;

    async fn execute(self, ctx: &Ctx) -> Result<SessionTask, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        if !q::session_in_org(&ctx.db, ctx.org_id(), session_id).await? {
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

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
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

#[command(
    name = "create_task_push_config",
    category = "session_tasks",
    description = "Create a per-task push-notification config.",
    method = "POST",
    path = "/v1/sessions/{session_id}/tasks/{task_id}/push-configs",
    policy = SESSION_MANAGE,
)]
impl Command for CreateTaskPushConfig {
    type Output = TaskPushConfig;

    async fn execute(self, ctx: &Ctx) -> Result<TaskPushConfig, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        // Authorize + existence check: the task must belong to a session in the
        // caller's org before we persist any target for it (tenant isolation).
        q::get_task_in_org(ctx, ctx.org_id(), session_id, &self.task_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Session task"))?;

        // SSRF: validate the URL against private/internal ranges before persist.
        // Delivery additionally pins DNS (see build_task_webhook_request).
        everruns_contracts::url_validation::validate_safe_url(&self.url)
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
        let row = ctx.db.create_task_push_config(input).await?;
        Ok(TaskPushConfig::from_row(row))
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ListTaskPushConfigs {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Task's prefixed public identifier.
    pub task_id: String,
}

#[command(
    name = "list_task_push_configs",
    category = "session_tasks",
    description = "List per-task push-notification configs.",
    method = "GET",
    path = "/v1/sessions/{session_id}/tasks/{task_id}/push-configs",
    policy = SESSION_VIEW,
)]
impl Command for ListTaskPushConfigs {
    type Output = Vec<TaskPushConfig>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<TaskPushConfig>, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        q::get_task_in_org(ctx, ctx.org_id(), session_id, &self.task_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Session task"))?;
        let rows = ctx
            .db
            .list_task_push_configs(session_id, &self.task_id)
            .await?;
        Ok(rows.into_iter().map(TaskPushConfig::from_row).collect())
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct DeleteTaskPushConfig {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Task's prefixed public identifier.
    pub task_id: String,
    /// Push config public id (tpc_...).
    pub config_id: String,
}

#[command(
    name = "delete_task_push_config",
    category = "session_tasks",
    description = "Delete a per-task push-notification config.",
    method = "DELETE",
    path = "/v1/sessions/{session_id}/tasks/{task_id}/push-configs/{config_id}",
    policy = SESSION_MANAGE,
)]
impl Command for DeleteTaskPushConfig {
    type Output = ();

    async fn execute(self, ctx: &Ctx) -> Result<(), CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        q::get_task_in_org(ctx, ctx.org_id(), session_id, &self.task_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Session task"))?;
        let deleted = ctx
            .db
            .delete_task_push_config(session_id, &self.task_id, &self.config_id)
            .await?;
        if deleted {
            Ok(())
        } else {
            Err(CommandError::not_found("Push config"))
        }
    }
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;
