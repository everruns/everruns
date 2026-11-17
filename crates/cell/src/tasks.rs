//! A session's tasks on a cell (data plane v2 step 11.4): the subagents,
//! threads and background work it started, with their message channels.
//! They live in the owning session's cell state, next to its schedules.
//!
//! Decisions:
//! - Every write is a [`Command::Platform`] on the owner's cell, built in
//!   full by the proposer (ids and times travel in the command), and goes
//!   through the shared `apply_task_update` invariants, as on every host.
//! - A transition that wakes the owner (per the task's wake policy: a
//!   terminal state, waiting for input, an outbound message) admits the wake
//!   message in the same command, so a crash can lose neither. Its id
//!   derives from the task, attempt and transition: a retry admits once.
//! - Tasks are kept as JSON with their raw spec: `SessionTask`'s own
//!   serializer redacts webhook secrets, which would lose them in the log.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::{EventId, MessageId, SessionId};
use everruns_core::session_task::{
    CreateSessionTask, NewTaskMessage, SessionTask, SessionTaskFilter, SessionTaskRegistry,
    SessionTaskState, SessionTaskUpdate, TaskMessage, TaskMessageDirection, apply_task_update,
    generate_task_message_id, new_session_task,
};
use everruns_core::task_observer::TaskTransition;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::hosting::Port;
use crate::state::{Change, Command, Response, SessionState};

/// Namespace of task wake message ids.
const WAKE: uuid::Uuid = uuid::Uuid::from_u128(0x6576_6572_7275_6e73_7461_736b_7761_6b65);

/// A change to a session's tasks ([`crate::platform::Op::Task`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "task_op", rename_all = "snake_case")]
pub enum TaskOp {
    /// Add `task` (its JSON with the raw spec), unless its id exists.
    Create {
        task: Value,
    },
    Update {
        task_id: String,
        update: Box<SessionTaskUpdate>,
    },
    RequestCancel {
        task_id: String,
    },
    /// Add `message` to the task's channel.
    Message {
        message: Box<TaskMessage>,
        expected_attempt: Option<i32>,
    },
}

/// JSON of `task` with its raw spec.
fn stored(task: &SessionTask) -> Value {
    let mut value = json!(task);
    value["spec"] = task.spec.clone();
    value
}

fn load(value: &Value) -> Option<SessionTask> {
    serde_json::from_value(value.clone()).ok()
}

fn at(ms: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ms).unwrap_or_default()
}

fn rejected(reason: impl Into<String>) -> Response {
    Response::Rejected {
        reason: reason.into(),
    }
}

/// Apply `op` to `state`, the session that owns the tasks.
pub(crate) fn apply(
    state: &mut SessionState,
    session: SessionId,
    op: TaskOp,
    now_ms: i64,
    seed: EventId,
    changes: &mut Vec<Change>,
) -> Response {
    let now = at(now_ms);
    let tasks = &mut state.platform.tasks;
    let value = match op {
        TaskOp::Create { task } => {
            let Some(new) = load(&task) else {
                return rejected("not a task");
            };
            if new.session_id != session {
                return rejected("a task belongs to the session that creates it");
            }
            if let Some(existing) = tasks.get(&new.id) {
                return Response::Value {
                    value: existing.clone(),
                };
            }
            tasks.insert(new.id.clone(), task.clone());
            task
        }
        TaskOp::Update { task_id, update } => {
            let Some(mut task) = tasks.get(&task_id).and_then(load) else {
                return Response::Value { value: Value::Null };
            };
            if update.expected_recovery.as_ref().is_some_and(|guard| {
                !guard.matches(&task)
                    || update
                        .expected_attempt
                        .is_some_and(|attempt| attempt != task.attempt)
            }) {
                return Response::Value { value: Value::Null };
            }
            let wants_terminal = update.state.is_some_and(|s| s.is_terminal());
            let wants_input = update.input_request.is_some()
                || update.state == Some(SessionTaskState::AwaitingInput);
            let prior = task.state;
            apply_task_update(&mut task, *update, now);
            let value = stored(&task);
            tasks.insert(task_id, value.clone());
            if wants_terminal && !prior.is_terminal() && task.state.is_terminal() {
                wake(
                    state,
                    session,
                    &task,
                    TaskTransition::Terminal,
                    now_ms,
                    seed,
                    changes,
                );
            }
            if wants_input
                && prior != SessionTaskState::AwaitingInput
                && task.state == SessionTaskState::AwaitingInput
            {
                wake(
                    state,
                    session,
                    &task,
                    TaskTransition::AwaitingInput,
                    now_ms,
                    seed,
                    changes,
                );
            }
            value
        }
        TaskOp::RequestCancel { task_id } => {
            let Some(mut task) = tasks.get(&task_id).and_then(load) else {
                return Response::Value { value: Value::Null };
            };
            if !task.state.is_terminal() && task.cancel_requested_at.is_none() {
                task.cancel_requested_at = Some(now);
                task.updated_at = now;
                tasks.insert(task_id, stored(&task));
            }
            stored(&task)
        }
        TaskOp::Message {
            message,
            expected_attempt,
        } => {
            let Some(mut task) = tasks.get(&message.task_id).and_then(load) else {
                return rejected(format!("no task {}", message.task_id));
            };
            if let Some(expected) = expected_attempt
                && expected != task.attempt
            {
                return rejected(format!(
                    "stale attempt for task {}: expected {expected}, current {}",
                    task.id, task.attempt
                ));
            }
            let channel = state
                .platform
                .task_messages
                .entry(task.id.clone())
                .or_default();
            if channel.iter().any(|m| m.id == message.id) {
                return Response::Value {
                    value: json!(message),
                };
            }
            channel.push((*message).clone());
            // An inbound answer clears a matching input request and returns
            // the task to running; outbound messages never do.
            if message.direction == TaskMessageDirection::Inbound
                && let Some(reply) = &message.in_reply_to
                && task.input_request.as_ref().is_some_and(|r| &r.id == reply)
            {
                apply_task_update(
                    &mut task,
                    SessionTaskUpdate {
                        state: Some(SessionTaskState::Running),
                        ..Default::default()
                    },
                    now,
                );
                state.platform.tasks.insert(task.id.clone(), stored(&task));
            }
            if message.direction == TaskMessageDirection::Outbound {
                wake(
                    state,
                    session,
                    &task,
                    TaskTransition::Message,
                    now_ms,
                    seed,
                    changes,
                );
            }
            json!(message)
        }
    };
    changes.push(Change::Session(session));
    Response::Value { value }
}

/// Admit the wake `transition` of `task` owes its session, if its policy
/// wants one.
fn wake(
    state: &mut SessionState,
    session: SessionId,
    task: &SessionTask,
    transition: TaskTransition,
    now_ms: i64,
    seed: EventId,
    changes: &mut Vec<Change>,
) {
    use everruns::hosting::spi::session_commands::SessionCommand;
    let Some(text) = everruns_core::wake_queue::wake_text_for(task, transition) else {
        return;
    };
    let identity = format!(
        "{}:{}:{}:{:?}:{}",
        task.session_id, task.id, task.attempt, transition, task.state
    );
    let mut message = everruns_core::RuntimeMessage::user(text);
    message.id = MessageId::from_uuid(uuid::Uuid::new_v5(&WAKE, identity.as_bytes()));
    message.created_at = at(now_ms);
    message.metadata = Some(everruns_core::message::task_wake_message_metadata());
    everruns_core::causal::task_causal_context(task).write_metadata(&mut message.metadata);
    let admit = SessionCommand::Admit {
        message: Box::new(message),
        cancelled: false,
        context: None,
    };
    match state.command(session, admit, now_ms, seed) {
        Ok((_, events)) => {
            for event in events {
                changes.push(Change::Event(session, Arc::new(event)));
            }
        }
        Err(reason) => tracing::debug!(%session, task = %task.id, %reason, "task wake refused"),
    }
}

/// The session's tasks as the control plane lists them (secrets redacted),
/// oldest first.
pub(crate) fn listed(state: &SessionState) -> Value {
    let mut tasks: Vec<SessionTask> = state.platform.tasks.values().filter_map(load).collect();
    tasks.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    json!(tasks)
}

/// One task with a page of its messages (the control plane's
/// `SessionTaskDetail`), or null.
pub(crate) fn detail(
    state: &SessionState,
    task_id: &str,
    after_id: Option<&str>,
    limit: usize,
) -> Value {
    let Some(task) = state.platform.tasks.get(task_id).and_then(load) else {
        return Value::Null;
    };
    let all = state
        .platform
        .task_messages
        .get(task_id)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let start = after_id
        .and_then(|after| all.iter().position(|m| m.id == after).map(|i| i + 1))
        .unwrap_or(0);
    let page: Vec<&TaskMessage> = all.iter().skip(start).take(limit).collect();
    json!({ "task": task, "messages": page })
}

/// The session tasks of an activation, in the owning sessions' cell state.
pub(crate) struct CellTasks {
    port: Port,
}

impl CellTasks {
    pub(crate) fn new(port: Port) -> Self {
        Self { port }
    }

    async fn propose(&self, session: SessionId, op: TaskOp) -> Result<Value> {
        self.port
            .propose(
                session,
                Command::Platform {
                    session,
                    action: crate::platform::Op::Task(op),
                    now_ms: crate::node::now_ms(),
                    seed: EventId::new(),
                },
            )
            .await
    }

    fn task(value: Value) -> Result<Option<SessionTask>> {
        if value.is_null() {
            return Ok(None);
        }
        serde_json::from_value(value)
            .map(Some)
            .map_err(|e| AgentLoopError::store(e.to_string()))
    }
}

#[async_trait]
impl SessionTaskRegistry for CellTasks {
    async fn create(&self, input: CreateSessionTask) -> Result<SessionTask> {
        let session = input.session_id;
        let task = new_session_task(input, Utc::now());
        let value = self
            .propose(
                session,
                TaskOp::Create {
                    task: stored(&task),
                },
            )
            .await
            .map_err(|error| match error {
                AgentLoopError::Configuration(reason) => AgentLoopError::store(reason),
                error => error,
            })?;
        Self::task(value)?.ok_or_else(|| AgentLoopError::store("task create answered nothing"))
    }

    async fn update(
        &self,
        session_id: SessionId,
        task_id: &str,
        update: SessionTaskUpdate,
    ) -> Result<Option<SessionTask>> {
        let op = TaskOp::Update {
            task_id: task_id.to_owned(),
            update: Box::new(update),
        };
        Self::task(self.propose(session_id, op).await?)
    }

    async fn get(&self, session_id: SessionId, task_id: &str) -> Result<Option<SessionTask>> {
        let task_id = task_id.to_owned();
        self.port
            .read(session_id, move |s| {
                s.platform.tasks.get(&task_id).and_then(load)
            })
            .await
    }

    async fn list(
        &self,
        session_id: SessionId,
        filter: Option<&SessionTaskFilter>,
    ) -> Result<Vec<SessionTask>> {
        let filter = filter.cloned().unwrap_or_default();
        self.port
            .read(session_id, move |s| {
                let mut tasks: Vec<SessionTask> = s
                    .platform
                    .tasks
                    .values()
                    .filter_map(load)
                    .filter(|t| filter.kind.as_ref().is_none_or(|k| &t.kind == k))
                    .filter(|t| filter.state.is_none_or(|state| t.state == state))
                    .collect();
                tasks.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
                tasks
            })
            .await
    }

    async fn request_cancel(
        &self,
        session_id: SessionId,
        task_id: &str,
    ) -> Result<Option<SessionTask>> {
        let op = TaskOp::RequestCancel {
            task_id: task_id.to_owned(),
        };
        Self::task(self.propose(session_id, op).await?)
    }

    async fn record_message(
        &self,
        session_id: SessionId,
        task_id: &str,
        message: NewTaskMessage,
    ) -> Result<TaskMessage> {
        let record = TaskMessage {
            id: generate_task_message_id(),
            task_id: task_id.to_owned(),
            direction: message.direction,
            content: message.content,
            in_reply_to: message.in_reply_to,
            created_at: Utc::now(),
        };
        let op = TaskOp::Message {
            message: Box::new(record),
            expected_attempt: message.expected_attempt,
        };
        let value = self
            .propose(session_id, op)
            .await
            .map_err(|error| match error {
                AgentLoopError::Configuration(reason) => AgentLoopError::tool(reason),
                error => error,
            })?;
        serde_json::from_value(value).map_err(|e| AgentLoopError::store(e.to_string()))
    }

    async fn list_messages(
        &self,
        session_id: SessionId,
        task_id: &str,
        limit: Option<u32>,
        after_id: Option<&str>,
    ) -> Result<Vec<TaskMessage>> {
        let task_id = task_id.to_owned();
        let after = after_id.map(str::to_owned);
        self.port
            .read(session_id, move |s| {
                let all = s
                    .platform
                    .task_messages
                    .get(&task_id)
                    .cloned()
                    .unwrap_or_default();
                let start = after
                    .and_then(|after| all.iter().position(|m| m.id == after).map(|i| i + 1))
                    .unwrap_or(0);
                let end = limit.map_or(all.len(), |limit| (start + limit as usize).min(all.len()));
                all[start.min(all.len())..end].to_vec()
            })
            .await
    }
}

#[cfg(test)]
#[path = "tasks_tests.rs"]
mod tests;
