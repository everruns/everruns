// Task registry invariants + restart-survivability over a file-backed DB.

use everruns_core::session_task::{
    CreateSessionTask, NewTaskMessage, SessionTaskFilter, SessionTaskRegistry, SessionTaskState,
    SessionTaskUpdate, TASK_KIND_BACKGROUND_TOOL, TASK_KIND_SUBAGENT, TaskInputRequest, TaskLinks,
    TaskWakePolicy,
};
use everruns_local::{LocalSessionTaskRegistry, SqliteDb};
use everruns_provider::typed_id::SessionId;

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
