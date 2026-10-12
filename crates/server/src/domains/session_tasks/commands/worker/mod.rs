//! The worker's session task registry, as internal commands.
//!
//! Decision: the runtime's `SessionTaskRegistry` (subagents, background tools,
//! monitors, external agents and their sinks) reaches these through
//! `ExecuteCommand` (gRPC workers) and `dispatch` (the in-process worker), so
//! one implementation serves both. They used to be seven bespoke RPCs that took
//! the session alone; as internal commands they run as the worker's org and
//! first confirm the session belongs to it. A task id is only ever looked up
//! within that session, so naming another session's task finds nothing.
//!
//! The registry they build is the worker's, not the public API's: it emits
//! `task.*` events and wakes the session per `wake_policy` (the API path must
//! not wake), and delivers task webhooks when the context has an egress
//! service. Tasks go back with their full `spec`: `SessionTask`'s own
//! serialization redacts it for public readers, but executors and the reaper
//! need the push-config secrets and origin event context it holds.
//!
//! The reaper's cross-org `ListOrphanedSessionTasks` and
//! `PruneTerminalSessionTasks` stay dedicated RPCs: they scan every org.

use crate::domains::common::*;
use crate::domains::session_storage::queries::verify_session_ownership;
use crate::domains::sessions::{SESSION_MANAGE, SESSION_VIEW};
use crate::kernel_imports::contracts::typed_id::SessionId;
use crate::storage::runtime::session_task::{
    DbSessionTaskRegistry, InjectedMessageWaker, TaskWebhookNotifier,
};
use everruns_core::SessionTask;
use everruns_core::session_task::{
    CreateSessionTask, NewTaskMessage, SessionTaskFilter, SessionTaskRegistry, SessionTaskState,
    SessionTaskUpdate, TaskMessage,
};
use serde::{Deserialize, Serialize, Serializer};
use std::sync::Arc;
use utoipa::ToSchema;

/// The session, after confirming it belongs to the caller's org: the registry
/// addresses rows by session alone.
async fn owned_session(ctx: &Ctx, session_id: &str) -> Result<SessionId, CommandError> {
    let session_id = super::q::parse_session_id(session_id)?;
    verify_session_ownership(&ctx.db, ctx.org_id(), session_id).await?;
    Ok(session_id)
}

/// The registry a worker writes through: events, wakes and webhooks, as the
/// RPCs' registry had (webhooks only where the context can reach egress).
fn registry(ctx: &Ctx) -> DbSessionTaskRegistry {
    let mut registry = DbSessionTaskRegistry::new(ctx.db.clone());
    if let Some(events) = &ctx.event_service {
        registry = registry
            .with_event_emitter(events.clone())
            .with_waker(Arc::new(InjectedMessageWaker {
                db: ctx.db.clone(),
                event_service: (**events).clone(),
                runner: ctx.runner.clone(),
            }));
    }
    if let Some(egress) = &ctx.egress_service {
        registry = registry.with_transition_observer(Arc::new(TaskWebhookNotifier {
            db: ctx.db.clone(),
            egress_service: egress.clone(),
        }));
    }
    registry
}

fn registry_failure(error: everruns_contracts::error::AgentLoopError) -> CommandError {
    CommandError::internal(anyhow::anyhow!(error))
}

/// The task within `session_id`, or NotFound: message writes and reads
/// name a task that must exist in the session they were checked against.
async fn owned_task(
    registry: &DbSessionTaskRegistry,
    session_id: SessionId,
    task_id: &str,
) -> Result<SessionTask, CommandError> {
    registry
        .get(session_id, task_id)
        .await
        .map_err(registry_failure)?
        .ok_or_else(|| CommandError::not_found("Session task"))
}

/// A task as the worker reads it, `spec` unredacted.
#[derive(Debug)]
pub struct WorkerSessionTask(pub SessionTask);

impl Serialize for WorkerSessionTask {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut value = serde_json::to_value(&self.0).map_err(serde::ser::Error::custom)?;
        if let Some(object) = value.as_object_mut() {
            object.insert("spec".to_string(), self.0.spec.clone());
        }
        value.serialize(serializer)
    }
}

fn worker_task(task: Option<SessionTask>) -> Option<WorkerSessionTask> {
    task.map(WorkerSessionTask)
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerCreateSessionTask {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// The task to create; its `session_id` must be this session.
    #[schema(value_type = Object)]
    pub task: CreateSessionTask,
}

#[command(
    name = "worker_create_session_task",
    category = "session_tasks",
    description = "Internal: create a session task (idempotent on a supplied id).",
    method = "POST",
    path = "/internal/sessions/{session_id}/tasks",
    policy = SESSION_MANAGE
)]
impl Command for WorkerCreateSessionTask {
    type Output = WorkerSessionTask;

    async fn execute(self, ctx: &Ctx) -> Result<WorkerSessionTask, CommandError> {
        let session_id = owned_session(ctx, &self.session_id).await?;
        if self.task.session_id != session_id {
            return Err(CommandError::bad_request(
                "The task's session_id must match the session it is created in",
            ));
        }
        registry(ctx)
            .create(self.task)
            .await
            .map(WorkerSessionTask)
            .map_err(registry_failure)
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerUpdateSessionTask {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Task's public identifier.
    pub task_id: String,
    /// Partial update, applied through the lifecycle invariants.
    #[schema(value_type = Object)]
    pub update: SessionTaskUpdate,
}

#[command(
    name = "worker_update_session_task",
    category = "session_tasks",
    description = "Internal: apply a partial update to a session task; null when the session has no such task.",
    method = "PATCH",
    path = "/internal/sessions/{session_id}/tasks/{task_id}",
    policy = SESSION_MANAGE
)]
impl Command for WorkerUpdateSessionTask {
    type Output = Option<WorkerSessionTask>;

    async fn execute(self, ctx: &Ctx) -> Result<Option<WorkerSessionTask>, CommandError> {
        let session_id = owned_session(ctx, &self.session_id).await?;
        registry(ctx)
            .update(session_id, &self.task_id, self.update)
            .await
            .map(worker_task)
            .map_err(registry_failure)
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerGetSessionTask {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Task's public identifier.
    pub task_id: String,
}

#[command(
    name = "worker_get_session_task",
    category = "session_tasks",
    description = "Internal: read a session task; null when the session has no such task.",
    method = "GET",
    path = "/internal/sessions/{session_id}/tasks/{task_id}",
    policy = SESSION_VIEW
)]
impl Command for WorkerGetSessionTask {
    type Output = Option<WorkerSessionTask>;

    async fn execute(self, ctx: &Ctx) -> Result<Option<WorkerSessionTask>, CommandError> {
        let session_id = owned_session(ctx, &self.session_id).await?;
        registry(ctx)
            .get(session_id, &self.task_id)
            .await
            .map(worker_task)
            .map_err(registry_failure)
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerListSessionTasks {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Only tasks of this kind.
    #[serde(default)]
    pub kind: Option<String>,
    /// Only tasks in this state.
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub state: Option<SessionTaskState>,
}

#[command(
    name = "worker_list_session_tasks",
    category = "session_tasks",
    description = "Internal: list a session's tasks, optionally by kind and state.",
    method = "GET",
    path = "/internal/sessions/{session_id}/tasks",
    policy = SESSION_VIEW
)]
impl Command for WorkerListSessionTasks {
    type Output = Vec<WorkerSessionTask>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<WorkerSessionTask>, CommandError> {
        let session_id = owned_session(ctx, &self.session_id).await?;
        let filter = (self.kind.is_some() || self.state.is_some()).then_some(SessionTaskFilter {
            kind: self.kind,
            state: self.state,
        });
        let tasks = registry(ctx)
            .list(session_id, filter.as_ref())
            .await
            .map_err(registry_failure)?;
        Ok(tasks.into_iter().map(WorkerSessionTask).collect())
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerRequestCancelSessionTask {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Task's public identifier.
    pub task_id: String,
}

#[command(
    name = "worker_request_cancel_session_task",
    category = "session_tasks",
    description = "Internal: record cooperative cancel intent on a session task; null when the session has no such task.",
    method = "POST",
    path = "/internal/sessions/{session_id}/tasks/{task_id}/cancel",
    policy = SESSION_MANAGE
)]
impl Command for WorkerRequestCancelSessionTask {
    type Output = Option<WorkerSessionTask>;

    async fn execute(self, ctx: &Ctx) -> Result<Option<WorkerSessionTask>, CommandError> {
        let session_id = owned_session(ctx, &self.session_id).await?;
        registry(ctx)
            .request_cancel(session_id, &self.task_id)
            .await
            .map(worker_task)
            .map_err(registry_failure)
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerRecordSessionTaskMessage {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Task's public identifier.
    pub task_id: String,
    /// The message, with its direction and optional attempt fence.
    #[schema(value_type = Object)]
    pub message: NewTaskMessage,
}

#[command(
    name = "worker_record_session_task_message",
    category = "session_tasks",
    description = "Internal: record a message on a session task's channel.",
    method = "POST",
    path = "/internal/sessions/{session_id}/tasks/{task_id}/messages",
    policy = SESSION_MANAGE
)]
impl Command for WorkerRecordSessionTaskMessage {
    type Output = TaskMessage;

    async fn execute(self, ctx: &Ctx) -> Result<TaskMessage, CommandError> {
        let session_id = owned_session(ctx, &self.session_id).await?;
        let registry = registry(ctx);
        owned_task(&registry, session_id, &self.task_id).await?;
        registry
            .record_message(session_id, &self.task_id, self.message)
            .await
            .map_err(registry_failure)
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerListSessionTaskMessages {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Task's public identifier.
    pub task_id: String,
    /// Most messages to return.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Only messages newer than this message id.
    #[serde(default)]
    pub after_id: Option<String>,
}

#[command(
    name = "worker_list_session_task_messages",
    category = "session_tasks",
    description = "Internal: list a session task's messages, oldest first.",
    method = "GET",
    path = "/internal/sessions/{session_id}/tasks/{task_id}/messages",
    policy = SESSION_VIEW
)]
impl Command for WorkerListSessionTaskMessages {
    type Output = Vec<TaskMessage>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<TaskMessage>, CommandError> {
        let session_id = owned_session(ctx, &self.session_id).await?;
        let registry = registry(ctx);
        owned_task(&registry, session_id, &self.task_id).await?;
        registry
            .list_messages(
                session_id,
                &self.task_id,
                self.limit,
                self.after_id.as_deref(),
            )
            .await
            .map_err(registry_failure)
    }
}

#[cfg(test)]
mod tests;
