// Database-backed session task registry.
//
// Implements the core SessionTaskRegistry trait over StorageBackend (both
// PostgreSQL and in-memory) and emits snapshot-carrying task.* events on the
// owning session's event stream. Event emission is best-effort: failures are
// logged and never fail the storage operation.
//
// Wake policy: after a state transition the registry calls the optional waker
// best-effort (log on error, never fail the operation).

use crate::kernel_imports::{
    contracts::error::AgentLoopError, contracts::error::Result, contracts::typed_id::SessionId,
};
use async_trait::async_trait;
use chrono::Utc;
use everruns_core::event_emitter::EventEmitter;
use everruns_core::events::{
    EventContext, EventData, EventRequest, SessionTaskEventData, TaskMessageEventData,
};
use everruns_core::session_task::{
    CreateSessionTask, NewTaskMessage, SessionTask, SessionTaskFilter, SessionTaskRegistry,
    SessionTaskState, SessionTaskUpdate, TaskMessage, TaskMessageDirection,
    generate_task_message_id, new_session_task, task_message_text,
};

use super::backend::StorageBackend;
use super::models::NewSessionTaskMessageRow;
use std::sync::Arc;

// ============================================================================
// SessionTaskWaker — server-side concern for injecting messages into sessions
// ============================================================================

/// Wake the owning session's agent by injecting a synthetic message. Errors
/// are treated best-effort: the caller logs and continues.
#[async_trait]
pub trait SessionTaskWaker: Send + Sync {
    async fn wake(&self, session_id: SessionId, text: &str) -> anyhow::Result<()>;
}

/// Wakes a session by recording a user-role message and starting a turn, the
/// same path a sent message takes. The message carries the task-wake origin so
/// the model and the UI can tell it apart from the person's own words.
pub(crate) struct InjectedMessageWaker {
    pub(crate) db: Arc<StorageBackend>,
    pub(crate) event_service: crate::services::EventService,
    pub(crate) runner: Option<Arc<dyn everruns_core::host::TurnBackend>>,
}

#[async_trait]
impl SessionTaskWaker for InjectedMessageWaker {
    async fn wake(&self, session_id: SessionId, text: &str) -> anyhow::Result<()> {
        // Unscoped lookup: the waker only learns a session id, and needs its
        // org and harness to start the turn.
        let session = self
            .db
            .get_session_unscoped(session_id)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to look up session for wake: {e}"))?;
        let Some(session) = session else {
            return Ok(());
        };
        let Some(harness_id) = session.harness_id else {
            tracing::debug!(%session_id, "SessionTaskWaker: session has no harness_id; skipping wake");
            return Ok(());
        };

        let message_id = everruns_contracts::typed_id::MessageId::new();
        // The worker resolves credentials and tools through the invocation of
        // the input message; without one the wake turn fails "Unknown
        // invocation". The wake continues the person's latest invocation.
        self.db
            .record_continued_runtime_invocation(
                session.org_id,
                session_id,
                message_id.uuid(),
                session.agent_id.map(|id| id.uuid()),
            )
            .await
            .map_err(|e| anyhow::anyhow!("Failed to record wake invocation: {e}"))?;
        let core_message = everruns_core::RuntimeMessage {
            id: message_id,
            role: everruns_core::RuntimeMessageRole::User,
            content: vec![everruns_core::ContentPart::text(text)],
            phase: None,
            phase_source: None,
            controls: None,
            metadata: Some(everruns_core::message::task_wake_message_metadata()),
            external_actor: None,
            created_at: Utc::now(),
        };
        self.event_service
            .emit(everruns_core::EventRequest::new(
                session_id,
                everruns_core::events::EventContext::empty(),
                everruns_core::events::InputMessageData::new(core_message),
            ))
            .await
            .map_err(|e| anyhow::anyhow!("Failed to emit wake message event: {e}"))?;

        if let Some(runner) = self.runner.clone() {
            let scope = crate::turns::scope(session.org_id, harness_id, session.agent_id);
            let request = crate::turns::stored_message(session_id, scope, message_id, None);
            tokio::spawn(async move {
                if let Err(e) = crate::turns::start(&*runner, request).await {
                    tracing::warn!(%session_id, "SessionTaskWaker: failed to start turn workflow: {e}");
                }
            });
        }
        Ok(())
    }
}

// ============================================================================
// Task transition observers — in-process callbacks on task transitions
// ============================================================================

// The transition enum and observer trait live in `everruns-core`
// (`TaskTransition` / `TaskTransitionObserver`) so `everruns-host` embedders
// can observe transitions in process without depending on the server or HTTP.
// The server's webhook dispatcher (`DirectTaskWebhookNotifier`) is one
// implementation; the registry fires each transition once to every registered
// observer (EVE-729).
pub use everruns_core::task_observer::{TaskTransition, TaskTransitionObserver};

// ============================================================================
// DbSessionTaskRegistry
// ============================================================================

/// Database-backed session task registry.
#[derive(Clone)]
pub struct DbSessionTaskRegistry {
    db: Arc<StorageBackend>,
    emitter: Option<Arc<dyn EventEmitter>>,
    waker: Option<Arc<dyn SessionTaskWaker>>,
    observers: Vec<Arc<dyn TaskTransitionObserver>>,
}

impl DbSessionTaskRegistry {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self {
            db,
            emitter: None,
            waker: None,
            observers: Vec::new(),
        }
    }

    pub fn with_event_emitter(mut self, emitter: Arc<dyn EventEmitter>) -> Self {
        self.emitter = Some(emitter);
        self
    }

    /// Attach a waker so the registry can inject wake messages into sessions
    /// on qualifying state transitions. Only registries constructed with an
    /// event emitter should also have a waker (worker/gRPC paths); the API
    /// path (user-initiated mutations) must not wake.
    pub fn with_waker(mut self, waker: Arc<dyn SessionTaskWaker>) -> Self {
        self.waker = Some(waker);
        self
    }

    /// Attach a task-transition observer. Each observer receives every real
    /// transition (terminal / awaiting_input / outbound message) once, off the
    /// task-update path. Best-effort only: observer errors are logged and never
    /// fail the task operation. The server's webhook dispatcher is registered
    /// here as one observer; in-process embedders can register their own so they
    /// see the same transitions the webhook path fires (EVE-729).
    pub fn with_transition_observer(mut self, observer: Arc<dyn TaskTransitionObserver>) -> Self {
        self.observers.push(observer);
        self
    }

    async fn emit(&self, session_id: SessionId, context: EventContext, data: EventData) {
        let Some(emitter) = &self.emitter else {
            return;
        };
        let request = EventRequest {
            event_type: data.event_type().to_string(),
            ts: Utc::now(),
            session_id,
            context,
            data,
            metadata: None,
            tags: None,
        };
        if let Err(e) = emitter.emit(request).await {
            tracing::warn!(session_id = %session_id, "Failed to emit task event: {e}");
        }
    }

    async fn emit_task_snapshot(&self, task: &SessionTask, created: bool) {
        let data = if created {
            EventData::TaskCreated(SessionTaskEventData { task: task.clone() })
        } else {
            EventData::TaskUpdated(SessionTaskEventData { task: task.clone() })
        };
        let context =
            everruns_core::session_task::task_origin_event_context(task).unwrap_or_default();
        self.emit(task.session_id, context, data).await;
    }

    /// Deliver a wake message to the owning session (best-effort, log on error).
    async fn try_wake(&self, session_id: SessionId, text: &str) {
        let Some(waker) = &self.waker else {
            return;
        };
        if let Err(e) = waker.wake(session_id, text).await {
            tracing::warn!(
                session_id = %session_id,
                "SessionTaskWaker failed (best-effort): {e}"
            );
        }
    }

    /// Compose the wake text for a terminal transition and wake if policy requires.
    ///
    /// Called after a successful `update` that moved a non-terminal task to a
    /// terminal state. Never double-wakes: we only fire when `prior` was
    /// non-terminal AND `task` is terminal.
    async fn maybe_wake_on_terminal(&self, prior: &SessionTask, task: &SessionTask) {
        use everruns_core::session_task::TaskWakePolicy;
        if prior.state.is_terminal() || !task.state.is_terminal() {
            return;
        }
        match task.wake_policy {
            TaskWakePolicy::Silent => {}
            TaskWakePolicy::OnTerminal | TaskWakePolicy::OnActivity => {
                let mut parts = vec![format!(
                    "Task \"{}\" ({}) finished: {}.",
                    task.display_name, task.id, task.state
                )];
                if let Some(summary) = &task.summary {
                    parts.push(format!("- summary: {summary}"));
                }
                if let Some(result_path) = &task.result_path {
                    parts.push(format!("- result_path: {result_path}"));
                }
                self.try_wake(task.session_id, &parts.join("\n")).await;
            }
        }
    }

    /// Wake on a transition INTO `awaiting_input` (OnActivity only).
    async fn maybe_wake_on_awaiting_input(&self, prior: &SessionTask, task: &SessionTask) {
        use everruns_core::session_task::TaskWakePolicy;
        if task.wake_policy != TaskWakePolicy::OnActivity {
            return;
        }
        if prior.state == SessionTaskState::AwaitingInput
            || task.state != SessionTaskState::AwaitingInput
        {
            return;
        }
        let prompt = task
            .input_request
            .as_ref()
            .map(|r| r.prompt.as_str())
            .unwrap_or("Task is awaiting input.");
        let text = format!(
            "Task \"{}\" ({}) is awaiting input: {}",
            task.display_name, task.id, prompt
        );
        self.try_wake(task.session_id, &text).await;
    }

    /// Wake on an outbound message (OnActivity only).
    async fn maybe_wake_on_outbound_message(&self, task: &SessionTask, message_text: &str) {
        use everruns_core::session_task::TaskWakePolicy;
        if task.wake_policy != TaskWakePolicy::OnActivity {
            return;
        }
        let message_text = if message_text.trim().is_empty() {
            "structured progress update"
        } else {
            message_text
        };
        let text = format!(
            "Task \"{}\" ({}) sent a message: {}",
            task.display_name, task.id, message_text
        );
        self.try_wake(task.session_id, &text).await;
    }

    /// Notify all registered observers of a task transition (best-effort).
    /// Each observer is dispatched on its own detached task so downstream
    /// latency (e.g. outbound webhook HTTP) never blocks task updates and one
    /// slow observer never delays another.
    fn notify_transition(&self, task: &SessionTask, transition: TaskTransition) {
        if self.observers.is_empty() {
            return;
        }
        for observer in &self.observers {
            let observer = observer.clone();
            let task = task.clone();
            tokio::spawn(async move {
                if let Err(e) = observer.on_transition(&task, transition).await {
                    tracing::warn!(
                        task_id = %task.id,
                        session_id = %task.session_id,
                        transition = ?transition,
                        "TaskTransitionObserver failed (best-effort): {e}"
                    );
                }
            });
        }
    }
}

#[async_trait]
impl SessionTaskRegistry for DbSessionTaskRegistry {
    async fn create(&self, input: CreateSessionTask) -> Result<SessionTask> {
        let task = new_session_task(input, Utc::now());
        let (row, inserted) =
            self.db.create_session_task(&task).await.map_err(|e| {
                AgentLoopError::store(format!("Failed to create session task: {e}"))
            })?;
        let task = row
            .to_task()
            .map_err(|e| AgentLoopError::store(format!("Invalid session task row: {e}")))?;
        if inserted {
            self.emit_task_snapshot(&task, true).await;
        }
        Ok(task)
    }

    async fn update(
        &self,
        session_id: SessionId,
        task_id: &str,
        update: SessionTaskUpdate,
    ) -> Result<Option<SessionTask>> {
        // Only THIS update's intent can trigger a wake: an unrelated update
        // (heartbeat/progress) racing with another worker's transition must
        // not observe prior=non-terminal/new=terminal and wake on its behalf.
        let wants_terminal_wake = update.state.is_some_and(|s| s.is_terminal());
        let wants_awaiting_input_wake =
            update.input_request.is_some() || update.state == Some(SessionTaskState::AwaitingInput);

        // Read prior state so we can detect the transition this update makes.
        // Best-effort: if the read fails we still proceed with the update.
        // Observers need `prior` for both terminal and awaiting_input
        // transitions (EVE-682), so they have the same gating as the waker.
        let needs_prior = (self.waker.is_some() || !self.observers.is_empty())
            && (wants_terminal_wake || wants_awaiting_input_wake);
        let prior = if needs_prior {
            self.get(session_id, task_id).await.ok().flatten()
        } else {
            None
        };

        let row = self
            .db
            .update_session_task(session_id, task_id, update)
            .await
            .map_err(|e| AgentLoopError::store(format!("Failed to update session task: {e}")))?;
        let task = row
            .as_ref()
            .map(|r| r.to_task())
            .transpose()
            .map_err(|e| AgentLoopError::store(format!("Invalid session task row: {e}")))?;
        if let Some(task) = &task {
            self.emit_task_snapshot(task, false).await;

            // Wake/notify enforcement: fire at most once per transition,
            // gated on the intent of this specific update.
            if let Some(prior) = &prior {
                if wants_terminal_wake {
                    self.maybe_wake_on_terminal(prior, task).await;
                    // Observers fire on the same terminal transition.
                    if !prior.state.is_terminal() && task.state.is_terminal() {
                        self.notify_transition(task, TaskTransition::Terminal);
                    }
                }
                if wants_awaiting_input_wake {
                    self.maybe_wake_on_awaiting_input(prior, task).await;
                    // Per-task push configs may opt into awaiting_input delivery
                    // (EVE-682). Fire only on the transition INTO awaiting_input,
                    // mirroring the wake gate so idempotent input_request churn
                    // never re-fires.
                    if prior.state != SessionTaskState::AwaitingInput
                        && task.state == SessionTaskState::AwaitingInput
                    {
                        self.notify_transition(task, TaskTransition::AwaitingInput);
                    }
                }
            }
        }
        Ok(task)
    }

    async fn get(&self, session_id: SessionId, task_id: &str) -> Result<Option<SessionTask>> {
        let row = self
            .db
            .get_session_task(session_id, task_id)
            .await
            .map_err(|e| AgentLoopError::store(format!("Failed to get session task: {e}")))?;
        row.as_ref()
            .map(|r| r.to_task())
            .transpose()
            .map_err(|e| AgentLoopError::store(format!("Invalid session task row: {e}")))
    }

    async fn list(
        &self,
        session_id: SessionId,
        filter: Option<&SessionTaskFilter>,
    ) -> Result<Vec<SessionTask>> {
        let kind = filter.and_then(|f| f.kind.as_deref());
        let state = filter.and_then(|f| f.state.map(|s| s.to_string()));
        let rows = self
            .db
            .list_session_tasks(session_id, kind, state.as_deref())
            .await
            .map_err(|e| AgentLoopError::store(format!("Failed to list session tasks: {e}")))?;
        rows.iter()
            .map(|r| {
                r.to_task()
                    .map_err(|e| AgentLoopError::store(format!("Invalid session task row: {e}")))
            })
            .collect()
    }

    async fn request_cancel(
        &self,
        session_id: SessionId,
        task_id: &str,
    ) -> Result<Option<SessionTask>> {
        let row = self
            .db
            .request_cancel_session_task(session_id, task_id)
            .await
            .map_err(|e| {
                AgentLoopError::store(format!("Failed to request session task cancel: {e}"))
            })?;
        let Some((row, changed)) = row else {
            return Ok(None);
        };
        let task = row
            .to_task()
            .map_err(|e| AgentLoopError::store(format!("Invalid session task row: {e}")))?;
        if changed {
            self.emit_task_snapshot(&task, false).await;
        }
        Ok(Some(task))
    }

    async fn record_message(
        &self,
        session_id: SessionId,
        task_id: &str,
        message: NewTaskMessage,
    ) -> Result<TaskMessage> {
        let task = self
            .get(session_id, task_id)
            .await?
            .ok_or_else(|| AgentLoopError::store(format!("Session task not found: {task_id}")))?;

        // Stale-attempt fence: a superseded executor must not append to the
        // thread (record_message emits events and can wake the parent).
        if let Some(expected) = message.expected_attempt
            && expected != task.attempt
        {
            return Err(AgentLoopError::store(format!(
                "Stale attempt {expected} for task {task_id} (current attempt {})",
                task.attempt
            )));
        }

        let row = self
            .db
            .insert_session_task_message(NewSessionTaskMessageRow {
                id: generate_task_message_id(),
                task_id: task_id.to_string(),
                session_id,
                direction: message.direction.to_string(),
                content: serde_json::to_value(&message.content).map_err(|e| {
                    AgentLoopError::store(format!("Invalid task message content: {e}"))
                })?,
                in_reply_to: message.in_reply_to.clone(),
            })
            .await
            .map_err(|e| AgentLoopError::store(format!("Failed to record task message: {e}")))?;
        let stored = row
            .to_message()
            .map_err(|e| AgentLoopError::store(format!("Invalid task message row: {e}")))?;

        let data = match stored.direction {
            TaskMessageDirection::Inbound => EventData::TaskMessageSent(TaskMessageEventData {
                task_id: task_id.to_string(),
                message: stored.clone(),
            }),
            TaskMessageDirection::Outbound => {
                EventData::TaskMessageReceived(TaskMessageEventData {
                    task_id: task_id.to_string(),
                    message: stored.clone(),
                })
            }
        };
        let context =
            everruns_core::session_task::task_origin_event_context(&task).unwrap_or_default();
        self.emit(session_id, context, data).await;

        // Wake on outbound messages for OnActivity tasks.
        if stored.direction == TaskMessageDirection::Outbound {
            let msg_text = task_message_text(&stored.content);
            self.maybe_wake_on_outbound_message(&task, &msg_text).await;
            // Per-task push configs may opt into message delivery (EVE-682).
            self.notify_transition(&task, TaskTransition::Message);
        }

        // An inbound answer to the pending input request resumes the task.
        if stored.direction == TaskMessageDirection::Inbound
            && let (Some(in_reply_to), Some(pending)) = (&stored.in_reply_to, &task.input_request)
            && in_reply_to == &pending.id
        {
            self.update(
                session_id,
                task_id,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Running),
                    ..Default::default()
                },
            )
            .await?;
        }

        Ok(stored)
    }

    async fn list_messages(
        &self,
        session_id: SessionId,
        task_id: &str,
        limit: Option<u32>,
        after_id: Option<&str>,
    ) -> Result<Vec<TaskMessage>> {
        let rows = self
            .db
            .list_session_task_messages(session_id, task_id, limit, after_id)
            .await
            .map_err(|e| AgentLoopError::store(format!("Failed to list task messages: {e}")))?;
        rows.iter()
            .map(|r| {
                r.to_message()
                    .map_err(|e| AgentLoopError::store(format!("Invalid task message row: {e}")))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
