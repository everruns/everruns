// Session tasks — unified registry of background work owned by a session.
//
// See knowledge/runtime-resources/session-tasks.md. A task is any asynchronous work a session owns
// (subagent, external A2A agent, background tool, monitor). The registry owns
// the record, lifecycle invariants, and task.* events; capabilities plug in
// `TaskExecutor`s (control plane) and report through `TaskSink` (report plane).
//
// Decision: lifecycle invariants live in `apply_task_update` so every backend
// (PostgreSQL, in-memory, gRPC) applies identical semantics.
// Decision: kind is a free-form string for extensibility — no enum.
// Decision: cancellation is cooperative — `request_cancel` records intent via
// `cancel_requested_at`; executors wind down and report the terminal state.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;
use std::sync::Arc;

use crate::error::Result;
use crate::typed_id::SessionId;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Progress shape shared with background tool execution.
pub type TaskProgress = crate::background::BackgroundProgress;

/// Well-known task kinds. Kind stays a free-form string; these constants
/// cover the built-in executors.
pub const TASK_KIND_SUBAGENT: &str = "subagent";
/// Detached peer session. Canceling this task cooperatively cancels the peer
/// session (standard send/cancel path) and settles the tracking task
/// `canceled` — cancel means cancel, not detach-only (EVE-766).
pub const TASK_KIND_SESSION: &str = "session";
/// Cross-agent handoff to a different configured Agent in the same harness.
/// Distinct from `subagent` so `list_tasks(kind="subagent")` returns only
/// same-agent subagents and not handoffs (they share the spawn shape but are a
/// different target). Matches the historical `session_resources.kind`.
pub const TASK_KIND_AGENT_HANDOFF: &str = "agent_handoff";
pub const TASK_KIND_EXTERNAL_AGENT: &str = "external_agent";
pub const TASK_KIND_BACKGROUND_TOOL: &str = "background_tool";
/// Long-lived monitor task linked to a session schedule. Stays `running`
/// until the linked schedule is exhausted (one-shot) or `cancel_task` is called.
pub const TASK_KIND_MONITOR: &str = "monitor";

/// Generate a new task ID (`task_` prefix per knowledge/foundations/id-schema.md).
pub fn generate_task_id() -> String {
    format!("task_{}", uuid::Uuid::now_v7().simple())
}

/// Generate a new task message ID.
pub fn generate_task_message_id() -> String {
    format!("tmsg_{}", uuid::Uuid::now_v7().simple())
}

/// Lifecycle state of a session task.
///
/// Three classes: active (`queued`, `running`), interrupted (`awaiting_input`,
/// resumable), terminal (`succeeded`, `failed`, `canceled`). Timeout and
/// rejection are `error.kind` values on `failed`, not states.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SessionTaskState {
    Queued,
    Running,
    AwaitingInput,
    Succeeded,
    Failed,
    Canceled,
}

impl SessionTaskState {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Canceled)
    }

    /// Strict parser for caller-supplied state strings (API filters, tool
    /// arguments). Unlike `From<&str>` — which exists for trusted,
    /// CHECK-constrained storage values and defaults to `Queued` — this
    /// returns None for unknown input so callers can reject it.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "queued" => Some(Self::Queued),
            "running" => Some(Self::Running),
            "awaiting_input" => Some(Self::AwaitingInput),
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            "canceled" => Some(Self::Canceled),
            _ => None,
        }
    }
}

impl std::fmt::Display for SessionTaskState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::AwaitingInput => "awaiting_input",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
        };
        write!(f, "{s}")
    }
}

impl From<&str> for SessionTaskState {
    fn from(s: &str) -> Self {
        match s {
            "running" => Self::Running,
            "awaiting_input" => Self::AwaitingInput,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "canceled" => Self::Canceled,
            _ => Self::Queued,
        }
    }
}

/// When outbound task activity wakes the owning session's agent.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TaskWakePolicy {
    /// Never wake; the agent polls via `get_task`/`list_tasks`.
    #[default]
    Silent,
    /// Wake on transition to a terminal state.
    OnTerminal,
    /// Wake on any outbound message or input request, and on terminal states.
    OnActivity,
}

/// Structured ask posted by a task that needs input to continue.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TaskInputRequest {
    /// Stable ID referenced by the answering message's `in_reply_to`.
    pub id: String,
    /// Human/agent-readable prompt.
    pub prompt: String,
    /// Optional machine-readable description of the expected answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub expected: Option<Value>,
}

/// Terminal error detail. Timeout/rejection/orphaned are kinds, not states.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TaskError {
    pub kind: String,
    pub message: String,
}

/// Typed link to something the task produced.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TaskArtifact {
    pub name: String,
    /// Artifact type: "file", "url", "session", "pr", etc.
    #[serde(rename = "type")]
    pub artifact_type: String,
    /// Session VFS path, when the artifact lives in the session filesystem.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// External URL, when the artifact lives elsewhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Cross-references owned by a task.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TaskLinks {
    /// Child session, for subagent-shaped tasks. Full transcript lives there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub child_session_id: Option<SessionId>,
    /// Remote task ID, for tasks wrapping an external protocol task (A2A).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_task_id: Option<String>,
    /// Session resources (sandboxes, browser sessions) this task holds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resource_ids: Vec<String>,
}

impl TaskLinks {
    pub fn is_empty(&self) -> bool {
        self.child_session_id.is_none()
            && self.remote_task_id.is_none()
            && self.resource_ids.is_empty()
    }
}

/// A unit of background work owned by a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SessionTask {
    /// `task_*` public ID.
    pub id: String,
    /// Owning session.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub session_id: SessionId,
    /// Root of the owning session's delegation tree (EVE-680). Populated on
    /// API reads from the denormalized storage column so cross-session tooling
    /// (e.g. the Work view) can group a whole tree's tasks by one id. `None`
    /// for a top-level session that is its own root, or when unavailable.
    /// Storage-derived, never client-settable on create.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub root_session_id: Option<SessionId>,
    /// Task kind: "subagent", "external_agent", "background_tool", "monitor", …
    pub kind: String,
    /// Human-readable label.
    pub display_name: String,
    /// Kind-specific input (instructions, tool args, external agent id).
    #[serde(default, serialize_with = "serialize_public_task_spec")]
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub spec: Value,
    pub state: SessionTaskState,
    /// Short live status line ("polling remote task", "iteration 4/10").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<TaskProgress>,
    /// Pending ask while `awaiting_input`; cleared when answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_request: Option<TaskInputRequest>,
    /// Cooperative cancel intent. A flag, not a state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel_requested_at: Option<DateTime<Utc>>,
    /// Human-readable outcome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Machine result in the session VFS: `/.tasks/{task_id}/result.json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_path: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<TaskArtifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<TaskError>,
    /// Execution attempt, starting at 1. Incremented on re-attach.
    #[serde(default = "default_attempt")]
    pub attempt: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heartbeat_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "TaskLinks::is_empty")]
    pub links: TaskLinks,
    #[serde(default)]
    pub wake_policy: TaskWakePolicy,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

fn default_attempt() -> i32 {
    1
}

fn serialize_public_task_spec<S>(
    spec: &Value,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error>
where
    S: Serializer,
{
    redacted_public_task_spec(spec).serialize(serializer)
}

fn redacted_public_task_spec(spec: &Value) -> Value {
    let mut public = spec.clone();
    let Some(configs) = public.get_mut("push_configs").and_then(Value::as_array_mut) else {
        return public;
    };
    for config in configs {
        let Some(config) = config.as_object_mut() else {
            continue;
        };
        if config.remove("secret").is_some() {
            config.insert("has_secret".to_string(), Value::Bool(true));
        }
    }
    public
}

/// Input for creating a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionTask {
    pub session_id: SessionId,
    /// Caller-supplied ID for idempotent creation; generated when None.
    #[serde(default)]
    pub id: Option<String>,
    pub kind: String,
    pub display_name: String,
    #[serde(default)]
    pub spec: Value,
    /// Initial state; defaults to Queued.
    #[serde(default = "default_queued")]
    pub state: SessionTaskState,
    #[serde(default)]
    pub links: TaskLinks,
    #[serde(default)]
    pub wake_policy: TaskWakePolicy,
}

fn default_queued() -> SessionTaskState {
    SessionTaskState::Queued
}

/// Partial update applied through `apply_task_update`. None = unchanged.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionTaskUpdate {
    pub state: Option<SessionTaskState>,
    pub state_detail: Option<String>,
    pub progress: Option<TaskProgress>,
    /// Setting an input request implies `awaiting_input`.
    pub input_request: Option<TaskInputRequest>,
    pub summary: Option<String>,
    pub result_path: Option<String>,
    /// Replaces the artifact list when set.
    pub artifacts: Option<Vec<TaskArtifact>>,
    pub error: Option<TaskError>,
    /// Merged field-by-field into existing links.
    pub links: Option<TaskLinks>,
    pub worker_id: Option<String>,
    /// Liveness heartbeat timestamp.
    pub heartbeat_at: Option<DateTime<Utc>>,
    /// Stale-attempt fence: when set, the update is silently ignored if
    /// `task.attempt != expected_attempt`. Executors and sinks set this to
    /// the attempt they captured at start; the reaper bumps `attempt` (via
    /// `increment_attempt`) when it fails an orphan, so a zombie executor's
    /// later writes are rejected. Writers that do not track attempts
    /// (e.g. `cancel_task` from the API) leave this None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_attempt: Option<i32>,
    /// Supersede the current attempt: bumps `task.attempt` so writes fenced
    /// on the previous attempt are rejected from now on. Set by the reaper
    /// when it fails an orphaned task. Ignored if the update itself is
    /// dropped by the fence or the terminal-state invariant.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub increment_attempt: bool,
}

/// Optional filter for listing tasks.
#[derive(Debug, Clone, Default)]
pub struct SessionTaskFilter {
    pub kind: Option<String>,
    pub state: Option<SessionTaskState>,
}

/// Apply a partial update to a task, enforcing lifecycle invariants.
///
/// All registry backends route updates through this function so semantics
/// stay identical across PostgreSQL, in-memory, and gRPC modes:
/// - terminal states are final: state changes on a terminal task are ignored
///   (content fields like summary/result still apply);
/// - first transition out of `queued` stamps `started_at`;
/// - transition into a terminal state stamps `finished_at`;
/// - setting `input_request` forces `awaiting_input`; leaving
///   `awaiting_input` clears it.
pub fn apply_task_update(task: &mut SessionTask, update: SessionTaskUpdate, now: DateTime<Utc>) {
    // Stale-attempt fence: if the update carries an attempt expectation and it
    // does not match the current attempt, this write came from a superseded
    // executor — ignore it entirely (heartbeats, state changes, everything).
    if let Some(expected) = update.expected_attempt
        && expected != task.attempt
    {
        return;
    }

    let was_terminal = task.state.is_terminal();

    // Terminal states are final. An update that asks for a *different* state
    // on an already-terminal task lost a race (e.g. the reaper marking a task
    // orphaned after it succeeded) — ignore it entirely so its content fields
    // (error, summary) cannot corrupt the terminal record. Updates that carry
    // the same terminal state (idempotent re-mirrors) or no state at all
    // (content enrichment) still apply below.
    if was_terminal
        && let Some(state) = update.state
        && state != task.state
    {
        return;
    }

    // Supersede the current attempt (reaper failing an orphan): writes fenced
    // on the old attempt are rejected from here on.
    if update.increment_attempt {
        task.attempt += 1;
    }

    let mut next_state = update.state;
    if update.input_request.is_some() && !was_terminal {
        next_state = Some(SessionTaskState::AwaitingInput);
    }

    if let Some(input_request) = update.input_request
        && !was_terminal
    {
        task.input_request = Some(input_request);
    }

    if let Some(state) = next_state
        && !was_terminal
        && task.state != state
    {
        if task.state == SessionTaskState::Queued && state != SessionTaskState::Queued {
            task.started_at.get_or_insert(now);
        }
        if state.is_terminal() {
            task.finished_at.get_or_insert(now);
        }
        if state != SessionTaskState::AwaitingInput {
            task.input_request = None;
        }
        task.state = state;
    }

    if let Some(detail) = update.state_detail {
        task.state_detail = Some(detail);
    }
    if let Some(progress) = update.progress {
        task.progress = Some(progress);
    }
    if let Some(summary) = update.summary {
        task.summary = Some(summary);
    }
    if let Some(result_path) = update.result_path {
        task.result_path = Some(result_path);
    }
    if let Some(artifacts) = update.artifacts {
        task.artifacts = artifacts;
    }
    if let Some(error) = update.error {
        task.error = Some(error);
    }
    if let Some(links) = update.links {
        if links.child_session_id.is_some() {
            task.links.child_session_id = links.child_session_id;
        }
        if links.remote_task_id.is_some() {
            task.links.remote_task_id = links.remote_task_id;
        }
        for id in links.resource_ids {
            if !task.links.resource_ids.contains(&id) {
                task.links.resource_ids.push(id);
            }
        }
    }
    if let Some(worker_id) = update.worker_id {
        task.worker_id = Some(worker_id);
    }
    if let Some(heartbeat_at) = update.heartbeat_at {
        task.heartbeat_at = Some(heartbeat_at);
    }

    task.updated_at = now;
}

/// Build a new task from creation input.
pub fn new_session_task(input: CreateSessionTask, now: DateTime<Utc>) -> SessionTask {
    let state = input.state;
    SessionTask {
        id: input.id.unwrap_or_else(generate_task_id),
        session_id: input.session_id,
        // Denormalized at storage insert from the owning session's root; a
        // freshly-built task carries no root until read back.
        root_session_id: None,
        kind: input.kind,
        display_name: input.display_name,
        spec: input.spec,
        state,
        state_detail: None,
        progress: None,
        input_request: None,
        cancel_requested_at: None,
        summary: None,
        result_path: None,
        artifacts: Vec::new(),
        error: None,
        attempt: 1,
        worker_id: None,
        heartbeat_at: None,
        links: input.links,
        wake_policy: input.wake_policy,
        created_at: now,
        started_at: if state == SessionTaskState::Queued {
            None
        } else {
            Some(now)
        },
        finished_at: if state.is_terminal() { Some(now) } else { None },
        updated_at: now,
    }
}

// ============================================================================
// Messages — bidirectional, persisted channel between session and task
// ============================================================================

/// Direction of a task message. Inbound = session → task.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TaskMessageDirection {
    Inbound,
    Outbound,
}

impl std::fmt::Display for TaskMessageDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Inbound => write!(f, "inbound"),
            Self::Outbound => write!(f, "outbound"),
        }
    }
}

impl From<&str> for TaskMessageDirection {
    fn from(s: &str) -> Self {
        match s {
            "outbound" => Self::Outbound,
            _ => Self::Inbound,
        }
    }
}

/// One content part of a task message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TaskMessagePart {
    Text {
        text: String,
    },
    Data {
        #[cfg_attr(feature = "openapi", schema(value_type = Object))]
        data: Value,
    },
}

impl TaskMessagePart {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }
}

/// A message exchanged between a session and one of its tasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TaskMessage {
    /// `tmsg_*` public ID.
    pub id: String,
    pub task_id: String,
    pub direction: TaskMessageDirection,
    pub content: Vec<TaskMessagePart>,
    /// Set when this message answers a `TaskInputRequest`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_reply_to: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Input for recording a task message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewTaskMessage {
    pub direction: TaskMessageDirection,
    pub content: Vec<TaskMessagePart>,
    #[serde(default)]
    pub in_reply_to: Option<String>,
    /// Stale-attempt fence for message writes: when set, registries reject
    /// the message if `task.attempt` no longer matches, so a superseded
    /// executor cannot append to the thread or trigger wake-ups. Not stored
    /// with the message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_attempt: Option<i32>,
}

impl NewTaskMessage {
    pub fn inbound_text(text: impl Into<String>) -> Self {
        Self {
            direction: TaskMessageDirection::Inbound,
            content: vec![TaskMessagePart::text(text)],
            in_reply_to: None,
            expected_attempt: None,
        }
    }

    pub fn outbound_text(text: impl Into<String>) -> Self {
        Self {
            direction: TaskMessageDirection::Outbound,
            content: vec![TaskMessagePart::text(text)],
            in_reply_to: None,
            expected_attempt: None,
        }
    }

    /// Fence this message write on the given attempt (see `expected_attempt`).
    pub fn with_expected_attempt(mut self, attempt: i32) -> Self {
        self.expected_attempt = Some(attempt);
        self
    }
}

/// Plain-text rendering of message content (for steering/wake messages).
pub fn task_message_text(content: &[TaskMessagePart]) -> String {
    content
        .iter()
        .filter_map(|part| match part {
            TaskMessagePart::Text { text } => Some(text.as_str()),
            TaskMessagePart::Data { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ============================================================================
// Registry — owns the record, invariants, events, and durability
// ============================================================================

/// Session task registry. Implementations emit `task.created` /
/// `task.updated` (full snapshots) and `task.message.*` events on the owning
/// session's event stream.
#[async_trait]
pub trait SessionTaskRegistry: Send + Sync {
    /// Create a task (idempotent on caller-supplied ID: re-creating an
    /// existing ID returns the stored task unchanged).
    async fn create(&self, input: CreateSessionTask) -> Result<SessionTask>;

    /// Apply a partial update through `apply_task_update` invariants.
    async fn update(
        &self,
        session_id: SessionId,
        task_id: &str,
        update: SessionTaskUpdate,
    ) -> Result<Option<SessionTask>>;

    async fn get(&self, session_id: SessionId, task_id: &str) -> Result<Option<SessionTask>>;

    async fn list(
        &self,
        session_id: SessionId,
        filter: Option<&SessionTaskFilter>,
    ) -> Result<Vec<SessionTask>>;

    /// Record cooperative cancel intent (idempotent). Does not change state;
    /// the executor winds down and reports the terminal state.
    async fn request_cancel(
        &self,
        session_id: SessionId,
        task_id: &str,
    ) -> Result<Option<SessionTask>>;

    /// Persist a message on the task's channel. Answering messages
    /// (`in_reply_to` set) clear a matching pending input request and return
    /// the task to `running`.
    async fn record_message(
        &self,
        session_id: SessionId,
        task_id: &str,
        message: NewTaskMessage,
    ) -> Result<TaskMessage>;

    /// List messages on the task's channel, oldest first.
    ///
    /// When `after_id` is `Some`, only messages newer than that message ID are
    /// returned (exclusive cursor, since_id-style). Both postgres and in-memory
    /// backends implement the cursor; other backends ignore it and return all
    /// messages up to `limit`.
    async fn list_messages(
        &self,
        session_id: SessionId,
        task_id: &str,
        limit: Option<u32>,
        after_id: Option<&str>,
    ) -> Result<Vec<TaskMessage>>;
}

// ============================================================================
// Executor — control plane, implemented per kind by capabilities
// ============================================================================

/// Control plane for a task kind. The registry/tools call into the executor;
/// the running work pushes into a `TaskSink`.
///
/// Default method bodies return `unsupported` so kinds implement only what
/// applies (e.g. a background tool rarely accepts inbound messages).
#[async_trait]
pub trait TaskExecutor: Send + Sync {
    fn kind(&self) -> &str;

    /// Whether this executor can re-attach to a running task after worker loss.
    ///
    /// Kinds returning `true` must implement `start` such that calling it with
    /// a re-attached task snapshot (attempt already bumped by the reaper)
    /// resumes the work idempotently and heartbeats with the new attempt.
    /// Kinds returning `false` (the default) are failed as orphaned immediately
    /// by the reaper.
    fn can_reattach(&self) -> bool {
        false
    }

    /// Whether this executor can re-attach to a *specific* task instance.
    ///
    /// Defaults to `self.can_reattach()`. Override to inspect per-task spec
    /// fields (e.g. whether the spawned tool declared itself idempotent).
    /// The reaper calls this instead of `can_reattach()` when a task snapshot
    /// is available.
    fn can_reattach_task(&self, task: &SessionTask) -> bool {
        let _ = task;
        self.can_reattach()
    }

    /// Begin execution, or re-attach after worker loss.
    ///
    /// Called by the reaper when re-attaching a task (attempt already bumped).
    /// Implementations must heartbeat using `task.attempt` so stale writes from
    /// the previous executor are rejected by the fence.
    async fn start(&self, task: &SessionTask, context: &crate::traits::ToolContext) -> Result<()> {
        let _ = (task, context);
        Err(crate::error::AgentLoopError::tool(format!(
            "task kind '{}' does not support start via the registry",
            self.kind()
        )))
    }

    /// Deliver an inbound message (steering or input answer) to the work.
    async fn deliver(
        &self,
        task: &SessionTask,
        message: &TaskMessage,
        context: &crate::traits::ToolContext,
    ) -> Result<()> {
        let _ = (task, message, context);
        Err(crate::error::AgentLoopError::tool(format!(
            "task kind '{}' does not accept inbound messages",
            self.kind()
        )))
    }

    /// Cooperatively wind down. The task may still end succeeded or failed.
    async fn cancel(&self, task: &SessionTask, context: &crate::traits::ToolContext) -> Result<()>;

    /// Refresh state for polled kinds (e.g. A2A remote tasks). Reports via
    /// the registry; no-op by default.
    async fn reconcile(
        &self,
        task: &SessionTask,
        context: &crate::traits::ToolContext,
    ) -> Result<()> {
        let _ = (task, context);
        Ok(())
    }
}

/// Inventory plugin so capabilities register executors without core knowing
/// about them (same pattern as `everruns-platform`'s
/// `SessionSandboxProviderPlugin`).
pub struct TaskExecutorPlugin {
    pub executor: fn() -> Arc<dyn TaskExecutor>,
}

inventory::collect!(TaskExecutorPlugin);

/// Find the registered executor for a task kind.
pub fn find_task_executor(kind: &str) -> Option<Arc<dyn TaskExecutor>> {
    inventory::iter::<TaskExecutorPlugin>
        .into_iter()
        .map(|plugin| (plugin.executor)())
        .find(|executor| executor.kind() == kind)
}

// ============================================================================
// Sink — report plane for running work
// ============================================================================

/// Report plane handed to running work. `state`/`progress`/`request_input`
/// mutate the task record (snapshot events fire); `post` appends to the
/// message channel; `output` is high-frequency and ephemeral.
#[async_trait]
pub trait TaskSink: Send + Sync {
    async fn state(&self, state: SessionTaskState, detail: Option<String>) -> Result<()>;

    async fn progress(&self, progress: TaskProgress) -> Result<()>;

    /// High-frequency output delta. Not persisted on the task record.
    async fn output(&self, stream: &str, delta: &str) -> Result<()>;

    /// Outbound message to the session; may wake the parent per wake policy.
    async fn post(&self, message: NewTaskMessage) -> Result<()>;

    /// Ask the session for input; transitions the task to `awaiting_input`.
    async fn request_input(&self, request: TaskInputRequest) -> Result<()>;

    async fn artifact(&self, artifact: TaskArtifact) -> Result<()>;
}

/// `TaskSink` backed by a `SessionTaskRegistry`. Output deltas are dropped
/// here; kinds with live output keep their existing streaming path.
///
/// Carries `attempt` for stale-attempt fencing: every update includes
/// `expected_attempt` so writes from a superseded executor are rejected once
/// the reaper increments the attempt counter on the task record.
pub struct RegistryTaskSink {
    registry: Arc<dyn SessionTaskRegistry>,
    session_id: SessionId,
    task_id: String,
    /// The attempt number this sink was created for (captured at task start).
    attempt: i32,
}

impl RegistryTaskSink {
    pub fn new(
        registry: Arc<dyn SessionTaskRegistry>,
        session_id: SessionId,
        task_id: String,
    ) -> Self {
        Self {
            registry,
            session_id,
            task_id,
            attempt: 1,
        }
    }

    /// Set the attempt number for fencing. Call this after reading the task
    /// record at start so the sink rejects writes once the attempt is bumped.
    pub fn with_attempt(mut self, attempt: i32) -> Self {
        self.attempt = attempt;
        self
    }
}

#[async_trait]
impl TaskSink for RegistryTaskSink {
    async fn state(&self, state: SessionTaskState, detail: Option<String>) -> Result<()> {
        self.registry
            .update(
                self.session_id,
                &self.task_id,
                SessionTaskUpdate {
                    state: Some(state),
                    state_detail: detail,
                    expected_attempt: Some(self.attempt),
                    ..Default::default()
                },
            )
            .await?;
        Ok(())
    }

    async fn progress(&self, progress: TaskProgress) -> Result<()> {
        self.registry
            .update(
                self.session_id,
                &self.task_id,
                SessionTaskUpdate {
                    progress: Some(progress),
                    expected_attempt: Some(self.attempt),
                    ..Default::default()
                },
            )
            .await?;
        Ok(())
    }

    async fn output(&self, _stream: &str, _delta: &str) -> Result<()> {
        Ok(())
    }

    async fn post(&self, message: NewTaskMessage) -> Result<()> {
        // Fence message writes too: record_message emits events and can wake
        // the parent session, so a superseded executor must not post.
        self.registry
            .record_message(
                self.session_id,
                &self.task_id,
                message.with_expected_attempt(self.attempt),
            )
            .await?;
        Ok(())
    }

    async fn request_input(&self, request: TaskInputRequest) -> Result<()> {
        self.registry
            .update(
                self.session_id,
                &self.task_id,
                SessionTaskUpdate {
                    input_request: Some(request),
                    expected_attempt: Some(self.attempt),
                    ..Default::default()
                },
            )
            .await?;
        Ok(())
    }

    async fn artifact(&self, artifact: TaskArtifact) -> Result<()> {
        let Some(task) = self.registry.get(self.session_id, &self.task_id).await? else {
            return Ok(());
        };
        // Check attempt before fetching artifacts to avoid a stale write.
        if task.attempt != self.attempt {
            return Ok(());
        }
        let mut artifacts = task.artifacts;
        artifacts.push(artifact);
        self.registry
            .update(
                self.session_id,
                &self.task_id,
                SessionTaskUpdate {
                    artifacts: Some(artifacts),
                    expected_attempt: Some(self.attempt),
                    ..Default::default()
                },
            )
            .await?;
        Ok(())
    }
}

/// VFS directory for a task's result and logs.
pub fn task_vfs_dir(task_id: &str) -> String {
    format!("/.tasks/{task_id}")
}

/// VFS path for a task's machine result.
pub fn task_result_path(task_id: &str) -> String {
    format!("/.tasks/{task_id}/result.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task() -> SessionTask {
        new_session_task(
            CreateSessionTask {
                session_id: SessionId::new(),
                id: None,
                kind: TASK_KIND_BACKGROUND_TOOL.to_string(),
                display_name: "Test".to_string(),
                spec: serde_json::json!({}),
                state: SessionTaskState::Queued,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            },
            Utc::now(),
        )
    }

    #[test]
    fn create_generates_prefixed_id() {
        let t = task();
        assert!(t.id.starts_with("task_"));
        assert_eq!(t.state, SessionTaskState::Queued);
        assert!(t.started_at.is_none());
    }

    #[test]
    fn serialization_redacts_spec_push_config_secrets() {
        let mut t = task();
        t.spec = serde_json::json!({
            "instructions": "notify",
            "push_configs": [
                {
                    "url": "https://hooks.example.com/everruns",
                    "secret": "LEAKME-HMAC-KEY",
                    "event_filter": ["terminal"]
                },
                {
                    "url": "https://hooks.example.com/no-secret",
                    "event_filter": ["message"]
                }
            ]
        });

        let serialized = serde_json::to_value(&t).expect("task serializes");
        let configs = serialized["spec"]["push_configs"]
            .as_array()
            .expect("push configs stay visible");
        assert_eq!(configs[0]["url"], "https://hooks.example.com/everruns");
        assert_eq!(configs[0]["has_secret"], true);
        assert!(configs[0].get("secret").is_none());
        assert!(configs[1].get("secret").is_none());

        // Redaction is presentation-only so the worker can still sign webhook
        // deliveries from the stored task spec.
        assert_eq!(
            t.spec["push_configs"][0]["secret"], "LEAKME-HMAC-KEY",
            "stored spec remains unchanged for delivery"
        );
    }

    #[test]
    fn first_transition_out_of_queued_stamps_started_at() {
        let mut t = task();
        let now = Utc::now();
        apply_task_update(
            &mut t,
            SessionTaskUpdate {
                state: Some(SessionTaskState::Running),
                ..Default::default()
            },
            now,
        );
        assert_eq!(t.state, SessionTaskState::Running);
        assert_eq!(t.started_at, Some(now));
        assert!(t.finished_at.is_none());
    }

    #[test]
    fn terminal_transition_stamps_finished_at_and_is_final() {
        let mut t = task();
        let now = Utc::now();
        apply_task_update(
            &mut t,
            SessionTaskUpdate {
                state: Some(SessionTaskState::Succeeded),
                summary: Some("done".to_string()),
                ..Default::default()
            },
            now,
        );
        assert_eq!(t.state, SessionTaskState::Succeeded);
        assert_eq!(t.finished_at, Some(now));

        // An update carrying a *different* state lost a race against the
        // terminal transition — it is ignored entirely, content included
        // (e.g. the reaper must not stamp an orphaned error on a task that
        // succeeded meanwhile).
        apply_task_update(
            &mut t,
            SessionTaskUpdate {
                state: Some(SessionTaskState::Failed),
                error: Some(TaskError {
                    kind: "orphaned".to_string(),
                    message: "stale".to_string(),
                }),
                ..Default::default()
            },
            Utc::now(),
        );
        assert_eq!(t.state, SessionTaskState::Succeeded);
        assert!(t.error.is_none());

        // Idempotent re-mirrors with the SAME terminal state still enrich.
        apply_task_update(
            &mut t,
            SessionTaskUpdate {
                state: Some(SessionTaskState::Succeeded),
                result_path: Some("/.tasks/x/result.json".to_string()),
                ..Default::default()
            },
            Utc::now(),
        );
        assert_eq!(t.result_path.as_deref(), Some("/.tasks/x/result.json"));

        // Content-only updates (no state) also still apply.
        apply_task_update(
            &mut t,
            SessionTaskUpdate {
                summary: Some("enriched".to_string()),
                ..Default::default()
            },
            Utc::now(),
        );
        assert_eq!(t.summary.as_deref(), Some("enriched"));
    }

    #[test]
    fn input_request_forces_awaiting_input_and_clears_on_resume() {
        let mut t = task();
        apply_task_update(
            &mut t,
            SessionTaskUpdate {
                input_request: Some(TaskInputRequest {
                    id: "req_1".to_string(),
                    prompt: "Approve?".to_string(),
                    expected: None,
                }),
                ..Default::default()
            },
            Utc::now(),
        );
        assert_eq!(t.state, SessionTaskState::AwaitingInput);
        assert!(t.input_request.is_some());

        apply_task_update(
            &mut t,
            SessionTaskUpdate {
                state: Some(SessionTaskState::Running),
                ..Default::default()
            },
            Utc::now(),
        );
        assert_eq!(t.state, SessionTaskState::Running);
        assert!(t.input_request.is_none());
    }

    #[test]
    fn links_merge_without_duplicates() {
        let mut t = task();
        let child = SessionId::new();
        apply_task_update(
            &mut t,
            SessionTaskUpdate {
                links: Some(TaskLinks {
                    child_session_id: Some(child),
                    remote_task_id: None,
                    resource_ids: vec!["res_1".to_string()],
                }),
                ..Default::default()
            },
            Utc::now(),
        );
        apply_task_update(
            &mut t,
            SessionTaskUpdate {
                links: Some(TaskLinks {
                    child_session_id: None,
                    remote_task_id: Some("rt_1".to_string()),
                    resource_ids: vec!["res_1".to_string(), "res_2".to_string()],
                }),
                ..Default::default()
            },
            Utc::now(),
        );
        assert_eq!(t.links.child_session_id, Some(child));
        assert_eq!(t.links.remote_task_id.as_deref(), Some("rt_1"));
        assert_eq!(t.links.resource_ids, vec!["res_1", "res_2"]);
    }

    #[test]
    fn message_text_rendering() {
        let msg = NewTaskMessage::outbound_text("hello");
        assert_eq!(task_message_text(&msg.content), "hello");
    }

    // -------------------------------------------------------------------------
    // Stale-attempt fencing tests
    // -------------------------------------------------------------------------

    #[test]
    fn update_with_matching_attempt_applies() {
        let mut t = task();
        // Task starts at attempt 1.
        assert_eq!(t.attempt, 1);
        let now = Utc::now();

        // An update that carries expected_attempt == task.attempt applies normally.
        apply_task_update(
            &mut t,
            SessionTaskUpdate {
                state: Some(SessionTaskState::Running),
                state_detail: Some("step 1".to_string()),
                heartbeat_at: Some(now),
                expected_attempt: Some(1),
                ..Default::default()
            },
            now,
        );
        assert_eq!(t.state, SessionTaskState::Running);
        assert_eq!(t.state_detail.as_deref(), Some("step 1"));
        assert_eq!(t.heartbeat_at, Some(now));
    }

    #[test]
    fn update_with_stale_attempt_is_fully_ignored() {
        let mut t = task();
        // Simulate the reaper bumping the attempt by directly setting it.
        t.attempt = 2;

        let now = Utc::now();
        let before_updated = t.updated_at;

        // A write from the old executor (attempt = 1) must be fully ignored —
        // state, heartbeat, and updated_at must not change.
        apply_task_update(
            &mut t,
            SessionTaskUpdate {
                state: Some(SessionTaskState::Running),
                state_detail: Some("superseded".to_string()),
                heartbeat_at: Some(now),
                expected_attempt: Some(1),
                ..Default::default()
            },
            now,
        );
        assert_eq!(t.state, SessionTaskState::Queued, "state must be unchanged");
        assert!(t.state_detail.is_none(), "state_detail must be unchanged");
        assert!(t.heartbeat_at.is_none(), "heartbeat must be unchanged");
        assert_eq!(t.updated_at, before_updated, "updated_at must be unchanged");
    }

    #[test]
    fn update_with_none_expected_attempt_applies_regardless() {
        let mut t = task();
        // Even with attempt = 99 and no expected_attempt, the update applies.
        t.attempt = 99;
        let now = Utc::now();

        apply_task_update(
            &mut t,
            SessionTaskUpdate {
                summary: Some("cancel from API".to_string()),
                expected_attempt: None,
                ..Default::default()
            },
            now,
        );
        // Writers that don't track attempts (e.g. cancel_task from the API) still apply.
        assert_eq!(t.summary.as_deref(), Some("cancel from API"));
    }

    #[test]
    fn reaper_update_increments_attempt_and_fences_old_executor() {
        let mut t = task();
        t.state = SessionTaskState::Running;
        assert_eq!(t.attempt, 1);
        let now = Utc::now();

        // Reaper-style update: fail as orphaned and supersede the attempt.
        apply_task_update(
            &mut t,
            SessionTaskUpdate {
                state: Some(SessionTaskState::Failed),
                error: Some(TaskError {
                    kind: "orphaned".to_string(),
                    message: "worker heartbeat stopped".to_string(),
                }),
                increment_attempt: true,
                ..Default::default()
            },
            now,
        );
        assert_eq!(t.state, SessionTaskState::Failed);
        assert_eq!(t.attempt, 2, "orphan reap must supersede the attempt");

        // The zombie executor's content-only heartbeat (no state change, so the
        // terminal invariant alone would let it through) is now fenced out.
        let later = now + chrono::Duration::seconds(5);
        apply_task_update(
            &mut t,
            SessionTaskUpdate {
                heartbeat_at: Some(later),
                expected_attempt: Some(1),
                ..Default::default()
            },
            later,
        );
        assert_ne!(
            t.heartbeat_at,
            Some(later),
            "stale heartbeat must be rejected"
        );
    }

    #[test]
    fn increment_attempt_is_inert_when_update_is_dropped() {
        let mut t = task();
        t.state = SessionTaskState::Succeeded;
        assert_eq!(t.attempt, 1);
        let now = Utc::now();

        // Reaper losing the race against a clean finish: the terminal-state
        // invariant drops the whole update, including the attempt bump.
        apply_task_update(
            &mut t,
            SessionTaskUpdate {
                state: Some(SessionTaskState::Failed),
                error: Some(TaskError {
                    kind: "orphaned".to_string(),
                    message: "worker heartbeat stopped".to_string(),
                }),
                increment_attempt: true,
                ..Default::default()
            },
            now,
        );
        assert_eq!(t.state, SessionTaskState::Succeeded);
        assert_eq!(t.attempt, 1, "dropped update must not bump the attempt");
    }
}
