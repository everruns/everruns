//! PostgreSQL recovery must fence activity and admit only one terminal writer.

use chrono::Utc;
use everruns_contracts::typed_id::PrincipalId;
use everruns_core::session_task::{
    CreateSessionTask, SessionTaskRecoveryGuard, SessionTaskState, SessionTaskUpdate, TaskLinks,
    TaskWakePolicy, new_session_task,
};
use everruns_server::storage::CreatePrincipalRow;
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn postgres_recovery_fences_activity_and_competing_recoverers() {
    let database = super::test_harness::isolated_test_database().await;
    let backend = database.backend();
    let principal = backend
        .create_principal(CreatePrincipalRow {
            id: PrincipalId::new(),
            org_id: 1,
            kind: "system".into(),
            subject_id: Some(Uuid::now_v7()),
            parent_principal_id: None,
            resolved_user_id: None,
            metadata: json!({"source": "session-task-recovery-test"}),
        })
        .await
        .unwrap();
    let session = backend
        .create_session(everruns_server::storage::CreateSessionRow {
            owner_principal_id: principal.id,
            ..super::session_row_fixture::base_session_row(1)
        })
        .await
        .unwrap();
    let task = new_session_task(
        CreateSessionTask {
            id: None,
            session_id: session.id,
            kind: "background_tool".into(),
            display_name: "Recovery guard".into(),
            spec: json!({}),
            state: SessionTaskState::Running,
            links: TaskLinks::default(),
            wake_policy: TaskWakePolicy::Silent,
        },
        Utc::now(),
    );
    backend.create_session_task(&task).await.unwrap();
    // Read the persisted timestamps: PostgreSQL rounds them to microseconds.
    let observed = backend
        .get_session_task(session.id, &task.id)
        .await
        .unwrap()
        .unwrap()
        .to_task()
        .unwrap();
    let stale_guard = SessionTaskRecoveryGuard::from_task(&observed);
    backend
        .update_session_task(
            session.id,
            &task.id,
            SessionTaskUpdate {
                worker_id: Some("current-owner".into()),
                heartbeat_at: Some(Utc::now()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    let active = backend
        .get_session_task(session.id, &task.id)
        .await
        .unwrap()
        .unwrap()
        .to_task()
        .unwrap();
    let recovery_update = |guard| SessionTaskUpdate {
        expected_attempt: Some(active.attempt),
        expected_recovery: Some(guard),
        increment_attempt: true,
        state: Some(SessionTaskState::Failed),
        summary: Some("Executor ownership was lost.".into()),
        ..Default::default()
    };
    assert!(
        backend
            .update_session_task(session.id, &task.id, recovery_update(stale_guard))
            .await
            .unwrap()
            .is_none(),
        "stale recovery must return no winning transition"
    );
    let mut conflicting = recovery_update(SessionTaskRecoveryGuard::from_task(&active));
    conflicting.expected_attempt = Some(active.attempt + 1);
    assert!(
        backend
            .update_session_task(session.id, &task.id, conflicting)
            .await
            .unwrap()
            .is_none()
    );
    let guard = SessionTaskRecoveryGuard::from_task(&active);
    let (first, second) = tokio::join!(
        backend.update_session_task(session.id, &task.id, recovery_update(guard.clone())),
        backend.update_session_task(session.id, &task.id, recovery_update(guard))
    );
    assert_eq!(
        [first.unwrap(), second.unwrap()]
            .iter()
            .filter(|winner| winner.is_some())
            .count(),
        1
    );
    let settled = backend
        .get_session_task(session.id, &task.id)
        .await
        .unwrap()
        .unwrap()
        .to_task()
        .unwrap();
    assert_eq!(settled.state, SessionTaskState::Failed);
    assert_eq!(settled.attempt, active.attempt + 1);
    assert_eq!(settled.worker_id, active.worker_id);
    backend.delete_session(1, session.id).await.unwrap();
}
