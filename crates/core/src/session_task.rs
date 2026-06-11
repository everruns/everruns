// Session tasks — unified registry of background work owned by a session.
//
// See specs/session-tasks.md. A task is any asynchronous work a session owns
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
use serde::{Deserialize, Serialize};
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
pub const TASK_KIND_EXTERNAL_AGENT: &str = "external_agent";
pub const TASK_KIND_BACKGROUND_TOOL: &str = "background_tool";

/// Generate a new task ID (`task_` prefix per specs/id-schema.md).
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
    /// Task kind: "subagent", "external_agent", "background_tool", "monitor", …
    pub kind: String,
    /// Human-readable label.
    pub display_name: String,
    /// Kind-specific input (instructions, tool args, external agent id).
    #[serde(default)]
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
    let was_terminal = task.state.is_terminal();

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
}

impl NewTaskMessage {
    pub fn inbound_text(text: impl Into<String>) -> Self {
        Self {
            direction: TaskMessageDirection::Inbound,
            content: vec![TaskMessagePart::text(text)],
            in_reply_to: None,
        }
    }

    pub fn outbound_text(text: impl Into<String>) -> Self {
        Self {
            direction: TaskMessageDirection::Outbound,
            content: vec![TaskMessagePart::text(text)],
            in_reply_to: None,
        }
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
    async fn list_messages(
        &self,
        session_id: SessionId,
        task_id: &str,
        limit: Option<u32>,
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

    /// Begin execution, or re-attach after worker loss.
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
/// about them (same pattern as `SessionSandboxProviderPlugin`).
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
pub struct RegistryTaskSink {
    registry: Arc<dyn SessionTaskRegistry>,
    session_id: SessionId,
    task_id: String,
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
        }
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
        self.registry
            .record_message(self.session_id, &self.task_id, message)
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
        let mut artifacts = task.artifacts;
        artifacts.push(artifact);
        self.registry
            .update(
                self.session_id,
                &self.task_id,
                SessionTaskUpdate {
                    artifacts: Some(artifacts),
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

        // State changes after terminal are ignored; content still applies.
        apply_task_update(
            &mut t,
            SessionTaskUpdate {
                state: Some(SessionTaskState::Running),
                result_path: Some("/.tasks/x/result.json".to_string()),
                ..Default::default()
            },
            Utc::now(),
        );
        assert_eq!(t.state, SessionTaskState::Succeeded);
        assert_eq!(t.result_path.as_deref(), Some("/.tasks/x/result.json"));
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
}
