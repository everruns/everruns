// Task registry invariants + restart-survivability over a file-backed DB.

use everruns::local::{LocalSessionTaskRegistry, SqliteDb};
use everruns_contracts::typed_id::SessionId;
use everruns_core::session_task::{
    CreateSessionTask, NewTaskMessage, SessionTask, SessionTaskFilter, SessionTaskRegistry,
    SessionTaskState, SessionTaskUpdate, TASK_KIND_BACKGROUND_TOOL, TASK_KIND_SUBAGENT,
    TaskInputRequest, TaskLinks, TaskWakePolicy, apply_task_update, new_session_task,
};

fn create_input(session_id: SessionId, kind: &str) -> CreateSessionTask {
    CreateSessionTask {
        session_id,
        id: None,
        kind: kind.to_string(),
        display_name: "Test Task".to_string(),
        spec: serde_json::json!({"instructions": "do the thing"}),
        state: SessionTaskState::Queued,
        links: TaskLinks::default(),
        wake_policy: TaskWakePolicy::Silent,
    }
}

#[tokio::test]
async fn create_is_idempotent_on_supplied_id() {
    let reg = LocalSessionTaskRegistry::new(SqliteDb::open_in_memory().unwrap()).unwrap();
    let session_id = SessionId::new();
    let mut input = create_input(session_id, TASK_KIND_BACKGROUND_TOOL);
    input.id = Some("task_fixed_1".to_string());

    let first = reg.create(input.clone()).await.unwrap();
    // Mutate state, then re-create the same id: must return the stored task.
    reg.update(
        session_id,
        &first.id,
        SessionTaskUpdate {
            state: Some(SessionTaskState::Running),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let again = reg.create(input).await.unwrap();
    assert_eq!(again.id, "task_fixed_1");
    assert_eq!(
        again.state,
        SessionTaskState::Running,
        "create is idempotent"
    );
}

#[tokio::test]
async fn task_spec_webhook_secret_survives_storage_round_trip() {
    let reg = LocalSessionTaskRegistry::new(SqliteDb::open_in_memory().unwrap()).unwrap();
    let session_id = SessionId::new();
    let mut input = create_input(session_id, TASK_KIND_SUBAGENT);
    input.spec = serde_json::json!({
        "push_configs": [{
            "url": "https://hooks.example.com/everruns",
            "secret": "spawn-time-hmac-secret",
            "event_filter": ["terminal"]
        }]
    });

    let created = reg.create(input).await.unwrap();
    let loaded = reg.get(session_id, &created.id).await.unwrap().unwrap();

    assert_eq!(
        loaded.spec["push_configs"][0]["secret"],
        "spawn-time-hmac-secret"
    );
    assert!(loaded.spec["push_configs"][0].get("has_secret").is_none());
}

#[tokio::test]
async fn create_rejects_id_reuse_across_sessions() {
    let reg = LocalSessionTaskRegistry::new(SqliteDb::open_in_memory().unwrap()).unwrap();
    let session_a = SessionId::new();
    let session_b = SessionId::new();

    let mut input_a = create_input(session_a, TASK_KIND_BACKGROUND_TOOL);
    input_a.id = Some("task_shared_id".to_string());
    reg.create(input_a).await.unwrap();

    // Reusing the same id under a different session must be rejected, not
    // silently aliased to session A's task.
    let mut input_b = create_input(session_b, TASK_KIND_BACKGROUND_TOOL);
    input_b.id = Some("task_shared_id".to_string());
    assert!(
        reg.create(input_b).await.is_err(),
        "cross-session id reuse must be rejected"
    );

    // Lookups are session-scoped: the task is invisible from session B and the
    // foreign session cannot read its message history.
    assert!(
        reg.get(session_b, "task_shared_id")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        reg.get(session_a, "task_shared_id")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        reg.list_messages(session_b, "task_shared_id", None, None)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn started_at_and_finished_at_invariants() {
    let reg = LocalSessionTaskRegistry::new(SqliteDb::open_in_memory().unwrap()).unwrap();
    let session_id = SessionId::new();
    let task = reg
        .create(create_input(session_id, TASK_KIND_BACKGROUND_TOOL))
        .await
        .unwrap();
    assert!(task.started_at.is_none());

    let running = reg
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
    assert!(running.started_at.is_some(), "started_at stamped");
    assert!(running.finished_at.is_none());

    let done = reg
        .update(
            session_id,
            &task.id,
            SessionTaskUpdate {
                state: Some(SessionTaskState::Succeeded),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert!(done.finished_at.is_some(), "finished_at stamped");

    // Terminal state is final: a different state is ignored.
    let after = reg
        .update(
            session_id,
            &task.id,
            SessionTaskUpdate {
                state: Some(SessionTaskState::Failed),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        after.state,
        SessionTaskState::Succeeded,
        "terminal is final"
    );
}

#[tokio::test]
async fn input_request_forces_awaiting_input() {
    let reg = LocalSessionTaskRegistry::new(SqliteDb::open_in_memory().unwrap()).unwrap();
    let session_id = SessionId::new();
    let task = reg
        .create(create_input(session_id, TASK_KIND_SUBAGENT))
        .await
        .unwrap();

    let awaiting = reg
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
        .unwrap()
        .unwrap();
    assert_eq!(awaiting.state, SessionTaskState::AwaitingInput);
    assert!(awaiting.input_request.is_some());

    // Answering message clears the pending request and returns to running.
    reg.record_message(
        session_id,
        &task.id,
        NewTaskMessage {
            direction: everruns_core::session_task::TaskMessageDirection::Inbound,
            content: vec![everruns_core::session_task::TaskMessagePart::text("yes")],
            in_reply_to: Some("req_1".to_string()),
            expected_attempt: None,
        },
    )
    .await
    .unwrap();
    let resumed = reg.get(session_id, &task.id).await.unwrap().unwrap();
    assert_eq!(resumed.state, SessionTaskState::Running);
    assert!(resumed.input_request.is_none());
}

#[tokio::test]
async fn request_cancel_records_intent_without_state_change() {
    let reg = LocalSessionTaskRegistry::new(SqliteDb::open_in_memory().unwrap()).unwrap();
    let session_id = SessionId::new();
    let task = reg
        .create(create_input(session_id, TASK_KIND_BACKGROUND_TOOL))
        .await
        .unwrap();
    let canceled = reg
        .request_cancel(session_id, &task.id)
        .await
        .unwrap()
        .unwrap();
    assert!(canceled.cancel_requested_at.is_some());
    assert_eq!(canceled.state, SessionTaskState::Queued, "state unchanged");
}

#[tokio::test]
async fn list_filters_by_kind_and_state() {
    let reg = LocalSessionTaskRegistry::new(SqliteDb::open_in_memory().unwrap()).unwrap();
    let session_id = SessionId::new();
    let bg = reg
        .create(create_input(session_id, TASK_KIND_BACKGROUND_TOOL))
        .await
        .unwrap();
    let _sub = reg
        .create(create_input(session_id, TASK_KIND_SUBAGENT))
        .await
        .unwrap();
    reg.update(
        session_id,
        &bg.id,
        SessionTaskUpdate {
            state: Some(SessionTaskState::Running),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let by_kind = reg
        .list(
            session_id,
            Some(&SessionTaskFilter {
                kind: Some(TASK_KIND_SUBAGENT.to_string()),
                state: None,
            }),
        )
        .await
        .unwrap();
    assert_eq!(by_kind.len(), 1);
    assert_eq!(by_kind[0].kind, TASK_KIND_SUBAGENT);

    let by_state = reg
        .list(
            session_id,
            Some(&SessionTaskFilter {
                kind: None,
                state: Some(SessionTaskState::Running),
            }),
        )
        .await
        .unwrap();
    assert_eq!(by_state.len(), 1);
    assert_eq!(by_state[0].id, bg.id);

    let all = reg.list(session_id, None).await.unwrap();
    assert_eq!(all.len(), 2);
}

#[tokio::test]
async fn messages_persist_and_support_after_cursor() {
    let reg = LocalSessionTaskRegistry::new(SqliteDb::open_in_memory().unwrap()).unwrap();
    let session_id = SessionId::new();
    let task = reg
        .create(create_input(session_id, TASK_KIND_SUBAGENT))
        .await
        .unwrap();

    let m1 = reg
        .record_message(session_id, &task.id, NewTaskMessage::outbound_text("one"))
        .await
        .unwrap();
    let _m2 = reg
        .record_message(session_id, &task.id, NewTaskMessage::outbound_text("two"))
        .await
        .unwrap();

    let all = reg
        .list_messages(session_id, &task.id, None, None)
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(
        everruns_core::session_task::task_message_text(&all[0].content),
        "one"
    );

    let after = reg
        .list_messages(session_id, &task.id, None, Some(&m1.id))
        .await
        .unwrap();
    assert_eq!(after.len(), 1, "after-cursor excludes m1");
    assert_eq!(
        everruns_core::session_task::task_message_text(&after[0].content),
        "two"
    );

    let limited = reg
        .list_messages(session_id, &task.id, Some(1), None)
        .await
        .unwrap();
    assert_eq!(limited.len(), 1);
}

#[tokio::test]
async fn restart_survivability_reopen_file_backed_db() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tasks.db");
    let session_id = SessionId::new();
    let task_id;

    // First process lifetime: create a task + message, then drop everything.
    {
        let reg = LocalSessionTaskRegistry::new(SqliteDb::open(&path).unwrap()).unwrap();
        let task = reg
            .create(create_input(session_id, TASK_KIND_SUBAGENT))
            .await
            .unwrap();
        task_id = task.id.clone();
        reg.update(
            session_id,
            &task_id,
            SessionTaskUpdate {
                state: Some(SessionTaskState::Running),
                state_detail: Some("iteration 1".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        reg.record_message(
            session_id,
            &task_id,
            NewTaskMessage::outbound_text("progress"),
        )
        .await
        .unwrap();
        // reg + db dropped here.
    }

    // Second process lifetime: reopen the same file and continue.
    let reg = LocalSessionTaskRegistry::new(SqliteDb::open(&path).unwrap()).unwrap();
    let reread = reg.get(session_id, &task_id).await.unwrap().unwrap();
    assert_eq!(reread.state, SessionTaskState::Running);
    assert_eq!(reread.state_detail.as_deref(), Some("iteration 1"));

    let messages = reg
        .list_messages(session_id, &task_id, None, None)
        .await
        .unwrap();
    assert_eq!(messages.len(), 1, "message survived restart");

    // Continue the task to completion after restart.
    let done = reg
        .update(
            session_id,
            &task_id,
            SessionTaskUpdate {
                state: Some(SessionTaskState::Succeeded),
                summary: Some("done after restart".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.state, SessionTaskState::Succeeded);
    assert_eq!(done.summary.as_deref(), Some("done after restart"));
}

#[tokio::test]
async fn record_message_rejects_stale_attempt() {
    let reg = LocalSessionTaskRegistry::new(SqliteDb::open_in_memory().unwrap()).unwrap();
    let session_id = SessionId::new();
    let task = reg
        .create(create_input(session_id, TASK_KIND_SUBAGENT))
        .await
        .unwrap();
    // Task attempt is 1; a message fenced on attempt 2 must be rejected.
    let err = reg
        .record_message(
            session_id,
            &task.id,
            NewTaskMessage::outbound_text("zombie").with_expected_attempt(2),
        )
        .await;
    assert!(err.is_err(), "stale-attempt message must be rejected");
}

// The busy handler marks the exact point where a second connection tries to
// acquire the writer lock. Commit the first writer there, without timing sleeps.
// A read outside the transaction sees the old snapshot and overwrites it;
// BEGIN IMMEDIATE waits here before reading and sees the committed snapshot.
type WriterGate = (
    std::sync::mpsc::SyncSender<()>,
    std::sync::mpsc::Receiver<()>,
);

thread_local! {
    static WRITER_GATE: std::cell::RefCell<Option<WriterGate>> = const { std::cell::RefCell::new(None) };
}

fn wait_for_writer_commit(attempt: i32) -> bool {
    if attempt != 0 {
        return false;
    }
    WRITER_GATE.with(|gate| {
        let Some((blocked, resume)) = gate.borrow_mut().take() else {
            return false;
        };
        blocked.send(()).is_ok()
            && resume
                .recv_timeout(std::time::Duration::from_secs(10))
                .is_ok()
    })
}

fn commit_before_registry_write<F, Fut, T>(
    worker_db: SqliteDb,
    writer: &mut everruns::sqlite::Connection,
    committed: &SessionTask,
    operation: F,
) -> T
where
    F: FnOnce(LocalSessionTaskRegistry) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = T>,
    T: Send + 'static,
{
    // Migrate before taking the writer lock so the busy handler observes the
    // operation under test, not registry construction.
    let registry = LocalSessionTaskRegistry::new(worker_db.clone()).unwrap();
    let tx = writer
        .transaction_with_behavior(everruns::sqlite::TransactionBehavior::Immediate)
        .unwrap();
    let mut snapshot = serde_json::to_value(committed).unwrap();
    snapshot["spec"] = committed.spec.clone();
    tx.execute(
        "INSERT INTO local_tasks (id, session_id, kind, state, snapshot)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET
            session_id = excluded.session_id,
            kind = excluded.kind,
            state = excluded.state,
            snapshot = excluded.snapshot",
        everruns::sqlite::params![
            committed.id,
            committed.session_id.to_string(),
            committed.kind,
            committed.state.to_string(),
            serde_json::to_string(&snapshot).unwrap(),
        ],
    )
    .unwrap();
    let (blocked_tx, blocked_rx) = std::sync::mpsc::sync_channel(1);
    let (resume_tx, resume_rx) = std::sync::mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        WRITER_GATE.with(|gate| *gate.borrow_mut() = Some((blocked_tx, resume_rx)));
        worker_db
            .with_conn(|conn| conn.busy_handler(Some(wait_for_writer_commit)))
            .unwrap();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(operation(registry))
    });
    blocked_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("registry operation must contend with the separate connection's writer lock");
    tx.commit().unwrap();
    resume_tx.send(()).unwrap();
    worker.join().unwrap()
}

fn independent_registries() -> (
    tempfile::TempDir,
    SqliteDb,
    everruns::sqlite::Connection,
    LocalSessionTaskRegistry,
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tasks.db");
    let worker = SqliteDb::open(&path).unwrap();
    let writer = everruns::sqlite::open(&path).unwrap();
    let registry = LocalSessionTaskRegistry::new(worker.clone()).unwrap();
    (dir, worker, writer, registry)
}

fn assert_same_task(actual: &SessionTask, expected: &SessionTask) {
    assert_eq!(
        serde_json::to_value(actual).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
    assert_eq!(actual.spec, expected.spec);
}

#[test]
fn concurrent_create_is_session_scoped_and_idempotent() {
    for same_session in [true, false] {
        let (_dir, worker, mut writer, registry) = independent_registries();
        let session_id = SessionId::new();
        let mut input = create_input(session_id, TASK_KIND_BACKGROUND_TOOL);
        input.id = Some("task_concurrent_create".into());
        let mut first_input = input.clone();
        first_input.session_id = if same_session {
            session_id
        } else {
            SessionId::new()
        };
        first_input.state = SessionTaskState::Running;
        first_input.display_name = "First writer".into();
        let committed = new_session_task(first_input, chrono::Utc::now());
        let result =
            commit_before_registry_write(worker, &mut writer, &committed, move |reg| async move {
                reg.create(input).await
            });
        if same_session {
            assert_same_task(&result.unwrap(), &committed);
        } else {
            assert!(result.is_err(), "another session cannot take over the id");
        }
        let stored = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(registry.get(committed.session_id, &committed.id))
            .unwrap()
            .unwrap();
        assert_same_task(&stored, &committed);
    }
}

#[tokio::test]
async fn concurrent_update_observes_attempt_fence() {
    let (_dir, worker, mut writer, registry) = independent_registries();
    let session_id = SessionId::new();
    let mut input = create_input(session_id, TASK_KIND_BACKGROUND_TOOL);
    input.state = SessionTaskState::Running;
    let mut committed = registry.create(input).await.unwrap();
    let expected_attempt = committed.attempt;
    apply_task_update(
        &mut committed,
        SessionTaskUpdate {
            increment_attempt: true,
            summary: Some("new attempt".into()),
            ..Default::default()
        },
        chrono::Utc::now(),
    );
    let id = committed.id.clone();
    let updated =
        commit_before_registry_write(worker, &mut writer, &committed, move |reg| async move {
            reg.update(
                session_id,
                &id,
                SessionTaskUpdate {
                    expected_attempt: Some(expected_attempt),
                    summary: Some("stale executor".into()),
                    heartbeat_at: Some(chrono::Utc::now()),
                    ..Default::default()
                },
            )
            .await
        })
        .unwrap()
        .unwrap();
    assert_same_task(&updated, &committed);
    assert_same_task(
        &registry
            .get(session_id, &committed.id)
            .await
            .unwrap()
            .unwrap(),
        &committed,
    );
}

#[tokio::test]
async fn concurrent_update_preserves_terminal_record() {
    let (_dir, worker, mut writer, registry) = independent_registries();
    let session_id = SessionId::new();
    let mut input = create_input(session_id, TASK_KIND_BACKGROUND_TOOL);
    input.state = SessionTaskState::Running;
    let mut committed = registry.create(input).await.unwrap();
    apply_task_update(
        &mut committed,
        SessionTaskUpdate {
            state: Some(SessionTaskState::Succeeded),
            summary: Some("completed successfully".into()),
            ..Default::default()
        },
        chrono::Utc::now(),
    );
    let id = committed.id.clone();
    let updated =
        commit_before_registry_write(worker, &mut writer, &committed, move |reg| async move {
            reg.update(
                session_id,
                &id,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Failed),
                    summary: Some("late orphan reaper".into()),
                    increment_attempt: true,
                    ..Default::default()
                },
            )
            .await
        })
        .unwrap()
        .unwrap();
    assert_same_task(&updated, &committed);
    assert_same_task(
        &registry
            .get(session_id, &committed.id)
            .await
            .unwrap()
            .unwrap(),
        &committed,
    );
}

#[tokio::test]
async fn concurrent_update_preserves_cancel_request() {
    let (_dir, worker, mut writer, registry) = independent_registries();
    let session_id = SessionId::new();
    let mut committed = registry
        .create(create_input(session_id, TASK_KIND_BACKGROUND_TOOL))
        .await
        .unwrap();
    committed.cancel_requested_at = Some(chrono::Utc::now());
    committed.updated_at = committed.cancel_requested_at.unwrap();
    let id = committed.id.clone();
    let updated =
        commit_before_registry_write(worker, &mut writer, &committed, move |reg| async move {
            reg.update(
                session_id,
                &id,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Running),
                    summary: Some("executor heartbeat".into()),
                    ..Default::default()
                },
            )
            .await
        })
        .unwrap()
        .unwrap();
    assert_eq!(updated.state, SessionTaskState::Running);
    assert_eq!(updated.summary.as_deref(), Some("executor heartbeat"));
    assert_eq!(updated.cancel_requested_at, committed.cancel_requested_at);
    assert_same_task(
        &registry
            .get(session_id, &committed.id)
            .await
            .unwrap()
            .unwrap(),
        &updated,
    );
}

#[tokio::test]
async fn concurrent_cancel_preserves_completion_and_first_request() {
    for completed in [false, true] {
        let (_dir, worker, mut writer, registry) = independent_registries();
        let session_id = SessionId::new();
        let mut input = create_input(session_id, TASK_KIND_BACKGROUND_TOOL);
        input.state = SessionTaskState::Running;
        let mut committed = registry.create(input).await.unwrap();
        if completed {
            apply_task_update(
                &mut committed,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Succeeded),
                    summary: Some("finished before cancellation".into()),
                    ..Default::default()
                },
                chrono::Utc::now(),
            );
        } else {
            committed.cancel_requested_at = Some(chrono::Utc::now());
            committed.updated_at = committed.cancel_requested_at.unwrap();
        }
        let id = committed.id.clone();
        let cancelled =
            commit_before_registry_write(worker, &mut writer, &committed, move |reg| async move {
                reg.request_cancel(session_id, &id).await
            })
            .unwrap()
            .unwrap();
        assert_same_task(&cancelled, &committed);
        assert_same_task(
            &registry
                .get(session_id, &committed.id)
                .await
                .unwrap()
                .unwrap(),
            &committed,
        );
    }
}

#[tokio::test]
async fn missing_and_cross_session_writes_preserve_snapshot() {
    let reg = LocalSessionTaskRegistry::new(SqliteDb::open_in_memory().unwrap()).unwrap();
    let session_id = SessionId::new();
    let task = reg
        .create(create_input(session_id, TASK_KIND_BACKGROUND_TOOL))
        .await
        .unwrap();
    for (session, id) in [
        (session_id, "missing"),
        (SessionId::new(), task.id.as_str()),
    ] {
        assert!(
            reg.update(
                session,
                id,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Failed),
                    increment_attempt: true,
                    ..Default::default()
                }
            )
            .await
            .unwrap()
            .is_none()
        );
        assert!(reg.request_cancel(session, id).await.unwrap().is_none());
    }
    assert_same_task(
        &reg.get(session_id, &task.id).await.unwrap().unwrap(),
        &task,
    );
}

#[tokio::test]
async fn concurrent_reply_preserves_cancellation_and_completion() {
    for completed in [false, true] {
        let (_dir, worker, mut writer, registry) = independent_registries();
        let session_id = SessionId::new();
        let task = registry
            .create(create_input(session_id, TASK_KIND_SUBAGENT))
            .await
            .unwrap();
        let mut committed = registry
            .update(
                session_id,
                &task.id,
                SessionTaskUpdate {
                    input_request: Some(TaskInputRequest {
                        id: "concurrent_req".into(),
                        prompt: "Approve?".into(),
                        expected: None,
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .unwrap();
        if completed {
            apply_task_update(
                &mut committed,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Succeeded),
                    summary: Some("already finished".into()),
                    ..Default::default()
                },
                chrono::Utc::now(),
            );
        } else {
            committed.cancel_requested_at = Some(chrono::Utc::now());
            committed.updated_at = committed.cancel_requested_at.unwrap();
            committed.summary = Some("latest snapshot".into());
        }
        let id = committed.id.clone();
        commit_before_registry_write(worker, &mut writer, &committed, move |reg| async move {
            reg.record_message(
                session_id,
                &id,
                NewTaskMessage {
                    direction: everruns_core::session_task::TaskMessageDirection::Inbound,
                    content: vec![everruns_core::session_task::TaskMessagePart::text("yes")],
                    in_reply_to: Some("concurrent_req".into()),
                    expected_attempt: None,
                },
            )
            .await
        })
        .unwrap();
        let stored = registry
            .get(session_id, &committed.id)
            .await
            .unwrap()
            .unwrap();
        if completed {
            assert_same_task(&stored, &committed);
        } else {
            assert_eq!(stored.state, SessionTaskState::Running);
            assert!(stored.input_request.is_none());
            assert_eq!(stored.cancel_requested_at, committed.cancel_requested_at);
            assert_eq!(stored.summary, committed.summary);
        }
        assert_eq!(
            registry
                .list_messages(session_id, &committed.id, None, None)
                .await
                .unwrap()
                .len(),
            1
        );
    }
}

#[tokio::test]
async fn concurrent_message_rejects_stale_attempt_without_inserting() {
    let (_dir, worker, mut writer, registry) = independent_registries();
    let session_id = SessionId::new();
    let mut committed = registry
        .create(create_input(session_id, TASK_KIND_SUBAGENT))
        .await
        .unwrap();
    let expected_attempt = committed.attempt;
    apply_task_update(
        &mut committed,
        SessionTaskUpdate {
            increment_attempt: true,
            ..Default::default()
        },
        chrono::Utc::now(),
    );
    let id = committed.id.clone();
    let result =
        commit_before_registry_write(worker, &mut writer, &committed, move |reg| async move {
            reg.record_message(
                session_id,
                &id,
                NewTaskMessage::outbound_text("stale message")
                    .with_expected_attempt(expected_attempt),
            )
            .await
        });
    assert!(result.is_err());
    assert!(
        registry
            .list_messages(session_id, &committed.id, None, None)
            .await
            .unwrap()
            .is_empty()
    );
    assert_same_task(
        &registry
            .get(session_id, &committed.id)
            .await
            .unwrap()
            .unwrap(),
        &committed,
    );
}

#[tokio::test]
async fn message_and_resume_roll_back_together() {
    let db = SqliteDb::open_in_memory().unwrap();
    let reg = LocalSessionTaskRegistry::new(db.clone()).unwrap();
    let session_id = SessionId::new();
    let task = reg
        .create(create_input(session_id, TASK_KIND_SUBAGENT))
        .await
        .unwrap();
    let awaiting = reg
        .update(
            session_id,
            &task.id,
            SessionTaskUpdate {
                input_request: Some(TaskInputRequest {
                    id: "rollback_req".into(),
                    prompt: "Approve?".into(),
                    expected: None,
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    db.with_conn(|conn| {
        conn.execute_batch(
            "CREATE TRIGGER reject_snapshot BEFORE UPDATE ON local_tasks
         BEGIN SELECT RAISE(FAIL, 'reject snapshot update'); END;",
        )
    })
    .unwrap();
    let result = reg
        .record_message(
            session_id,
            &task.id,
            NewTaskMessage {
                direction: everruns_core::session_task::TaskMessageDirection::Inbound,
                content: vec![everruns_core::session_task::TaskMessagePart::text("yes")],
                in_reply_to: Some("rollback_req".into()),
                expected_attempt: None,
            },
        )
        .await;
    assert!(result.is_err());
    assert!(
        reg.list_messages(session_id, &task.id, None, None)
            .await
            .unwrap()
            .is_empty()
    );
    assert_same_task(
        &reg.get(session_id, &task.id).await.unwrap().unwrap(),
        &awaiting,
    );
}

#[tokio::test]
async fn causal_task_origin_survives_database_reopen_and_reaches_wake_queue() {
    use everruns_contracts::typed_id::{MessageId, TurnId};
    use everruns_core::events::EventContext;
    use everruns_core::session_task::TurnCorrelatedSessionTaskRegistry;
    use std::sync::Arc;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("causal-tasks.db");
    let session = SessionId::new();
    let context = EventContext::turn(TurnId::new(), MessageId::new());
    let expected = everruns::CausalContext::from_traceparents([
        "00-1234567890abcdef1234567890abcdef-1234567890abcdef-00",
    ]);
    let id = {
        let registry = LocalSessionTaskRegistry::new(SqliteDb::open(&path).unwrap()).unwrap();
        let bound = TurnCorrelatedSessionTaskRegistry::wrap_with_causal_context(
            Arc::new(registry),
            context.clone(),
            expected.clone(),
        );
        let mut input = create_input(session, TASK_KIND_SUBAGENT);
        input.wake_policy = TaskWakePolicy::OnTerminal;
        bound.create(input).await.unwrap().id
    };
    let registry = LocalSessionTaskRegistry::new(SqliteDb::open(&path).unwrap()).unwrap();
    let task = registry.get(session, &id).await.unwrap().unwrap();
    assert_eq!(everruns_core::causal::task_causal_context(&task), expected);
    assert!(
        !serde_json::to_string(&task)
            .unwrap()
            .contains("__everruns_origin_event_context")
    );
    let queue = everruns_core::SessionWakeQueue::new();
    assert!(queue.note_transition(&task, everruns_core::TaskTransition::Terminal));
    assert_eq!(queue.drain(session)[0].causal_context, expected);
}
