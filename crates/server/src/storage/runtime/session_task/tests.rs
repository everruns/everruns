use super::*;
use everruns_contracts::typed_id::{EventId, MessageId, TurnId};
use everruns_core::events::Event;
use everruns_core::session_task::{
    TaskInputRequest, TaskLinks, TaskMessagePart, TaskWakePolicy, TurnCorrelatedSessionTaskRegistry,
};
use std::sync::Mutex;

#[derive(Default)]
struct RecordingEmitter {
    requests: Mutex<Vec<EventRequest>>,
}

#[async_trait::async_trait]
impl EventEmitter for RecordingEmitter {
    async fn emit(&self, request: EventRequest) -> Result<Event> {
        self.requests.lock().unwrap().push(request.clone());
        Ok(Event {
            id: EventId::new(),
            event_type: request.event_type,
            ts: request.ts,
            session_id: request.session_id,
            context: request.context,
            data: request.data,
            metadata: request.metadata,
            tags: request.tags,
            sequence: None,
        })
    }
}

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
    DbSessionTaskRegistry::new(Arc::new(StorageBackend::test_database())).with_waker(waker)
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
    DbSessionTaskRegistry::new(Arc::new(StorageBackend::test_database()))
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
    DbSessionTaskRegistry::new(Arc::new(StorageBackend::test_database()))
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
async fn lifecycle_events_keep_concurrent_turn_origins_isolated() {
    let emitter = Arc::new(RecordingEmitter::default());
    let db = Arc::new(StorageBackend::test_database());
    let session_id = db.create_test_session().await;
    let base: Arc<dyn SessionTaskRegistry> =
        Arc::new(DbSessionTaskRegistry::new(db).with_event_emitter(emitter.clone()));
    let first_message = MessageId::from_seed(1);
    let second_message = MessageId::from_seed(2);
    let first = TurnCorrelatedSessionTaskRegistry::wrap(
        base.clone(),
        EventContext::turn(TurnId::from_seed(1), first_message),
    );
    let second = TurnCorrelatedSessionTaskRegistry::wrap(
        base,
        EventContext::turn(TurnId::from_seed(2), second_message),
    );

    let first_task = first.create(create_input(session_id)).await.unwrap();
    second.create(create_input(session_id)).await.unwrap();
    first
        .update(
            session_id,
            &first_task.id,
            SessionTaskUpdate {
                state: Some(SessionTaskState::Running),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let requests = emitter.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].context.input_message_id, Some(first_message));
    assert_eq!(requests[1].context.input_message_id, Some(second_message));
    assert_eq!(requests[2].context.input_message_id, Some(first_message));
}

#[tokio::test]
async fn sink_artifacts_survive_storage_conversion_and_attempt_fencing() {
    use everruns_core::session_task::{RegistryTaskSink, TaskArtifact, TaskSink};
    let registry = Arc::new(registry());
    let session_id = registry.db.create_test_session().await;
    let task = registry.create(create_input(session_id)).await.unwrap();
    let sink = RegistryTaskSink::new(registry.clone(), session_id, task.id.clone());
    let first = TaskArtifact {
        name: "first".into(),
        artifact_type: "file".into(),
        path: Some("/first".into()),
        url: None,
    };
    let second = TaskArtifact {
        name: "second".into(),
        artifact_type: "url".into(),
        path: None,
        url: Some("https://example.com/result".into()),
    };
    sink.artifact(first.clone()).await.unwrap();
    sink.artifact(second.clone()).await.unwrap();
    let stored = registry.get(session_id, &task.id).await.unwrap().unwrap();
    assert_eq!(stored.artifacts, [first.clone(), second.clone()]);
    registry
        .update(
            session_id,
            &task.id,
            SessionTaskUpdate {
                increment_attempt: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    sink.artifact(first).await.unwrap();
    let fenced = registry.get(session_id, &task.id).await.unwrap().unwrap();
    assert_eq!(fenced.artifacts, stored.artifacts);
    assert_eq!(fenced.attempt, 2);
    let other = RegistryTaskSink::new(
        registry.clone(),
        registry.db.create_test_session().await,
        task.id.clone(),
    );
    other.artifact(second).await.unwrap();
    assert_eq!(
        registry
            .get(session_id, &task.id)
            .await
            .unwrap()
            .unwrap()
            .artifacts,
        stored.artifacts
    );
}

#[tokio::test]
async fn create_is_idempotent_on_id() {
    let registry = registry();
    let session_id = registry.db.create_test_session().await;
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
    let mut input = create_input(registry.db.create_test_session().await);
    input.id = Some("task_shared".to_string());
    registry.create(input.clone()).await.unwrap();

    let mut other = create_input(registry.db.create_test_session().await);
    other.id = Some("task_shared".to_string());
    assert!(registry.create(other).await.is_err());
}

#[tokio::test]
async fn update_applies_lifecycle_invariants() {
    let registry = registry();
    let session_id = registry.db.create_test_session().await;
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
    let session_id = registry.db.create_test_session().await;
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
    let session_id = registry.db.create_test_session().await;
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
    let session_id = registry.db.create_test_session().await;
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
    let session_id = registry.db.create_test_session().await;
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
    let session_id = registry.db.create_test_session().await;
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

fn create_input_with_policy(session_id: SessionId, policy: TaskWakePolicy) -> CreateSessionTask {
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
    let session_id = registry.db.create_test_session().await;

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
    let session_id = registry.db.create_test_session().await;

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
    let session_id = registry.db.create_test_session().await;

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
    let session_id = registry.db.create_test_session().await;

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
    // `webhook` stands in for the server's TaskWebhookNotifier; both
    // are just `TaskTransitionObserver`s after EVE-729.
    let webhook = Arc::new(RecordingObserver::default());
    let in_process = Arc::new(RecordingObserver::default());
    let registry = DbSessionTaskRegistry::new(Arc::new(StorageBackend::test_database()))
        .with_transition_observer(webhook.clone())
        .with_transition_observer(in_process.clone());
    let session_id = registry.db.create_test_session().await;

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
    let session_id = registry.db.create_test_session().await;

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
