// Embeddable in-process task-transition observer (EVE-729).
//
// A `TaskTransition` names a lifecycle transition a `SessionTaskRegistry` fires
// on: reaching a terminal state, entering `awaiting_input`, or emitting an
// outbound message. A `TaskTransitionObserver` receives those transitions in
// process — the same seam the server's webhook dispatcher uses, minus HTTP.
//
// Design Decision: the enum + trait live in `everruns-core` (not the server) so
// `everruns-host` embedders can observe task transitions without depending on
// the control-plane server or making HTTP calls. The server webhook dispatcher
// (`DirectTaskWebhookNotifier`) is one implementation of this trait; in-process
// embedders provide their own. A `SessionTaskRegistry` fires each real
// transition once to every registered observer, so an in-process observer sees
// exactly the same transitions the webhook path fires (see the parity test in
// `crates/server/src/storage/session_task_store.rs`).
//
// Filter semantics: `Terminal` is the regression-safe default (org webhooks only
// ever fire on it); `AwaitingInput` and `Message` are the non-terminal
// transitions that are opt-in per delivery target via `event_filter` (EVE-682).
// The `filter_value` / `event_name` strings are shared with webhook payloads so
// the two paths stay byte-for-byte aligned.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use async_trait::async_trait;

use crate::error::Result;
use crate::session_task::{
    CreateSessionTask, NewTaskMessage, SessionTask, SessionTaskFilter, SessionTaskRegistry,
    SessionTaskState, SessionTaskUpdate, TaskMessage, TaskMessageDirection,
};
use crate::typed_id::SessionId;

/// A task lifecycle transition an observer can be notified of.
///
/// `Terminal` is the only transition org webhooks ever fire on (regression-safe).
/// `AwaitingInput` and `Message` are opt-in per delivery target via
/// `event_filter` (EVE-682).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskTransition {
    /// Task reached a terminal state (succeeded / failed / canceled).
    Terminal,
    /// Task transitioned into `awaiting_input`.
    AwaitingInput,
    /// Task emitted an outbound message.
    Message,
}

impl TaskTransition {
    /// The `event_filter` member string that enables this transition.
    pub fn filter_value(&self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
            Self::AwaitingInput => "awaiting_input",
            Self::Message => "message",
        }
    }

    /// The `event` field value in a delivered webhook payload.
    pub fn event_name(&self) -> &'static str {
        match self {
            Self::Terminal => "task.terminal",
            Self::AwaitingInput => "task.awaiting_input",
            Self::Message => "task.message",
        }
    }
}

/// Receive task-transition notifications in process.
///
/// A `SessionTaskRegistry` invokes `on_transition` once per real transition for
/// every registered observer. Implementations must treat delivery as
/// best-effort: the registry logs errors and never fails the underlying task
/// operation because an observer returned `Err`. Observers must not block for
/// long — the registry dispatches them off the task-update path, but a slow
/// observer still delays its own delivery.
///
/// The server webhook dispatcher (`DirectTaskWebhookNotifier`) is one
/// implementation. Embedders of `everruns-host` implement this trait to get
/// in-process callbacks with the same transition semantics, without HTTP.
#[async_trait]
pub trait TaskTransitionObserver: Send + Sync + 'static {
    /// Handle one task transition. Best-effort: returning `Err` is logged and
    /// never fails the task operation that produced the transition.
    async fn on_transition(
        &self,
        task: &SessionTask,
        transition: TaskTransition,
    ) -> anyhow::Result<()>;
}

type TaskUpdateLocks = HashMap<(SessionId, String), Weak<tokio::sync::Mutex<()>>>;

/// A [`SessionTaskRegistry`] decorator that fans real task transitions out to
/// registered [`TaskTransitionObserver`]s.
///
/// This is the reusable, storage-agnostic form of the fan-out the server's
/// `DbSessionTaskRegistry` performs inline (EVE-729): it wraps *any* inner
/// registry (in-memory, SQLite, gRPC) so an embedder — e.g. `everruns-host`
/// with a [`crate::wake_queue::SessionWakeQueue`] — gets the same transition
/// notifications without depending on the control-plane server.
///
/// Transition detection mirrors `DbSessionTaskRegistry` exactly so mid-turn and
/// between-turn delivery agree on *when* a wake fires:
///
///   * `Terminal` — fired once when an update moves a non-terminal task into a
///     terminal state, gated on this update's own intent (an update that does
///     not set a terminal state never fires it, so a racing heartbeat cannot
///     wake on another writer's transition).
///   * `AwaitingInput` — fired once on the transition into `awaiting_input`.
///   * `Message` — fired for each outbound message.
///
/// Observers are awaited in registration order (fast, in-process consumers); a
/// failing observer is logged and never fails the underlying task op.
pub struct ObservingTaskRegistry {
    inner: Arc<dyn SessionTaskRegistry>,
    observers: Vec<Arc<dyn TaskTransitionObserver>>,
    update_locks: Mutex<TaskUpdateLocks>,
}

impl ObservingTaskRegistry {
    pub fn new(inner: Arc<dyn SessionTaskRegistry>) -> Self {
        Self {
            inner,
            observers: Vec::new(),
            update_locks: Mutex::new(HashMap::new()),
        }
    }

    /// Register an observer to receive every real transition.
    pub fn with_observer(mut self, observer: Arc<dyn TaskTransitionObserver>) -> Self {
        self.observers.push(observer);
        self
    }

    /// Whether any observer is registered (fan-out is otherwise a no-op).
    pub fn has_observers(&self) -> bool {
        !self.observers.is_empty()
    }

    fn task_lock(&self, session_id: SessionId, task_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self
            .update_locks
            .lock()
            .expect("task update locks poisoned");
        locks.retain(|_, lock| lock.strong_count() > 0);
        let key = (session_id, task_id.to_string());
        if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        locks.insert(key, Arc::downgrade(&lock));
        lock
    }

    async fn notify(&self, task: &SessionTask, transition: TaskTransition) {
        for observer in &self.observers {
            if let Err(e) = observer.on_transition(task, transition).await {
                tracing::warn!(
                    task_id = %task.id,
                    session_id = %task.session_id,
                    transition = ?transition,
                    "TaskTransitionObserver failed (best-effort): {e}"
                );
            }
        }
    }
}

#[async_trait]
impl SessionTaskRegistry for ObservingTaskRegistry {
    async fn create(&self, input: CreateSessionTask) -> Result<SessionTask> {
        self.inner.create(input).await
    }

    async fn update(
        &self,
        session_id: SessionId,
        task_id: &str,
        update: SessionTaskUpdate,
    ) -> Result<Option<SessionTask>> {
        // Keep the prior snapshot and mutation together, so competing writers
        // cannot both claim the same transition. Callbacks run after unlocking.
        let task_lock = self
            .has_observers()
            .then(|| self.task_lock(session_id, task_id));
        let guard = match task_lock.as_ref() {
            Some(lock) => Some(lock.lock().await),
            None => None,
        };
        // Only this update's own intent can trigger a wake, so a racing
        // heartbeat/progress update never fires on another writer's transition.
        let wants_terminal = update.state.is_some_and(|s| s.is_terminal());
        let wants_awaiting_input =
            update.input_request.is_some() || update.state == Some(SessionTaskState::AwaitingInput);

        let needs_prior = self.has_observers() && (wants_terminal || wants_awaiting_input);
        let prior = if needs_prior {
            self.inner.get(session_id, task_id).await.ok().flatten()
        } else {
            None
        };

        let updated = self.inner.update(session_id, task_id, update).await?;
        drop(guard);

        if let (Some(task), Some(prior)) = (&updated, &prior) {
            if wants_terminal && !prior.state.is_terminal() && task.state.is_terminal() {
                self.notify(task, TaskTransition::Terminal).await;
            }
            if wants_awaiting_input
                && prior.state != SessionTaskState::AwaitingInput
                && task.state == SessionTaskState::AwaitingInput
            {
                self.notify(task, TaskTransition::AwaitingInput).await;
            }
        }
        Ok(updated)
    }

    async fn get(&self, session_id: SessionId, task_id: &str) -> Result<Option<SessionTask>> {
        self.inner.get(session_id, task_id).await
    }

    async fn list(
        &self,
        session_id: SessionId,
        filter: Option<&SessionTaskFilter>,
    ) -> Result<Vec<SessionTask>> {
        self.inner.list(session_id, filter).await
    }

    async fn request_cancel(
        &self,
        session_id: SessionId,
        task_id: &str,
    ) -> Result<Option<SessionTask>> {
        self.inner.request_cancel(session_id, task_id).await
    }

    async fn record_message(
        &self,
        session_id: SessionId,
        task_id: &str,
        message: NewTaskMessage,
    ) -> Result<TaskMessage> {
        // Inbound replies can leave awaiting_input, so message writes share
        // the task update lock even when they do not notify observers.
        let task_lock = self
            .has_observers()
            .then(|| self.task_lock(session_id, task_id));
        let guard = match task_lock.as_ref() {
            Some(lock) => Some(lock.lock().await),
            None => None,
        };
        let direction = message.direction;
        let stored = self
            .inner
            .record_message(session_id, task_id, message)
            .await?;
        let task = if direction == TaskMessageDirection::Outbound && self.has_observers() {
            self.inner.get(session_id, task_id).await.ok().flatten()
        } else {
            None
        };
        drop(guard);
        if let Some(task) = task {
            self.notify(&task, TaskTransition::Message).await;
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
        self.inner
            .list_messages(session_id, task_id, limit, after_id)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_value_and_event_name_are_stable() {
        // These strings are a wire contract shared with webhook payloads and the
        // per-task `event_filter`; changing them silently breaks delivery.
        assert_eq!(TaskTransition::Terminal.filter_value(), "terminal");
        assert_eq!(
            TaskTransition::AwaitingInput.filter_value(),
            "awaiting_input"
        );
        assert_eq!(TaskTransition::Message.filter_value(), "message");

        assert_eq!(TaskTransition::Terminal.event_name(), "task.terminal");
        assert_eq!(
            TaskTransition::AwaitingInput.event_name(),
            "task.awaiting_input"
        );
        assert_eq!(TaskTransition::Message.event_name(), "task.message");
    }

    // ---- ObservingTaskRegistry fan-out gating -----------------------------

    use crate::session_task::{
        SessionTaskState, TaskWakePolicy, apply_task_update, new_session_task,
    };
    use crate::typed_id::SessionId;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemRegistry {
        tasks: Mutex<HashMap<String, SessionTask>>,
        yield_after_read: bool,
    }

    #[async_trait]
    impl SessionTaskRegistry for MemRegistry {
        async fn create(&self, input: CreateSessionTask) -> Result<SessionTask> {
            let task = new_session_task(input, chrono::Utc::now());
            self.tasks
                .lock()
                .unwrap()
                .insert(task.id.clone(), task.clone());
            Ok(task)
        }
        async fn update(
            &self,
            session_id: SessionId,
            task_id: &str,
            update: SessionTaskUpdate,
        ) -> Result<Option<SessionTask>> {
            let mut tasks = self.tasks.lock().unwrap();
            let Some(task) = tasks.get_mut(task_id) else {
                return Ok(None);
            };
            if task.session_id != session_id {
                return Ok(None);
            }
            apply_task_update(task, update, chrono::Utc::now());
            Ok(Some(task.clone()))
        }
        async fn get(&self, session_id: SessionId, task_id: &str) -> Result<Option<SessionTask>> {
            let task = self
                .tasks
                .lock()
                .unwrap()
                .get(task_id)
                .filter(|t| t.session_id == session_id)
                .cloned();
            if self.yield_after_read {
                tokio::task::yield_now().await;
            }
            Ok(task)
        }
        async fn list(
            &self,
            _session_id: SessionId,
            _filter: Option<&SessionTaskFilter>,
        ) -> Result<Vec<SessionTask>> {
            Ok(Vec::new())
        }
        async fn request_cancel(
            &self,
            _session_id: SessionId,
            _task_id: &str,
        ) -> Result<Option<SessionTask>> {
            Ok(None)
        }
        async fn record_message(
            &self,
            _session_id: SessionId,
            task_id: &str,
            message: NewTaskMessage,
        ) -> Result<TaskMessage> {
            Ok(TaskMessage {
                id: "tmsg_x".into(),
                task_id: task_id.into(),
                direction: message.direction,
                content: message.content,
                in_reply_to: message.in_reply_to,
                created_at: chrono::Utc::now(),
            })
        }
        async fn list_messages(
            &self,
            _session_id: SessionId,
            _task_id: &str,
            _limit: Option<u32>,
            _after_id: Option<&str>,
        ) -> Result<Vec<TaskMessage>> {
            Ok(Vec::new())
        }
    }

    #[derive(Default)]
    struct Recorder {
        seen: Mutex<Vec<TaskTransition>>,
        snapshots: Mutex<Vec<serde_json::Value>>,
    }

    #[async_trait]
    impl TaskTransitionObserver for Recorder {
        async fn on_transition(
            &self,
            task: &SessionTask,
            transition: TaskTransition,
        ) -> anyhow::Result<()> {
            self.snapshots
                .lock()
                .unwrap()
                .push(serde_json::to_value(task).unwrap());
            self.seen.lock().unwrap().push(transition);
            Ok(())
        }
    }

    async fn seed_running(reg: &MemRegistry, session_id: SessionId) -> String {
        reg.create(CreateSessionTask {
            id: None,
            session_id,
            kind: "background_tool".into(),
            display_name: "T".into(),
            spec: serde_json::Value::Null,
            state: SessionTaskState::Running,
            links: Default::default(),
            wake_policy: TaskWakePolicy::OnActivity,
        })
        .await
        .unwrap()
        .id
    }

    #[tokio::test]
    async fn fires_terminal_once_and_not_on_heartbeat() {
        let inner = Arc::new(MemRegistry::default());
        let recorder = Arc::new(Recorder::default());
        let reg = ObservingTaskRegistry::new(inner.clone()).with_observer(recorder.clone());
        let session_id = SessionId::new();
        let task_id = seed_running(&inner, session_id).await;

        // A heartbeat-only update (no state change) must not fire any transition.
        reg.update(
            session_id,
            &task_id,
            SessionTaskUpdate {
                heartbeat_at: Some(chrono::Utc::now()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(
            recorder.seen.lock().unwrap().is_empty(),
            "heartbeat must not fire a transition"
        );

        // Transition to terminal fires exactly one Terminal.
        reg.update(
            session_id,
            &task_id,
            SessionTaskUpdate {
                state: Some(SessionTaskState::Succeeded),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // A redundant terminal-state update on an already-terminal task must not
        // re-fire (prior is already terminal).
        reg.update(
            session_id,
            &task_id,
            SessionTaskUpdate {
                state: Some(SessionTaskState::Succeeded),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(
            *recorder.seen.lock().unwrap(),
            vec![TaskTransition::Terminal],
            "terminal fires exactly once, never on heartbeat or re-terminal"
        );
    }

    #[tokio::test]
    async fn fires_awaiting_input_only_on_entry() {
        let inner = Arc::new(MemRegistry::default());
        let recorder = Arc::new(Recorder::default());
        let reg = ObservingTaskRegistry::new(inner.clone()).with_observer(recorder.clone());
        let session_id = SessionId::new();
        let task_id = seed_running(&inner, session_id).await;

        reg.update(
            session_id,
            &task_id,
            SessionTaskUpdate {
                input_request: Some(crate::session_task::TaskInputRequest {
                    id: "ir_1".into(),
                    prompt: "approve?".into(),
                    expected: None,
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(
            *recorder.seen.lock().unwrap(),
            vec![TaskTransition::AwaitingInput],
            "awaiting_input fires once on entry"
        );
        reg.update(
            session_id,
            &task_id,
            SessionTaskUpdate {
                state: Some(SessionTaskState::AwaitingInput),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            *recorder.seen.lock().unwrap(),
            vec![TaskTransition::AwaitingInput]
        );
        reg.update(
            session_id,
            &task_id,
            SessionTaskUpdate {
                state: Some(SessionTaskState::Running),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let updated = reg
            .update(
                session_id,
                &task_id,
                SessionTaskUpdate {
                    input_request: Some(crate::session_task::TaskInputRequest {
                        id: "ir_2".into(),
                        prompt: "choose again".into(),
                        expected: None,
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            *recorder.seen.lock().unwrap(),
            vec![TaskTransition::AwaitingInput, TaskTransition::AwaitingInput]
        );
        assert_eq!(
            recorder.snapshots.lock().unwrap().last().unwrap(),
            &serde_json::to_value(updated).unwrap()
        );
    }
    #[tokio::test]
    async fn competing_terminal_updates_emit_one_transition() {
        let inner = Arc::new(MemRegistry {
            yield_after_read: true,
            ..Default::default()
        });
        let recorder = Arc::new(Recorder::default());
        let reg = ObservingTaskRegistry::new(inner.clone()).with_observer(recorder.clone());
        let session = SessionId::from_seed(1);
        let task = seed_running(&inner, session).await;
        let update = || SessionTaskUpdate {
            state: Some(SessionTaskState::Succeeded),
            ..Default::default()
        };
        let (first, second) = tokio::join!(
            reg.update(session, &task, update()),
            reg.update(session, &task, update())
        );
        assert_eq!(first.unwrap().unwrap().state, SessionTaskState::Succeeded);
        assert_eq!(second.unwrap().unwrap().state, SessionTaskState::Succeeded);
        assert_eq!(
            *recorder.seen.lock().unwrap(),
            vec![TaskTransition::Terminal]
        );
    }
    #[derive(Default)]
    struct FailingObserver {
        registry: Mutex<Weak<ObservingTaskRegistry>>,
    }
    #[async_trait]
    impl TaskTransitionObserver for FailingObserver {
        async fn on_transition(&self, task: &SessionTask, _: TaskTransition) -> anyhow::Result<()> {
            let registry = self.registry.lock().unwrap().upgrade().unwrap();
            let lock = registry.task_lock(task.session_id, &task.id);
            assert!(
                lock.try_lock().is_ok(),
                "callbacks must run outside the update lock"
            );
            anyhow::bail!("observer unavailable")
        }
    }

    #[tokio::test]
    async fn outbound_messages_preserve_payload_and_survive_observer_failure() {
        let inner = Arc::new(MemRegistry::default());
        let recorder = Arc::new(Recorder::default());
        let failing = Arc::new(FailingObserver::default());
        let reg = Arc::new(
            ObservingTaskRegistry::new(inner.clone())
                .with_observer(failing.clone())
                .with_observer(recorder.clone()),
        );
        *failing.registry.lock().unwrap() = Arc::downgrade(&reg);
        let session = SessionId::from_seed(2);
        let task = seed_running(&inner, session).await;
        let inbound = reg
            .record_message(session, &task, NewTaskMessage::inbound_text("answer"))
            .await
            .unwrap();
        assert_eq!(inbound.direction, TaskMessageDirection::Inbound);
        assert!(recorder.seen.lock().unwrap().is_empty());
        for text in ["progress α", "finished"] {
            let mut message = NewTaskMessage::outbound_text(text);
            message.in_reply_to = Some("request_1".into());
            let saved = reg.record_message(session, &task, message).await.unwrap();
            assert_eq!(saved.task_id, task);
            assert_eq!(saved.direction, TaskMessageDirection::Outbound);
            assert_eq!(
                saved.content,
                vec![crate::session_task::TaskMessagePart::text(text)]
            );
            assert_eq!(saved.in_reply_to.as_deref(), Some("request_1"));
        }
        assert_eq!(
            *recorder.seen.lock().unwrap(),
            vec![TaskTransition::Message, TaskTransition::Message]
        );
        let snapshot =
            serde_json::to_value(inner.get(session, &task).await.unwrap().unwrap()).unwrap();
        assert_eq!(
            *recorder.snapshots.lock().unwrap(),
            vec![snapshot.clone(), snapshot]
        );
        let updated = reg
            .update(
                session,
                &task,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Failed),
                    summary: Some("failed work".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.state, SessionTaskState::Failed);
        assert_eq!(
            recorder.seen.lock().unwrap().last(),
            Some(&TaskTransition::Terminal)
        );
        assert_eq!(
            recorder.snapshots.lock().unwrap().last().unwrap(),
            &serde_json::to_value(updated).unwrap()
        );
    }

    #[test]
    fn task_locks_isolate_keys_and_release_idle_entries() {
        let reg = ObservingTaskRegistry::new(Arc::new(MemRegistry::default()));
        let first = reg.task_lock(SessionId::from_seed(1), "task_a");
        let same = reg.task_lock(SessionId::from_seed(1), "task_a");
        let other_task = reg.task_lock(SessionId::from_seed(1), "task_b");
        let other_session = reg.task_lock(SessionId::from_seed(2), "task_a");
        let guard = first.try_lock().unwrap();
        assert!(same.try_lock().is_err());
        assert!(other_task.try_lock().is_ok());
        assert!(other_session.try_lock().is_ok());
        drop(guard);
        drop((first, same, other_task, other_session));
        let next = reg.task_lock(SessionId::from_seed(3), "task_c");
        assert_eq!(reg.update_locks.lock().unwrap().len(), 1);
        assert!(next.try_lock().is_ok());
    }
}
