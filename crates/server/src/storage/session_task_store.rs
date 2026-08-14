// Database-backed session task registry.
//
// Implements the core SessionTaskRegistry trait over StorageBackend (both
// PostgreSQL and in-memory) and emits snapshot-carrying task.* events on the
// owning session's event stream. Event emission is best-effort: failures are
// logged and never fail the storage operation.
//
// Wake policy: after a state transition the registry calls the optional waker
// best-effort (log on error, never fail the operation).

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
use everruns_core::{AgentLoopError, Result, SessionId};

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

    async fn emit(&self, session_id: SessionId, data: EventData) {
        let Some(emitter) = &self.emitter else {
            return;
        };
        let request = EventRequest {
            event_type: data.event_type().to_string(),
            ts: Utc::now(),
            session_id,
            context: EventContext::default(),
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
        self.emit(task.session_id, data).await;
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
        self.emit(session_id, data).await;

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
mod tests {
    use super::*;
    use everruns_core::session_task::{
        TaskInputRequest, TaskLinks, TaskMessagePart, TaskWakePolicy,
    };
    use std::sync::Mutex;

    // -------------------------------------------------------------------------
    // Recording test waker
    // -------------------------------------------------------------------------

    #[derive(Default, Clone)]
    struct RecordingWaker {
        calls: Arc<Mutex<Vec<(SessionId, String)>>>,
    }

    impl RecordingWaker {
        fn recorded(&self) -> Vec<(SessionId, String)> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl SessionTaskWaker for RecordingWaker {
        async fn wake(&self, session_id: SessionId, text: &str) -> anyhow::Result<()> {
            self.calls
                .lock()
                .unwrap()
                .push((session_id, text.to_string()));
            Ok(())
        }
    }

    fn registry_with_waker(waker: Arc<dyn SessionTaskWaker>) -> DbSessionTaskRegistry {
        DbSessionTaskRegistry::new(Arc::new(StorageBackend::in_memory())).with_waker(waker)
    }

    // -------------------------------------------------------------------------
    // Recording test observer (stands in for a webhook or in-process observer)
    // -------------------------------------------------------------------------

    #[derive(Default, Clone)]
    struct RecordingObserver {
        calls: Arc<Mutex<Vec<(String, TaskTransition)>>>,
    }

    impl RecordingObserver {
        fn recorded(&self) -> Vec<(String, TaskTransition)> {
            self.calls.lock().unwrap().clone()
        }

        fn transitions(&self) -> Vec<TaskTransition> {
            self.recorded().into_iter().map(|(_, t)| t).collect()
        }
    }

    #[async_trait::async_trait]
    impl TaskTransitionObserver for RecordingObserver {
        async fn on_transition(
            &self,
            task: &SessionTask,
            transition: TaskTransition,
        ) -> anyhow::Result<()> {
            self.calls
                .lock()
                .unwrap()
                .push((task.id.clone(), transition));
            Ok(())
        }
    }

    fn registry_with_observer(observer: Arc<RecordingObserver>) -> DbSessionTaskRegistry {
        DbSessionTaskRegistry::new(Arc::new(StorageBackend::in_memory()))
            .with_transition_observer(observer)
    }

    /// `notify_transition` spawns a detached task per observer; poll until the
    /// expected number of notifications land (or give up after bounded yields).
    async fn wait_for_notifications(observer: &RecordingObserver, expected: usize) {
        for _ in 0..200 {
            if observer.recorded().len() >= expected {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    fn registry() -> DbSessionTaskRegistry {
        DbSessionTaskRegistry::new(Arc::new(StorageBackend::in_memory()))
    }

    fn create_input(session_id: SessionId) -> CreateSessionTask {
        CreateSessionTask {
            session_id,
            id: None,
            kind: "background_tool".to_string(),
            display_name: "Test run".to_string(),
            spec: serde_json::json!({"tool": "demo"}),
            state: SessionTaskState::Queued,
            links: TaskLinks::default(),
            wake_policy: TaskWakePolicy::Silent,
        }
    }

    #[tokio::test]
    async fn create_is_idempotent_on_id() {
        let registry = registry();
        let session_id = SessionId::new();
        let mut input = create_input(session_id);
        input.id = Some("task_fixed".to_string());

        let first = registry.create(input.clone()).await.unwrap();
        let mut second_input = input.clone();
        second_input.display_name = "Changed".to_string();
        let second = registry.create(second_input).await.unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(second.display_name, "Test run");
    }

    #[tokio::test]
    async fn create_rejects_id_reuse_across_sessions() {
        let registry = registry();
        let mut input = create_input(SessionId::new());
        input.id = Some("task_shared".to_string());
        registry.create(input.clone()).await.unwrap();

        let mut other = create_input(SessionId::new());
        other.id = Some("task_shared".to_string());
        assert!(registry.create(other).await.is_err());
    }

    #[tokio::test]
    async fn update_applies_lifecycle_invariants() {
        let registry = registry();
        let session_id = SessionId::new();
        let task = registry.create(create_input(session_id)).await.unwrap();
        assert!(task.started_at.is_none());

        let task = registry
            .update(
                session_id,
                &task.id,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Running),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(task.state, SessionTaskState::Running);
        assert!(task.started_at.is_some());

        let task = registry
            .update(
                session_id,
                &task.id,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Succeeded),
                    summary: Some("done".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert!(task.finished_at.is_some());

        // Terminal is final.
        let task = registry
            .update(
                session_id,
                &task.id,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Running),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(task.state, SessionTaskState::Succeeded);
    }

    #[tokio::test]
    async fn request_cancel_is_idempotent() {
        let registry = registry();
        let session_id = SessionId::new();
        let task = registry.create(create_input(session_id)).await.unwrap();

        let first = registry
            .request_cancel(session_id, &task.id)
            .await
            .unwrap()
            .unwrap();
        let stamp = first.cancel_requested_at.unwrap();
        let second = registry
            .request_cancel(session_id, &task.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second.cancel_requested_at, Some(stamp));
        assert_eq!(second.state, SessionTaskState::Queued);
    }

    #[tokio::test]
    async fn answering_input_request_resumes_task() {
        let registry = registry();
        let session_id = SessionId::new();
        let task = registry.create(create_input(session_id)).await.unwrap();

        let task = registry
            .update(
                session_id,
                &task.id,
                SessionTaskUpdate {
                    input_request: Some(TaskInputRequest {
                        id: "req_1".to_string(),
                        prompt: "Proceed?".to_string(),
                        expected: None,
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(task.state, SessionTaskState::AwaitingInput);

        let mut answer = NewTaskMessage::inbound_text("yes");
        answer.in_reply_to = Some("req_1".to_string());
        registry
            .record_message(session_id, &task.id, answer)
            .await
            .unwrap();

        let task = registry.get(session_id, &task.id).await.unwrap().unwrap();
        assert_eq!(task.state, SessionTaskState::Running);
        assert!(task.input_request.is_none());

        let messages = registry
            .list_messages(session_id, &task.id, None, None)
            .await
            .unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].in_reply_to.as_deref(), Some("req_1"));
    }

    #[tokio::test]
    async fn list_filters_by_kind_and_state() {
        let registry = registry();
        let session_id = SessionId::new();
        let mut a = create_input(session_id);
        a.kind = "subagent".to_string();
        let mut b = create_input(session_id);
        b.kind = "background_tool".to_string();
        b.state = SessionTaskState::Running;
        registry.create(a).await.unwrap();
        registry.create(b).await.unwrap();

        let subagents = registry
            .list(
                session_id,
                Some(&SessionTaskFilter {
                    kind: Some("subagent".to_string()),
                    state: None,
                }),
            )
            .await
            .unwrap();
        assert_eq!(subagents.len(), 1);

        let running = registry
            .list(
                session_id,
                Some(&SessionTaskFilter {
                    kind: None,
                    state: Some(SessionTaskState::Running),
                }),
            )
            .await
            .unwrap();
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].kind, "background_tool");
    }

    #[tokio::test]
    async fn message_limit_returns_most_recent_oldest_first() {
        let registry = registry();
        let session_id = SessionId::new();
        let task = registry.create(create_input(session_id)).await.unwrap();
        for i in 0..5 {
            registry
                .record_message(
                    session_id,
                    &task.id,
                    NewTaskMessage::inbound_text(format!("m{i}")),
                )
                .await
                .unwrap();
        }
        let messages = registry
            .list_messages(session_id, &task.id, Some(2), None)
            .await
            .unwrap();
        assert_eq!(messages.len(), 2);
        let texts: Vec<String> = messages
            .iter()
            .map(|m| everruns_core::session_task::task_message_text(&m.content))
            .collect();
        assert_eq!(texts, vec!["m3", "m4"]);
    }

    #[tokio::test]
    async fn record_message_rejects_stale_attempt() {
        let registry = registry();
        let session_id = SessionId::new();
        let task = registry
            .create(create_input_with_policy(session_id, TaskWakePolicy::Silent))
            .await
            .unwrap();

        // Reaper-style supersede: fail as orphaned and bump the attempt.
        registry
            .update(
                session_id,
                &task.id,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Failed),
                    increment_attempt: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // The zombie executor (attempt 1) must not append to the thread.
        let err = registry
            .record_message(
                session_id,
                &task.id,
                NewTaskMessage::outbound_text("zombie").with_expected_attempt(1),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Stale attempt"), "got: {err}");

        // Unfenced writers (API messages) still apply.
        registry
            .record_message(session_id, &task.id, NewTaskMessage::inbound_text("user"))
            .await
            .unwrap();
    }

    // -------------------------------------------------------------------------
    // Wake policy tests
    // -------------------------------------------------------------------------

    fn create_input_with_policy(
        session_id: SessionId,
        policy: TaskWakePolicy,
    ) -> CreateSessionTask {
        CreateSessionTask {
            session_id,
            id: None,
            kind: "background_tool".to_string(),
            display_name: "Test task".to_string(),
            spec: serde_json::json!({}),
            state: SessionTaskState::Queued,
            links: TaskLinks::default(),
            wake_policy: policy,
        }
    }

    #[tokio::test]
    async fn silent_policy_never_wakes() {
        let waker = Arc::new(RecordingWaker::default());
        let registry = registry_with_waker(waker.clone());
        let session_id = SessionId::new();

        let task = registry
            .create(create_input_with_policy(session_id, TaskWakePolicy::Silent))
            .await
            .unwrap();
        registry
            .update(
                session_id,
                &task.id,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Running),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        registry
            .update(
                session_id,
                &task.id,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Succeeded),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        // Record an outbound message to verify no wake fires.
        registry
            .record_message(session_id, &task.id, NewTaskMessage::outbound_text("hello"))
            .await
            .unwrap();

        assert!(waker.recorded().is_empty(), "Silent should never wake");
    }

    #[tokio::test]
    async fn on_terminal_wakes_exactly_once_on_terminal_transition() {
        let waker = Arc::new(RecordingWaker::default());
        let registry = registry_with_waker(waker.clone());
        let session_id = SessionId::new();

        let task = registry
            .create(create_input_with_policy(
                session_id,
                TaskWakePolicy::OnTerminal,
            ))
            .await
            .unwrap();

        // Non-terminal transition: no wake.
        registry
            .update(
                session_id,
                &task.id,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Running),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(waker.recorded().is_empty(), "Should not wake on running");

        // Terminal transition: one wake.
        registry
            .update(
                session_id,
                &task.id,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Succeeded),
                    summary: Some("done".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let calls = waker.recorded();
        assert_eq!(calls.len(), 1, "Should wake exactly once on terminal");
        assert!(
            calls[0].1.contains("finished: succeeded"),
            "Wake text should describe terminal state"
        );
        assert!(
            calls[0].1.contains("summary: done"),
            "Wake text should include summary"
        );

        // Second update on already-terminal task: no second wake.
        registry
            .update(
                session_id,
                &task.id,
                SessionTaskUpdate {
                    result_path: Some("/.tasks/x/result.json".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(waker.recorded().len(), 1, "No double-wake after terminal");
    }

    #[tokio::test]
    async fn on_activity_wakes_on_awaiting_input_and_outbound_message() {
        let waker = Arc::new(RecordingWaker::default());
        let registry = registry_with_waker(waker.clone());
        let session_id = SessionId::new();

        let task = registry
            .create(create_input_with_policy(
                session_id,
                TaskWakePolicy::OnActivity,
            ))
            .await
            .unwrap();

        // Running transition: no wake.
        registry
            .update(
                session_id,
                &task.id,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Running),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(waker.recorded().is_empty());

        // Outbound message: one wake.
        registry
            .record_message(
                session_id,
                &task.id,
                NewTaskMessage::outbound_text("task says hi"),
            )
            .await
            .unwrap();
        let calls = waker.recorded();
        assert_eq!(calls.len(), 1, "Should wake on outbound message");
        assert!(calls[0].1.contains("task says hi"));

        // Data-only outbound messages still get a readable wake prompt.
        registry
            .record_message(
                session_id,
                &task.id,
                NewTaskMessage {
                    direction: TaskMessageDirection::Outbound,
                    content: vec![TaskMessagePart::Data {
                        data: serde_json::json!({"step": "halfway"}),
                    }],
                    in_reply_to: None,
                    expected_attempt: None,
                },
            )
            .await
            .unwrap();
        let calls = waker.recorded();
        assert_eq!(calls.len(), 2, "Should wake on data-only outbound message");
        assert!(calls[1].1.contains("structured progress update"));

        // Awaiting input: another wake.
        registry
            .update(
                session_id,
                &task.id,
                SessionTaskUpdate {
                    input_request: Some(TaskInputRequest {
                        id: "req_1".to_string(),
                        prompt: "Approve?".to_string(),
                        expected: None,
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let calls = waker.recorded();
        assert_eq!(calls.len(), 3, "Should wake on awaiting_input transition");
        assert!(calls[2].1.contains("Approve?"));

        // Second awaiting_input update (idempotent input_request churn from
        // polling): no extra wake because state is already awaiting_input.
        registry
            .update(
                session_id,
                &task.id,
                SessionTaskUpdate {
                    input_request: Some(TaskInputRequest {
                        id: "req_1".to_string(),
                        prompt: "Approve?".to_string(),
                        expected: None,
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            waker.recorded().len(),
            3,
            "No duplicate wake for repeated awaiting_input"
        );

        // Terminal transition also wakes.
        let mut answer = NewTaskMessage::inbound_text("yes");
        answer.in_reply_to = Some("req_1".to_string());
        registry
            .record_message(session_id, &task.id, answer)
            .await
            .unwrap();
        registry
            .update(
                session_id,
                &task.id,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Succeeded),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let calls = waker.recorded();
        assert_eq!(
            calls.len(),
            4,
            "Should also wake on terminal for OnActivity"
        );
    }

    /// The registry fires observer notifications on terminal, awaiting_input, and
    /// outbound-message transitions — each exactly once per real transition, and
    /// never on inbound messages. Event-filter honoring is the observer's job
    /// (tested there); here we assert the store emits the right transition kinds.
    #[tokio::test]
    async fn observer_fires_on_terminal_awaiting_input_and_outbound() {
        let observer = Arc::new(RecordingObserver::default());
        let registry = registry_with_observer(observer.clone());
        let session_id = SessionId::new();

        let task = registry
            .create(create_input_with_policy(session_id, TaskWakePolicy::Silent))
            .await
            .unwrap();

        // Running: no notification (non-terminal, not awaiting_input).
        registry
            .update(
                session_id,
                &task.id,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Running),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // Awaiting input: one AwaitingInput notification.
        registry
            .update(
                session_id,
                &task.id,
                SessionTaskUpdate {
                    input_request: Some(TaskInputRequest {
                        id: "req_1".to_string(),
                        prompt: "Approve?".to_string(),
                        expected: None,
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        wait_for_notifications(&observer, 1).await;

        // Repeated awaiting_input churn: no extra notification.
        registry
            .update(
                session_id,
                &task.id,
                SessionTaskUpdate {
                    input_request: Some(TaskInputRequest {
                        id: "req_1".to_string(),
                        prompt: "Approve?".to_string(),
                        expected: None,
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // Answer resumes to running, then an outbound message fires Message.
        let mut answer = NewTaskMessage::inbound_text("yes");
        answer.in_reply_to = Some("req_1".to_string());
        registry
            .record_message(session_id, &task.id, answer)
            .await
            .unwrap();
        registry
            .record_message(
                session_id,
                &task.id,
                NewTaskMessage::outbound_text("progress"),
            )
            .await
            .unwrap();
        wait_for_notifications(&observer, 2).await;

        // Terminal transition fires Terminal.
        registry
            .update(
                session_id,
                &task.id,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Succeeded),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        wait_for_notifications(&observer, 3).await;

        let events: Vec<TaskTransition> = observer.transitions();
        assert!(
            events.contains(&TaskTransition::AwaitingInput),
            "expected an awaiting_input notification, got: {events:?}"
        );
        assert!(
            events.contains(&TaskTransition::Message),
            "expected an outbound message notification, got: {events:?}"
        );
        assert!(
            events.contains(&TaskTransition::Terminal),
            "expected a terminal notification, got: {events:?}"
        );
        // Exactly one of each — no double-fire on awaiting_input churn, and the
        // inbound answer never produced a Message event.
        assert_eq!(
            events.len(),
            3,
            "each transition must notify exactly once, got: {events:?}"
        );
    }

    /// EVE-729 parity: an in-process observer registered alongside the webhook
    /// observer receives exactly the same transitions, in the same order, that
    /// the webhook path fires. Both observers share the registry's single
    /// transition-detection path, so an embedder's in-process callback is
    /// guaranteed the same event stream as HTTP webhook delivery — no HTTP.
    #[tokio::test]
    async fn in_process_observer_has_parity_with_webhook_observer() {
        // `webhook` stands in for the server's DirectTaskWebhookNotifier; both
        // are just `TaskTransitionObserver`s after EVE-729.
        let webhook = Arc::new(RecordingObserver::default());
        let in_process = Arc::new(RecordingObserver::default());
        let registry = DbSessionTaskRegistry::new(Arc::new(StorageBackend::in_memory()))
            .with_transition_observer(webhook.clone())
            .with_transition_observer(in_process.clone());
        let session_id = SessionId::new();

        let task = registry
            .create(create_input_with_policy(session_id, TaskWakePolicy::Silent))
            .await
            .unwrap();

        // Drive running -> awaiting_input -> (answer resumes) running -> outbound
        // message -> terminal, waiting for each transition to land on both
        // observers before the next so the recorded order is deterministic.
        registry
            .update(
                session_id,
                &task.id,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Running),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        registry
            .update(
                session_id,
                &task.id,
                SessionTaskUpdate {
                    input_request: Some(TaskInputRequest {
                        id: "req_1".to_string(),
                        prompt: "Approve?".to_string(),
                        expected: None,
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        wait_for_notifications(&webhook, 1).await;
        wait_for_notifications(&in_process, 1).await;

        let mut answer = NewTaskMessage::inbound_text("yes");
        answer.in_reply_to = Some("req_1".to_string());
        registry
            .record_message(session_id, &task.id, answer)
            .await
            .unwrap();
        registry
            .record_message(
                session_id,
                &task.id,
                NewTaskMessage::outbound_text("progress"),
            )
            .await
            .unwrap();
        wait_for_notifications(&webhook, 2).await;
        wait_for_notifications(&in_process, 2).await;

        registry
            .update(
                session_id,
                &task.id,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Succeeded),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        wait_for_notifications(&webhook, 3).await;
        wait_for_notifications(&in_process, 3).await;

        let webhook_events = webhook.transitions();
        let in_process_events = in_process.transitions();
        assert_eq!(
            webhook_events,
            vec![
                TaskTransition::AwaitingInput,
                TaskTransition::Message,
                TaskTransition::Terminal,
            ],
            "webhook observer should see the canonical transition stream"
        );
        assert_eq!(
            in_process_events, webhook_events,
            "in-process observer must receive the same transitions the webhook path fires"
        );
    }

    #[tokio::test]
    async fn inbound_messages_do_not_wake() {
        let waker = Arc::new(RecordingWaker::default());
        let registry = registry_with_waker(waker.clone());
        let session_id = SessionId::new();

        let task = registry
            .create(create_input_with_policy(
                session_id,
                TaskWakePolicy::OnActivity,
            ))
            .await
            .unwrap();
        registry
            .record_message(
                session_id,
                &task.id,
                NewTaskMessage::inbound_text("steering"),
            )
            .await
            .unwrap();
        assert!(
            waker.recorded().is_empty(),
            "Inbound messages should not wake"
        );
    }
}
