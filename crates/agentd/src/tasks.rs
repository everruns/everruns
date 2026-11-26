//! A session's tasks on a shard (data plane v2 step 11.4): the subagents,
//! threads and background work it started, with their message channels.
//! They live in the owning session's shard state, next to its schedules.
//!
//! Decisions:
//! - Every write is a [`Command::Platform`] on the owner's shard, built in
//!   full by the proposer (ids and times travel in the command), and goes
//!   through the shared `apply_task_update` invariants, as on every host.
//! - A transition that wakes the owner (per the task's wake policy: a
//!   terminal state, waiting for input, an outbound message) admits the wake
//!   message in the same command, so a crash can lose neither. Its id
//!   derives from the task, attempt and transition: a retry admits once.
//! - Tasks are kept as JSON with their raw spec: `SessionTask`'s own
//!   serializer redacts webhook secrets, which would lose them in the log.
//! - Push configs (parity, Phase B) live here too, beside the task they
//!   watch. An API-made signing secret is sealed with the org's data key by
//!   the edge that takes it (`gateway_v1::seal_push_secret`), so replicated
//!   state and the feed hold ciphertext only, and the control plane, which
//!   delivers the webhooks, opens it to sign. A secret given at spawn
//!   (`spec.push_configs`) is the agent's own tool input, already in the
//!   session's log, and travels as given.
//! - A transition a webhook may want goes on the feed
//!   (`Record::TaskTransition`) from the state machine, with the targets
//!   that ask for it, and the control plane delivers it: every terminal
//!   state (the org's task webhooks fire on those), and waiting for input
//!   or an outbound message only when a push config names it. The feed is
//!   the channel the control plane drains once per position, so a leader
//!   change neither loses nor repeats a delivery; a shard has no outbound
//!   webhook path of its own.

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

use everruns_contracts::dataplane::{PushTarget, Record};

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
    /// Add a push config to the task, unless its id exists.
    AddPushConfig {
        task_id: String,
        config: PushConfig,
    },
    /// Remove one of the task's push configs.
    DeletePushConfig {
        task_id: String,
        config_id: String,
    },
}

/// The push configs a task may have.
pub const MAX_PUSH_CONFIGS: usize = 20;

/// The `event_filter` members a push config may name.
pub const EVENT_FILTERS: &[&str] = &["terminal", "awaiting_input", "message"];

/// A push config made through the API. Its signing secret, when it has
/// one, is kept sealed for the control plane, which delivers the webhooks:
/// replicated state holds no plain secret.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct PushConfig {
    /// `tpc_` and 32 hex digits.
    pub id: String,
    pub url: String,
    #[serde(default)]
    pub has_secret: bool,
    /// Its signing secret, sealed with the org's data key by the edge that
    /// took it (`PushTarget::sealed`); never shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sealed_secret: Option<everruns_contracts::dataplane::SealedSecret>,
    pub event_filter: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl std::fmt::Debug for PushConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PushConfig")
            .field("id", &self.id)
            .field("url", &self.url)
            .field("has_secret", &self.has_secret)
            .field("event_filter", &self.event_filter)
            .finish()
    }
}

impl PushConfig {
    /// As the API shows it: whether it has a secret, never the secret.
    pub fn public(&self) -> Value {
        json!({
            "id": self.id,
            "url": self.url,
            "has_secret": self.has_secret,
            "event_filter": self.event_filter,
            "created_at": self.created_at,
            "updated_at": self.updated_at,
        })
    }
}

/// `filter` with duplicates dropped, terminal-only when empty; the
/// control plane's rules (`normalize_event_filter`).
pub fn event_filter(filter: Option<Vec<String>>) -> std::result::Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for member in filter.unwrap_or_default() {
        if !EVENT_FILTERS.contains(&member.as_str()) {
            return Err(format!(
                "Unknown event_filter \"{member}\". Valid values: {}.",
                EVENT_FILTERS.join(", ")
            ));
        }
        if !out.contains(&member) {
            out.push(member);
        }
    }
    if out.is_empty() {
        out.push("terminal".into());
    }
    Ok(out)
}

/// The targets of `task`'s push configs, made through the API (`stored`)
/// or given at spawn (`spec.push_configs`), that ask for `transition`. A
/// spawn-time config with no filter asks for terminal states only.
fn targets(
    stored: &[PushConfig],
    task: &SessionTask,
    transition: TaskTransition,
) -> Vec<PushTarget> {
    let event = transition.filter_value();
    let mut out: Vec<PushTarget> = stored
        .iter()
        .filter(|c| c.event_filter.iter().any(|f| f == event))
        // A config whose secret was held by the control plane before
        // secrets travelled sealed has none here: never sent unsigned.
        .filter(|c| !c.has_secret || c.sealed_secret.is_some())
        .map(|c| PushTarget {
            label: c.id.clone(),
            url: c.url.clone(),
            secret: None,
            sealed: c.sealed_secret.clone(),
        })
        .collect();
    for entry in task
        .spec
        .get("push_configs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(url) = entry.get("url").and_then(Value::as_str) else {
            continue;
        };
        let wanted = match entry.get("event_filter").and_then(Value::as_array) {
            Some(filters) => filters.iter().any(|f| f.as_str() == Some(event)),
            None => transition == TaskTransition::Terminal,
        };
        if wanted {
            out.push(PushTarget {
                label: "spec".into(),
                url: url.to_owned(),
                secret: entry
                    .get("secret")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                sealed: None,
            });
        }
    }
    out
}

/// The feed record `transition` of `task` owes the control plane, if any.
fn notice(state: &SessionState, task: &SessionTask, transition: TaskTransition) -> Option<Record> {
    let stored = state
        .platform
        .task_push_configs
        .get(&task.id)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let targets = targets(stored, task, transition);
    if transition != TaskTransition::Terminal && targets.is_empty() {
        return None;
    }
    Some(Record::TaskTransition {
        session_id: task.session_id.to_string(),
        transition: transition.filter_value().into(),
        task: json!({
            "id": task.id,
            "display_name": task.display_name,
            "kind": task.kind,
            "state": task.state.to_string(),
            "session_id": task.session_id,
            "summary": task.summary,
            "result_path": task.result_path,
        }),
        targets,
    })
}

/// JSON of `task` with its raw spec.
fn stored(task: &SessionTask) -> Value {
    let mut value = json!(task);
    value["spec"] = task.spec.clone();
    value
}

fn gateway_err((status, reason): (axum::http::StatusCode, String)) -> AgentLoopError {
    if status == axum::http::StatusCode::NOT_FOUND {
        AgentLoopError::store(format!("not found: {reason}"))
    } else {
        AgentLoopError::store(format!("{status}: {reason}"))
    }
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

/// Apply `op` to `state`, the session that owns the tasks. The feed
/// records its transitions owe the control plane go to `notices`.
pub(crate) fn apply(
    state: &mut SessionState,
    session: SessionId,
    op: TaskOp,
    now_ms: i64,
    seed: EventId,
    changes: &mut Vec<Change>,
    notices: &mut Vec<Record>,
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
                notices.extend(notice(state, &task, TaskTransition::Terminal));
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
                notices.extend(notice(state, &task, TaskTransition::AwaitingInput));
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
                notices.extend(notice(state, &task, TaskTransition::Message));
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
        TaskOp::AddPushConfig { task_id, config } => {
            if !tasks.contains_key(&task_id) {
                return Response::Value { value: Value::Null };
            }
            let configs = state.platform.task_push_configs.entry(task_id).or_default();
            if let Some(existing) = configs.iter().find(|c| c.id == config.id) {
                return Response::Value {
                    value: existing.public(),
                };
            }
            if configs.len() >= MAX_PUSH_CONFIGS {
                return rejected(format!(
                    "a task takes at most {MAX_PUSH_CONFIGS} push configs"
                ));
            }
            let value = config.public();
            configs.push(config);
            value
        }
        TaskOp::DeletePushConfig { task_id, config_id } => {
            if !tasks.contains_key(&task_id) {
                return Response::Value { value: Value::Null };
            }
            let configs = state
                .platform
                .task_push_configs
                .get_mut(&task_id)
                .map(std::mem::take)
                .unwrap_or_default();
            let before = configs.len();
            let kept: Vec<PushConfig> = configs.into_iter().filter(|c| c.id != config_id).collect();
            let deleted = kept.len() < before;
            if !kept.is_empty() {
                state.platform.task_push_configs.insert(task_id, kept);
            } else {
                state.platform.task_push_configs.remove(&task_id);
            }
            json!({ "deleted": deleted })
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
    state.platform.senders.inherit(message.id);
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

/// A person's write to one of a session's tasks through the `/v1` API
/// (step 11.6): a cancel, or a message to the task.
#[derive(Debug, Deserialize)]
pub struct ApiWrite {
    pub task: String,
    #[serde(default)]
    pub cancel: bool,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub content: Option<Vec<everruns_core::session_task::TaskMessagePart>>,
    #[serde(default)]
    pub in_reply_to: Option<String>,
}

/// The kind of task `task` of the session, if it has one.
pub(crate) fn kind_of(state: &SessionState, task: &str) -> Option<String> {
    state
        .platform
        .tasks
        .get(task)
        .and_then(load)
        .map(|t| t.kind)
}

/// The change `write` makes to a task of `kind`, or why it is refused,
/// with the control plane's rules: a subagent or handoff is steered by its
/// parent agent and an assignment through its thread, and a message needs
/// content. A person's message applies whatever the task's attempt.
pub fn api_op(write: ApiWrite, kind: &str) -> std::result::Result<TaskOp, String> {
    use everruns_core::session_task::{
        TASK_KIND_AGENT_HANDOFF, TASK_KIND_ASSIGNMENT, TASK_KIND_SUBAGENT, TaskMessagePart,
    };
    if write.cancel {
        return Ok(TaskOp::RequestCancel {
            task_id: write.task,
        });
    }
    if kind == TASK_KIND_SUBAGENT || kind == TASK_KIND_AGENT_HANDOFF {
        return Err(
            "Subagent and agent-handoff tasks are steered by their parent agent via the \
                    message_task tool; HTTP message delivery is not supported for this task kind."
                .into(),
        );
    }
    if kind == TASK_KIND_ASSIGNMENT {
        return Err(
            "Assignments belong to a thread: send the message to the thread session \
                    itself, or ask the coordinator to relay it."
                .into(),
        );
    }
    let content = match (write.content, write.text) {
        (Some(content), _) if !content.is_empty() => content,
        (_, Some(text)) if !text.trim().is_empty() => vec![TaskMessagePart::text(text)],
        _ => return Err("Message requires non-empty `content` parts or `text`".into()),
    };
    let message = TaskMessage {
        id: generate_task_message_id(),
        task_id: write.task,
        direction: TaskMessageDirection::Inbound,
        content,
        in_reply_to: write.in_reply_to.filter(|s| !s.is_empty()),
        created_at: Utc::now(),
    };
    Ok(TaskOp::Message {
        message: Box::new(message),
        expected_attempt: None,
    })
}

/// The session tasks of an activation, in the owning sessions' shard state.
pub(crate) struct ShardTasks {
    port: Port,
    /// The activation's parent, when it is a child (a thread or a spawned
    /// agent): the one other session whose tasks it may reach on another
    /// shard, through the gateway. Nothing else leaves this shard.
    parent: Option<SessionId>,
}

impl ShardTasks {
    pub(crate) fn new(port: Port) -> Self {
        Self { port, parent: None }
    }

    pub(crate) fn with_parent(mut self, parent: Option<SessionId>) -> Self {
        self.parent = parent;
        self
    }

    /// Whether `session`'s tasks are reached through the gateway: the
    /// parent's always are. An executor's mirror holds only the sessions it
    /// runs, and a node holds the parent only when it shares its shard, so
    /// the gateway is the one way that works from everywhere.
    fn remote(&self, session: SessionId) -> bool {
        self.parent == Some(session)
    }

    async fn propose(&self, session: SessionId, op: TaskOp) -> Result<Value> {
        let command = |op: TaskOp| Command::Platform {
            session,
            action: crate::platform::Op::Task(op),
            now_ms: crate::node::now_ms(),
            seed: EventId::new(),
        };
        if self.remote(session) {
            return self.propose_elsewhere(session, command(op)).await;
        }
        self.port.propose(session, command(op)).await
    }

    /// Propose `command` on the shard that holds `session`, through the
    /// gateway, following moves: a thread on one shard finishing the
    /// assignment its parent holds on another.
    async fn propose_elsewhere(&self, session: SessionId, command: Command) -> Result<Value> {
        let gateway = self.gateway(session)?;
        for _ in 0..5 {
            let shard = gateway.locate(session).await.map_err(gateway_err)?;
            match gateway
                .propose(shard, &command)
                .await
                .map_err(gateway_err)?
            {
                Response::Value { value } => return Ok(value),
                Response::Moved { .. } => gateway.forget(session),
                Response::Frozen => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
                Response::NotFound => return Err(AgentLoopError::SessionNotFound(session)),
                Response::Rejected { reason } => return Err(AgentLoopError::config(reason)),
                other => {
                    return Err(AgentLoopError::store(format!("unexpected reply {other:?}")));
                }
            }
        }
        Err(AgentLoopError::store(format!(
            "session {session} keeps moving"
        )))
    }

    /// `session`'s tasks (`task` none) or one task's detail, from the shard
    /// that holds it, through the gateway.
    async fn read_elsewhere(&self, session: SessionId, body: Value) -> Result<Value> {
        self.gateway(session)?
            .session_command(session, "tasks", &body)
            .await
            .map_err(gateway_err)
    }

    fn gateway(&self, session: SessionId) -> Result<Arc<crate::gateway::Gateway>> {
        self.port
            .gateway()
            .ok_or(AgentLoopError::SessionNotFound(session))
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
impl SessionTaskRegistry for ShardTasks {
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
        if self.remote(session_id) {
            let detail = self
                .read_elsewhere(session_id, json!({ "task": task_id }))
                .await?;
            return Ok(load(&detail["task"]));
        }
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
        let keep = move |t: &SessionTask| {
            filter.kind.as_ref().is_none_or(|k| &t.kind == k)
                && filter.state.is_none_or(|state| t.state == state)
        };
        if self.remote(session_id) {
            // Already in the shard's order (`listed`).
            let listed = self.read_elsewhere(session_id, json!({})).await?;
            return Ok(listed
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .filter_map(load)
                .filter(|t| keep(t))
                .collect());
        }
        self.port
            .read(session_id, move |s| {
                let mut tasks: Vec<SessionTask> = s
                    .platform
                    .tasks
                    .values()
                    .filter_map(load)
                    .filter(|t| keep(t))
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
        if self.remote(session_id) {
            let body = json!({
                "task": task_id,
                "after_id": after_id,
                "limit": limit.map_or(500, |l| l.clamp(1, 500)),
            });
            let detail = self.read_elsewhere(session_id, body).await?;
            return Ok(serde_json::from_value(detail["messages"].clone()).unwrap_or_default());
        }
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
