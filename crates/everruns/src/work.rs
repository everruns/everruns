//! Session-owned background work and wakes.
//!
//! [`WorkQueue`](crate::work::WorkQueue) is the application-facing control
//! plane for work owned by an Everruns session. The default
//! [`InMemoryWorkBackend`](crate::work::InMemoryWorkBackend) is offline and
//! database-free. Hosts that need process-restart durability can implement
//! [`WorkBackend`](crate::work::WorkBackend) without exposing platform stores
//! or runtime registries to application code.
//!
//! Work and wake delivery are **at least once**. Claims have explicit leases;
//! an unacknowledged delivery becomes claimable again after its lease expires.
//! Handlers should therefore use
//! [`Task::idempotency_key`](crate::work::Task::idempotency_key) or
//! [`Task::id`](crate::work::Task::id) when applying external side effects.
//! Cancellation is cooperative for claimed work:
//! [`SessionWork::cancel`](crate::work::SessionWork::cancel) records intent,
//! and the handler settles the task with
//! [`TaskOutcome::Canceled`](crate::work::TaskOutcome::Canceled) after it
//! stops safely.
//!
//! # Example
//!
//! ```
//! use everruns::work::{TaskRequest, WakePolicy, WorkQueue};
//! use serde_json::json;
//!
//! # async fn example() -> Result<(), everruns::work::WorkError> {
//! let queue = WorkQueue::in_memory();
//! let session_work = queue.for_session("session_123");
//! session_work
//!     .submit(
//!         TaskRequest::new("thumbnail", json!({ "image": "cover.png" }))
//!             .idempotency_key("thumbnail:cover.png")
//!             .wake_policy(WakePolicy::OnCompletion),
//!     )
//!     .await?;
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use serde_json::Value;

/// When a task becomes eligible for delivery.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum WorkSchedule {
    /// Deliver the task as soon as a worker claims due work.
    #[default]
    Immediate,
    /// Deliver this one-shot task no earlier than this wall-clock time.
    At(SystemTime),
}

/// Whether a terminal task creates a wake for its owning session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum WakePolicy {
    /// Do not create a wake; the application can inspect the task explicitly.
    #[default]
    Never,
    /// Create one leased wake when the task first reaches a terminal state.
    OnCompletion,
}

/// A request for session-owned work.
#[derive(Clone, Debug, PartialEq)]
pub struct TaskRequest {
    kind: String,
    input: Value,
    schedule: WorkSchedule,
    idempotency_key: Option<String>,
    wake_policy: WakePolicy,
}

impl TaskRequest {
    /// Create an immediate work request for an application-defined `kind`.
    ///
    /// `kind` is routing metadata for the application's worker. It must not be
    /// blank. `input` is stored unchanged and delivered to the worker.
    pub fn new(kind: impl Into<String>, input: impl Into<Value>) -> Self {
        Self {
            kind: kind.into(),
            input: input.into(),
            schedule: WorkSchedule::Immediate,
            idempotency_key: None,
            wake_policy: WakePolicy::Never,
        }
    }

    /// Set when this work becomes eligible for delivery.
    pub fn schedule(mut self, schedule: WorkSchedule) -> Self {
        self.schedule = schedule;
        self
    }

    /// Set a session-scoped idempotency key for submission.
    ///
    /// Repeating the same request with the same key returns the original task.
    /// Reusing the key for a different request returns
    /// [`WorkError::IdempotencyConflict`].
    pub fn idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    /// Set the task's terminal wake policy.
    pub fn wake_policy(mut self, policy: WakePolicy) -> Self {
        self.wake_policy = policy;
        self
    }

    /// Application-defined routing name.
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// Application-defined JSON input.
    pub fn input(&self) -> &Value {
        &self.input
    }

    /// Requested delivery time.
    pub fn work_schedule(&self) -> WorkSchedule {
        self.schedule
    }

    /// Session-scoped submission idempotency key, if supplied.
    pub fn submission_key(&self) -> Option<&str> {
        self.idempotency_key.as_deref()
    }

    /// Terminal wake policy.
    pub fn completion_wake_policy(&self) -> WakePolicy {
        self.wake_policy
    }
}

/// Lifecycle state of a session task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TaskState {
    /// Stored but not currently claimed by a worker.
    Pending,
    /// Claimed by a worker. Its lease may still expire and be redelivered.
    Running,
    /// Finished successfully.
    Succeeded,
    /// Finished with an application error.
    Failed,
    /// Canceled before execution or cooperatively stopped by a worker.
    Canceled,
}

impl TaskState {
    /// Whether no further work deliveries can be claimed for this task.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Canceled)
    }
}

/// Terminal result reported by a worker.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum TaskOutcome {
    /// Work completed; `output` is application-defined persisted result data.
    Succeeded {
        /// Optional human-readable summary suitable for a wake or activity UI.
        summary: Option<String>,
        /// Application-defined machine-readable output.
        output: Value,
    },
    /// Work could not complete.
    Failed {
        /// Stable, application-defined failure message.
        error: String,
    },
    /// The worker observed cancellation and stopped safely.
    Canceled,
}

impl TaskOutcome {
    /// Construct a successful outcome with JSON output.
    pub fn success(output: impl Into<Value>) -> Self {
        Self::Succeeded {
            summary: None,
            output: output.into(),
        }
    }

    /// Construct a successful outcome with a human summary and JSON output.
    pub fn success_with_summary(summary: impl Into<String>, output: impl Into<Value>) -> Self {
        Self::Succeeded {
            summary: Some(summary.into()),
            output: output.into(),
        }
    }

    /// Construct a failed outcome.
    pub fn failure(error: impl Into<String>) -> Self {
        Self::Failed {
            error: error.into(),
        }
    }

    /// Terminal task state represented by this outcome.
    pub fn task_state(&self) -> TaskState {
        match self {
            Self::Succeeded { .. } => TaskState::Succeeded,
            Self::Failed { .. } => TaskState::Failed,
            Self::Canceled => TaskState::Canceled,
        }
    }
}

/// A background task owned by one session.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Task {
    /// Opaque stable task identifier. Use it to deduplicate handler effects.
    pub id: String,
    /// Opaque identifier of the owning Everruns session.
    pub session_id: String,
    /// Application-defined routing name.
    pub kind: String,
    /// Application-defined JSON input.
    pub input: Value,
    /// Current lifecycle state.
    pub state: TaskState,
    /// When cooperative cancellation was first requested.
    pub cancel_requested_at: Option<SystemTime>,
    /// Session-scoped submission idempotency key, if supplied.
    pub idempotency_key: Option<String>,
    /// Earliest delivery time.
    pub scheduled_at: SystemTime,
    /// Time the request was first accepted.
    pub created_at: SystemTime,
    /// Number of work deliveries claimed so far, including retries.
    pub attempts: u32,
    /// Time the task first reached a terminal state.
    pub finished_at: Option<SystemTime>,
    /// Terminal result, once settled.
    pub outcome: Option<TaskOutcome>,
}

impl Task {
    /// Construct a pending task snapshot from an accepted request.
    ///
    /// Custom [`WorkBackend`] implementations can use this constructor before
    /// persisting the task. The high-level [`WorkQueue`] validates the request
    /// before calling the backend.
    pub fn pending(
        id: impl Into<String>,
        session_id: impl Into<String>,
        request: &TaskRequest,
        created_at: SystemTime,
    ) -> Self {
        let scheduled_at = match request.schedule {
            WorkSchedule::Immediate => created_at,
            WorkSchedule::At(at) => at,
        };
        Self {
            id: id.into(),
            session_id: session_id.into(),
            kind: request.kind.clone(),
            input: request.input.clone(),
            state: TaskState::Pending,
            cancel_requested_at: None,
            idempotency_key: request.idempotency_key.clone(),
            scheduled_at,
            created_at,
            attempts: 0,
            finished_at: None,
            outcome: None,
        }
    }
}

/// A leased, at-least-once task delivery.
#[derive(Clone, PartialEq)]
pub struct TaskDelivery {
    /// Current task snapshot.
    pub task: Task,
    /// One-based delivery attempt number.
    pub attempt: u32,
    /// Time after which an unfinished delivery may be claimed again.
    pub lease_expires_at: SystemTime,
    lease_token: String,
}

impl std::fmt::Debug for TaskDelivery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskDelivery")
            .field("task", &self.task)
            .field("attempt", &self.attempt)
            .field("lease_expires_at", &self.lease_expires_at)
            .field("lease_token", &"[redacted]")
            .finish()
    }
}

impl TaskDelivery {
    /// Construct a backend delivery.
    ///
    /// Custom [`WorkBackend`] implementations use this after atomically
    /// claiming a task. Tokens must be unique per attempt and unguessable when
    /// they cross a trust boundary.
    pub fn from_claim(
        task: Task,
        attempt: u32,
        lease_expires_at: SystemTime,
        lease_token: impl Into<String>,
    ) -> Self {
        Self {
            task,
            attempt,
            lease_expires_at,
            lease_token: lease_token.into(),
        }
    }

    /// Opaque token fencing settlement to this delivery attempt.
    pub fn lease_token(&self) -> &str {
        &self.lease_token
    }
}

/// Why an application should wake a session.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum WakeReason {
    /// An application requested an immediate wake with arbitrary JSON data.
    Requested {
        /// Application-defined wake data.
        payload: Value,
    },
    /// A task with [`WakePolicy::OnCompletion`] reached a terminal state.
    TaskFinished {
        /// Stable identifier of the completed task.
        task_id: String,
        /// Terminal task state.
        state: TaskState,
        /// Task outcome copied into the wake for handling without another read.
        outcome: TaskOutcome,
    },
}

/// A wake owned by one session.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct SessionWake {
    /// Opaque stable wake identifier.
    pub id: String,
    /// Opaque identifier of the session to wake.
    pub session_id: String,
    /// Reason and application payload for the wake.
    pub reason: WakeReason,
    /// Time the wake was first recorded.
    pub created_at: SystemTime,
    /// Session-scoped request idempotency key, when this was a direct wake.
    pub idempotency_key: Option<String>,
}

impl SessionWake {
    /// Construct a direct application-requested wake.
    ///
    /// Custom [`WorkBackend`] implementations can use this constructor before
    /// atomically persisting the wake.
    pub fn requested(
        id: impl Into<String>,
        session_id: impl Into<String>,
        request: &WakeRequest,
        created_at: SystemTime,
    ) -> Self {
        Self {
            id: id.into(),
            session_id: session_id.into(),
            reason: WakeReason::Requested {
                payload: request.payload.clone(),
            },
            created_at,
            idempotency_key: request.idempotency_key.clone(),
        }
    }

    /// Construct a completion wake from a terminal task snapshot.
    ///
    /// Returns `None` when the task has no terminal outcome.
    pub fn task_finished(
        id: impl Into<String>,
        task: &Task,
        created_at: SystemTime,
    ) -> Option<Self> {
        let outcome = task.outcome.clone()?;
        if !task.state.is_terminal() || outcome.task_state() != task.state {
            return None;
        }
        Some(Self {
            id: id.into(),
            session_id: task.session_id.clone(),
            reason: WakeReason::TaskFinished {
                task_id: task.id.clone(),
                state: task.state,
                outcome,
            },
            created_at,
            idempotency_key: None,
        })
    }
}

/// A request to wake a session immediately.
#[derive(Clone, Debug, PartialEq)]
pub struct WakeRequest {
    payload: Value,
    idempotency_key: Option<String>,
}

impl WakeRequest {
    /// Create an immediate wake carrying application-defined JSON data.
    pub fn new(payload: impl Into<Value>) -> Self {
        Self {
            payload: payload.into(),
            idempotency_key: None,
        }
    }

    /// Set a session-scoped idempotency key.
    ///
    /// Repeating an identical request returns the original wake. Reusing the
    /// key for different data returns [`WorkError::IdempotencyConflict`].
    pub fn idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    /// Application-defined wake data.
    pub fn payload(&self) -> &Value {
        &self.payload
    }

    /// Session-scoped idempotency key, if supplied.
    pub fn submission_key(&self) -> Option<&str> {
        self.idempotency_key.as_deref()
    }
}

/// A leased, at-least-once wake delivery.
#[derive(Clone, PartialEq)]
pub struct WakeDelivery {
    /// Current wake snapshot.
    pub wake: SessionWake,
    /// One-based delivery attempt number.
    pub attempt: u32,
    /// Time after which an unacknowledged wake may be delivered again.
    pub lease_expires_at: SystemTime,
    lease_token: String,
}

impl std::fmt::Debug for WakeDelivery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WakeDelivery")
            .field("wake", &self.wake)
            .field("attempt", &self.attempt)
            .field("lease_expires_at", &self.lease_expires_at)
            .field("lease_token", &"[redacted]")
            .finish()
    }
}

impl WakeDelivery {
    /// Construct a backend wake delivery after atomically claiming it.
    pub fn from_claim(
        wake: SessionWake,
        attempt: u32,
        lease_expires_at: SystemTime,
        lease_token: impl Into<String>,
    ) -> Self {
        Self {
            wake,
            attempt,
            lease_expires_at,
            lease_token: lease_token.into(),
        }
    }

    /// Opaque token fencing acknowledgment to this delivery attempt.
    pub fn lease_token(&self) -> &str {
        &self.lease_token
    }
}

/// Failure from the high-level work API or its configured backend.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum WorkError {
    /// A required application value was blank or otherwise invalid.
    InvalidRequest {
        /// Name of the invalid field.
        field: &'static str,
        /// Human-readable validation detail.
        message: String,
    },
    /// No task exists with the requested id in the requested session.
    TaskNotFound {
        /// Requested task id.
        task_id: String,
    },
    /// An idempotency key was reused for a different request.
    IdempotencyConflict {
        /// Conflicting session-scoped key.
        key: String,
    },
    /// The delivery lease was superseded or does not own the task/wake.
    StaleDelivery {
        /// Task or wake id whose claim is stale.
        id: String,
    },
    /// The requested lifecycle change is not valid from the current state.
    InvalidTransition {
        /// Human-readable transition detail.
        message: String,
    },
    /// The selected backend failed.
    Backend {
        /// Backend-provided failure detail.
        message: String,
    },
}

impl WorkError {
    /// Construct an error from a custom backend failure.
    pub fn backend(message: impl Into<String>) -> Self {
        Self::Backend {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for WorkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRequest { field, message } => {
                write!(f, "invalid work request field {field}: {message}")
            }
            Self::TaskNotFound { task_id } => write!(f, "task not found: {task_id}"),
            Self::IdempotencyConflict { key } => {
                write!(f, "idempotency key was reused for different work: {key}")
            }
            Self::StaleDelivery { id } => write!(f, "delivery no longer owns {id}"),
            Self::InvalidTransition { message } => write!(f, "invalid work transition: {message}"),
            Self::Backend { message } => write!(f, "work backend failed: {message}"),
        }
    }
}

impl std::error::Error for WorkError {}

/// Storage and leasing provider for [`WorkQueue`].
///
/// Implementations own persistence, atomic claim fencing, restart behavior,
/// retention, and multi-host coordination. The trait deliberately speaks only
/// in application-level tasks and wakes. A durable platform adapter may use a
/// database or scheduler internally without exposing either to library users.
///
/// `submit` and `request_wake` must enforce session-scoped idempotency. Claims
/// must be at least once: expired, unfinished leases become eligible again.
/// `finish` and `acknowledge_wake` must fence stale lease tokens and be
/// idempotent when repeated with the same successful token and value. Hosted
/// implementations also own tenant authorization, payload limits, quotas, and
/// retention at the boundary where they expose this provider.
#[async_trait]
pub trait WorkBackend: Send + Sync {
    /// Store a task request or return the existing idempotent task.
    async fn submit(
        &self,
        session_id: &str,
        request: TaskRequest,
        now: SystemTime,
    ) -> Result<Task, WorkError>;

    /// Read one task owned by `session_id`.
    async fn task(&self, session_id: &str, task_id: &str) -> Result<Option<Task>, WorkError>;

    /// Request cooperative cancellation and return the updated task.
    async fn cancel(
        &self,
        session_id: &str,
        task_id: &str,
        now: SystemTime,
    ) -> Result<Task, WorkError>;

    /// Atomically claim up to `limit` due or lease-expired tasks.
    async fn claim_due(
        &self,
        now: SystemTime,
        lease_for: Duration,
        limit: usize,
    ) -> Result<Vec<TaskDelivery>, WorkError>;

    /// Settle a claimed task and atomically persist any completion wake.
    async fn finish(
        &self,
        delivery: &TaskDelivery,
        outcome: TaskOutcome,
        now: SystemTime,
    ) -> Result<Task, WorkError>;

    /// Store a direct session wake or return the existing idempotent wake.
    async fn request_wake(
        &self,
        session_id: &str,
        request: WakeRequest,
        now: SystemTime,
    ) -> Result<SessionWake, WorkError>;

    /// Atomically claim up to `limit` unacknowledged or lease-expired wakes.
    async fn claim_wakes(
        &self,
        now: SystemTime,
        lease_for: Duration,
        limit: usize,
    ) -> Result<Vec<WakeDelivery>, WorkError>;

    /// Acknowledge a wake. Repeating the same acknowledgment is idempotent.
    async fn acknowledge_wake(&self, delivery: &WakeDelivery) -> Result<(), WorkError>;
}

/// High-level entry point for session work and wakes.
///
/// [`Default`] and [`in_memory`](Self::in_memory) use a process-local backend.
/// Recreating a `WorkQueue` around the same backend preserves its state;
/// dropping the backend or restarting the process loses it. Use
/// [`with_backend`](Self::with_backend) with a host-owned durable provider when
/// process-restart recovery is required.
#[derive(Clone)]
pub struct WorkQueue {
    backend: Arc<dyn WorkBackend>,
}

impl WorkQueue {
    /// Create an offline, database-free queue.
    pub fn in_memory() -> Self {
        Self::with_backend(Arc::new(InMemoryWorkBackend::new()))
    }

    /// Use an application- or host-provided persistence and leasing backend.
    pub fn with_backend(backend: Arc<dyn WorkBackend>) -> Self {
        Self { backend }
    }

    /// Scope requests and reads to one opaque session id.
    pub fn for_session(&self, session_id: impl Into<String>) -> SessionWork {
        SessionWork {
            queue: self.clone(),
            session_id: session_id.into(),
        }
    }

    /// Claim due work using the current wall clock.
    pub async fn claim_due(
        &self,
        lease_for: Duration,
        limit: usize,
    ) -> Result<Vec<TaskDelivery>, WorkError> {
        self.claim_due_at(SystemTime::now(), lease_for, limit).await
    }

    /// Claim due work at a caller-supplied time.
    ///
    /// Hosts can call this from their scheduler. Supplying time explicitly
    /// keeps delayed-work tests deterministic and avoids a hidden polling loop.
    pub async fn claim_due_at(
        &self,
        now: SystemTime,
        lease_for: Duration,
        limit: usize,
    ) -> Result<Vec<TaskDelivery>, WorkError> {
        validate_claim(lease_for, limit)?;
        self.backend.claim_due(now, lease_for, limit).await
    }

    /// Settle a claimed task using the current wall clock.
    pub async fn finish(
        &self,
        delivery: &TaskDelivery,
        outcome: TaskOutcome,
    ) -> Result<Task, WorkError> {
        self.finish_at(delivery, outcome, SystemTime::now()).await
    }

    /// Settle a claimed task at a caller-supplied time.
    pub async fn finish_at(
        &self,
        delivery: &TaskDelivery,
        outcome: TaskOutcome,
        now: SystemTime,
    ) -> Result<Task, WorkError> {
        self.backend.finish(delivery, outcome, now).await
    }

    /// Claim pending wakes using the current wall clock.
    pub async fn claim_wakes(
        &self,
        lease_for: Duration,
        limit: usize,
    ) -> Result<Vec<WakeDelivery>, WorkError> {
        self.claim_wakes_at(SystemTime::now(), lease_for, limit)
            .await
    }

    /// Claim pending wakes at a caller-supplied time.
    pub async fn claim_wakes_at(
        &self,
        now: SystemTime,
        lease_for: Duration,
        limit: usize,
    ) -> Result<Vec<WakeDelivery>, WorkError> {
        validate_claim(lease_for, limit)?;
        self.backend.claim_wakes(now, lease_for, limit).await
    }

    /// Acknowledge successful handling of one wake.
    pub async fn acknowledge_wake(&self, delivery: &WakeDelivery) -> Result<(), WorkError> {
        self.backend.acknowledge_wake(delivery).await
    }
}

impl Default for WorkQueue {
    fn default() -> Self {
        Self::in_memory()
    }
}

/// A [`WorkQueue`] scoped to one owning session.
#[derive(Clone)]
pub struct SessionWork {
    queue: WorkQueue,
    session_id: String,
}

impl SessionWork {
    /// Opaque id of the session that owns every request through this handle.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Submit immediate or scheduled work.
    pub async fn submit(&self, request: TaskRequest) -> Result<Task, WorkError> {
        self.submit_at(request, SystemTime::now()).await
    }

    /// Submit work with a caller-supplied acceptance time.
    pub async fn submit_at(
        &self,
        request: TaskRequest,
        now: SystemTime,
    ) -> Result<Task, WorkError> {
        validate_session_id(&self.session_id)?;
        validate_task_request(&request)?;
        self.queue
            .backend
            .submit(&self.session_id, request, now)
            .await
    }

    /// Read a task owned by this session.
    pub async fn task(&self, task_id: &str) -> Result<Option<Task>, WorkError> {
        validate_session_id(&self.session_id)?;
        validate_task_id(task_id)?;
        self.queue.backend.task(&self.session_id, task_id).await
    }

    /// Request cancellation of a task owned by this session.
    ///
    /// Pending work becomes canceled immediately. Running work records
    /// `cancel_requested`; its handler must stop cooperatively and finish with
    /// [`TaskOutcome::Canceled`]. Repeating cancellation is safe.
    pub async fn cancel(&self, task_id: &str) -> Result<Task, WorkError> {
        self.cancel_at(task_id, SystemTime::now()).await
    }

    /// Request cancellation at a caller-supplied time.
    pub async fn cancel_at(&self, task_id: &str, now: SystemTime) -> Result<Task, WorkError> {
        validate_session_id(&self.session_id)?;
        validate_task_id(task_id)?;
        self.queue
            .backend
            .cancel(&self.session_id, task_id, now)
            .await
    }

    /// Record an immediate wake for this session.
    pub async fn wake(&self, request: WakeRequest) -> Result<SessionWake, WorkError> {
        self.wake_at(request, SystemTime::now()).await
    }

    /// Record an immediate wake with a caller-supplied acceptance time.
    pub async fn wake_at(
        &self,
        request: WakeRequest,
        now: SystemTime,
    ) -> Result<SessionWake, WorkError> {
        validate_session_id(&self.session_id)?;
        validate_wake_request(&request)?;
        self.queue
            .backend
            .request_wake(&self.session_id, request, now)
            .await
    }
}

fn validate_session_id(session_id: &str) -> Result<(), WorkError> {
    validate_non_blank("session_id", session_id)
}

fn validate_task_id(task_id: &str) -> Result<(), WorkError> {
    validate_non_blank("task_id", task_id)
}

fn validate_non_blank(field: &'static str, value: &str) -> Result<(), WorkError> {
    if value.trim().is_empty() {
        return Err(WorkError::InvalidRequest {
            field,
            message: "must not be blank".to_string(),
        });
    }
    Ok(())
}

fn validate_task_request(request: &TaskRequest) -> Result<(), WorkError> {
    if request.kind.trim().is_empty() {
        return Err(WorkError::InvalidRequest {
            field: "kind",
            message: "must not be blank".to_string(),
        });
    }
    if request
        .idempotency_key
        .as_ref()
        .is_some_and(|key| key.trim().is_empty())
    {
        return Err(WorkError::InvalidRequest {
            field: "idempotency_key",
            message: "must not be blank".to_string(),
        });
    }
    Ok(())
}

fn validate_wake_request(request: &WakeRequest) -> Result<(), WorkError> {
    if request
        .idempotency_key
        .as_ref()
        .is_some_and(|key| key.trim().is_empty())
    {
        return Err(WorkError::InvalidRequest {
            field: "idempotency_key",
            message: "must not be blank".to_string(),
        });
    }
    Ok(())
}

fn validate_claim(lease_for: Duration, limit: usize) -> Result<(), WorkError> {
    if lease_for.is_zero() {
        return Err(WorkError::InvalidRequest {
            field: "lease_for",
            message: "must be positive".to_string(),
        });
    }
    if limit == 0 {
        return Err(WorkError::InvalidRequest {
            field: "limit",
            message: "must be positive".to_string(),
        });
    }
    Ok(())
}

/// Process-local backend used by [`WorkQueue::in_memory`].
///
/// State survives replacing a `WorkQueue` when the same `Arc` backend is
/// retained, but does not survive a process restart. Claims and completion
/// wakes follow the same at-least-once and idempotency contract as durable
/// backends, making it suitable for local development and deterministic tests.
/// It retains accepted payloads until the backend is dropped and applies no
/// admission quota; hosted providers should enforce their own limits and
/// retention policy.
pub struct InMemoryWorkBackend {
    state: Mutex<MemoryState>,
}

impl InMemoryWorkBackend {
    /// Create an empty in-memory backend.
    pub fn new() -> Self {
        Self {
            state: Mutex::new(MemoryState::default()),
        }
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, MemoryState>, WorkError> {
        self.state
            .lock()
            .map_err(|_| WorkError::backend("in-memory state lock was poisoned"))
    }
}

impl Default for InMemoryWorkBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Default)]
struct MemoryState {
    tasks: HashMap<String, StoredTask>,
    task_order: Vec<String>,
    task_keys: HashMap<(String, String), String>,
    wakes: HashMap<String, StoredWake>,
    wake_order: Vec<String>,
    wake_keys: HashMap<(String, String), String>,
}

struct StoredTask {
    task: Task,
    request: TaskRequest,
    wake_policy: WakePolicy,
    claim: Option<Lease>,
    last_settlement: Option<(String, TaskOutcome)>,
}

struct StoredWake {
    wake: SessionWake,
    request: Option<WakeRequest>,
    claim: Option<Lease>,
    acknowledged_by: Option<String>,
    attempts: u32,
}

#[derive(Clone)]
struct Lease {
    token: String,
    expires_at: SystemTime,
}

#[async_trait]
impl WorkBackend for InMemoryWorkBackend {
    async fn submit(
        &self,
        session_id: &str,
        request: TaskRequest,
        now: SystemTime,
    ) -> Result<Task, WorkError> {
        let mut state = self.lock()?;
        if let Some(key) = request.idempotency_key.as_ref() {
            let scope = (session_id.to_string(), key.clone());
            if let Some(task_id) = state.task_keys.get(&scope) {
                let stored = state
                    .tasks
                    .get(task_id)
                    .expect("key references stored task");
                if stored.request == request {
                    return Ok(stored.task.clone());
                }
                return Err(WorkError::IdempotencyConflict { key: key.clone() });
            }
        }

        let task = Task::pending(new_id("task"), session_id, &request, now);
        if let Some(key) = request.idempotency_key.as_ref() {
            state
                .task_keys
                .insert((session_id.to_string(), key.clone()), task.id.clone());
        }
        state.task_order.push(task.id.clone());
        state.tasks.insert(
            task.id.clone(),
            StoredTask {
                task: task.clone(),
                wake_policy: request.wake_policy,
                request,
                claim: None,
                last_settlement: None,
            },
        );
        Ok(task)
    }

    async fn task(&self, session_id: &str, task_id: &str) -> Result<Option<Task>, WorkError> {
        let state = self.lock()?;
        Ok(state
            .tasks
            .get(task_id)
            .filter(|stored| stored.task.session_id == session_id)
            .map(|stored| stored.task.clone()))
    }

    async fn cancel(
        &self,
        session_id: &str,
        task_id: &str,
        now: SystemTime,
    ) -> Result<Task, WorkError> {
        let mut state = self.lock()?;
        let (task, wake_policy, should_wake) = {
            let stored = state
                .tasks
                .get_mut(task_id)
                .filter(|stored| stored.task.session_id == session_id)
                .ok_or_else(|| WorkError::TaskNotFound {
                    task_id: task_id.to_string(),
                })?;

            if stored.task.state.is_terminal() {
                return Ok(stored.task.clone());
            }
            stored.task.cancel_requested_at.get_or_insert(now);
            let mut should_wake = false;
            if stored.task.state == TaskState::Pending {
                stored.task.state = TaskState::Canceled;
                stored.task.finished_at = Some(now);
                stored.task.outcome = Some(TaskOutcome::Canceled);
                should_wake = stored.wake_policy == WakePolicy::OnCompletion;
            }
            (stored.task.clone(), stored.wake_policy, should_wake)
        };
        if should_wake {
            push_task_wake(&mut state, &task, wake_policy, now);
        }
        Ok(task)
    }

    async fn claim_due(
        &self,
        now: SystemTime,
        lease_for: Duration,
        limit: usize,
    ) -> Result<Vec<TaskDelivery>, WorkError> {
        let mut state = self.lock()?;
        let mut deliveries = Vec::new();
        for task_id in state.task_order.clone() {
            if deliveries.len() == limit {
                break;
            }
            let stored = state.tasks.get_mut(&task_id).expect("ordered task exists");
            let pending_due =
                stored.task.state == TaskState::Pending && stored.task.scheduled_at <= now;
            let lease_expired = stored.task.state == TaskState::Running
                && stored
                    .claim
                    .as_ref()
                    .is_none_or(|claim| claim.expires_at <= now);
            if !pending_due && !lease_expired {
                continue;
            }

            stored.task.state = TaskState::Running;
            stored.task.attempts = stored.task.attempts.saturating_add(1);
            let attempt = stored.task.attempts;
            let expires_at =
                now.checked_add(lease_for)
                    .ok_or_else(|| WorkError::InvalidRequest {
                        field: "lease_for",
                        message: "lease expiration is outside SystemTime range".to_string(),
                    })?;
            let token = new_id("lease");
            stored.claim = Some(Lease {
                token: token.clone(),
                expires_at,
            });
            deliveries.push(TaskDelivery::from_claim(
                stored.task.clone(),
                attempt,
                expires_at,
                token,
            ));
        }
        Ok(deliveries)
    }

    async fn finish(
        &self,
        delivery: &TaskDelivery,
        outcome: TaskOutcome,
        now: SystemTime,
    ) -> Result<Task, WorkError> {
        let mut state = self.lock()?;
        let (task, wake_policy, should_wake) =
            {
                let stored = state.tasks.get_mut(&delivery.task.id).ok_or_else(|| {
                    WorkError::TaskNotFound {
                        task_id: delivery.task.id.clone(),
                    }
                })?;

                if let Some((token, settled)) = stored.last_settlement.as_ref()
                    && token == delivery.lease_token()
                {
                    if settled == &outcome {
                        return Ok(stored.task.clone());
                    }
                    return Err(WorkError::InvalidTransition {
                        message: "the delivery was already settled with a different outcome"
                            .to_string(),
                    });
                }

                let owns_claim = stored.task.state == TaskState::Running
                    && stored.task.attempts == delivery.attempt
                    && stored
                        .claim
                        .as_ref()
                        .is_some_and(|claim| claim.token == delivery.lease_token());
                if !owns_claim {
                    return Err(WorkError::StaleDelivery {
                        id: delivery.task.id.clone(),
                    });
                }

                stored.task.state = outcome.task_state();
                stored.task.finished_at = Some(now);
                stored.task.outcome = Some(outcome.clone());
                stored.claim = None;
                stored.last_settlement = Some((delivery.lease_token().to_string(), outcome));
                let should_wake = stored.wake_policy == WakePolicy::OnCompletion;
                (stored.task.clone(), stored.wake_policy, should_wake)
            };
        if should_wake {
            push_task_wake(&mut state, &task, wake_policy, now);
        }
        Ok(task)
    }

    async fn request_wake(
        &self,
        session_id: &str,
        request: WakeRequest,
        now: SystemTime,
    ) -> Result<SessionWake, WorkError> {
        let mut state = self.lock()?;
        if let Some(key) = request.idempotency_key.as_ref() {
            let scope = (session_id.to_string(), key.clone());
            if let Some(wake_id) = state.wake_keys.get(&scope) {
                let stored = state
                    .wakes
                    .get(wake_id)
                    .expect("key references stored wake");
                if stored.request.as_ref() == Some(&request) {
                    return Ok(stored.wake.clone());
                }
                return Err(WorkError::IdempotencyConflict { key: key.clone() });
            }
        }

        let wake = SessionWake::requested(new_id("wake"), session_id, &request, now);
        if let Some(key) = request.idempotency_key.as_ref() {
            state
                .wake_keys
                .insert((session_id.to_string(), key.clone()), wake.id.clone());
        }
        push_wake(&mut state, wake.clone(), Some(request));
        Ok(wake)
    }

    async fn claim_wakes(
        &self,
        now: SystemTime,
        lease_for: Duration,
        limit: usize,
    ) -> Result<Vec<WakeDelivery>, WorkError> {
        let mut state = self.lock()?;
        let mut deliveries = Vec::new();
        for wake_id in state.wake_order.clone() {
            if deliveries.len() == limit {
                break;
            }
            let stored = state.wakes.get_mut(&wake_id).expect("ordered wake exists");
            if stored.acknowledged_by.is_some() {
                continue;
            }
            let available = stored
                .claim
                .as_ref()
                .is_none_or(|claim| claim.expires_at <= now);
            if !available {
                continue;
            }
            stored.attempts = stored.attempts.saturating_add(1);
            let expires_at =
                now.checked_add(lease_for)
                    .ok_or_else(|| WorkError::InvalidRequest {
                        field: "lease_for",
                        message: "lease expiration is outside SystemTime range".to_string(),
                    })?;
            let token = new_id("wakelease");
            stored.claim = Some(Lease {
                token: token.clone(),
                expires_at,
            });
            deliveries.push(WakeDelivery::from_claim(
                stored.wake.clone(),
                stored.attempts,
                expires_at,
                token,
            ));
        }
        Ok(deliveries)
    }

    async fn acknowledge_wake(&self, delivery: &WakeDelivery) -> Result<(), WorkError> {
        let mut state = self.lock()?;
        let stored =
            state
                .wakes
                .get_mut(&delivery.wake.id)
                .ok_or_else(|| WorkError::StaleDelivery {
                    id: delivery.wake.id.clone(),
                })?;
        if stored.acknowledged_by.as_deref() == Some(delivery.lease_token()) {
            return Ok(());
        }
        let owns_claim = stored.attempts == delivery.attempt
            && stored
                .claim
                .as_ref()
                .is_some_and(|claim| claim.token == delivery.lease_token());
        if !owns_claim {
            return Err(WorkError::StaleDelivery {
                id: delivery.wake.id.clone(),
            });
        }
        stored.acknowledged_by = Some(delivery.lease_token().to_string());
        stored.claim = None;
        Ok(())
    }
}

fn push_task_wake(state: &mut MemoryState, task: &Task, wake_policy: WakePolicy, now: SystemTime) {
    if wake_policy != WakePolicy::OnCompletion {
        return;
    }
    let Some(wake) = SessionWake::task_finished(new_id("wake"), task, now) else {
        return;
    };
    push_wake(state, wake, None);
}

fn push_wake(state: &mut MemoryState, wake: SessionWake, request: Option<WakeRequest>) {
    state.wake_order.push(wake.id.clone());
    state.wakes.insert(
        wake.id.clone(),
        StoredWake {
            wake,
            request,
            claim: None,
            acknowledged_by: None,
            attempts: 0,
        },
    );
}

fn new_id(prefix: &str) -> String {
    let generated = everruns_core::session_task::generate_task_id();
    format!("{prefix}_{}", generated.trim_start_matches("task_"))
}
