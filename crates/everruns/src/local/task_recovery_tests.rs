use super::*;
use everruns_contracts::runtime::tool_context::ToolContext;
use serde_json::json;

use crate::local::LocalSessionTaskRegistry;

const KIND: &str = "local_recovery_test";

struct Executor {
    ownership: TaskOwnership,
    pid: Option<u32>,
}

#[async_trait]
impl TaskExecutor for Executor {
    fn kind(&self) -> &'static str {
        KIND
    }

    fn ownership(&self, _task: &SessionTask) -> TaskOwnership {
        self.ownership
    }

    fn local_owner_process(&self, _task: &SessionTask) -> Option<u32> {
        self.pid
    }

    async fn cancel(&self, _task: &SessionTask, _context: &ToolContext) -> Result<()> {
        Ok(())
    }
}

fn input(session_id: SessionId, kind: &str, state: SessionTaskState) -> CreateSessionTask {
    CreateSessionTask {
        session_id,
        kind: kind.into(),
        id: None,
        display_name: "recover fixture".into(),
        spec: json!({"keep": true}),
        state,
        links: Default::default(),
        wake_policy: Default::default(),
    }
}

fn recovery(
    db: SqliteDb,
    ownership: TaskOwnership,
    pid: Option<u32>,
) -> (Arc<LocalSessionTaskRegistry>, Arc<LocalTaskRecovery>) {
    let registry = Arc::new(LocalSessionTaskRegistry::new(db.clone()).unwrap());
    let executor: Arc<dyn TaskExecutor> = Arc::new(Executor { ownership, pid });
    let mut recovery = LocalTaskRecovery::new(db, registry.clone());
    recovery.executor_lookup = Arc::new(move |kind| (kind == KIND).then(|| executor.clone()));
    (registry, Arc::new(recovery))
}

#[tokio::test]
async fn file_backed_restart_recovers_lost_execution_once() {
    let directory = tempfile::TempDir::new().unwrap();
    let path = directory.path().join("profile.sqlite");
    let session_id = SessionId::new();
    let task_id = {
        let registry = LocalSessionTaskRegistry::new(SqliteDb::open(&path).unwrap()).unwrap();
        let task = registry
            .create(input(session_id, KIND, SessionTaskState::Running))
            .await
            .unwrap();
        registry
            .update(
                session_id,
                &task.id,
                SessionTaskUpdate {
                    worker_id: Some("old-owner-instance".into()),
                    increment_attempt: true,
                    result_path: Some("result.txt".into()),
                    artifacts: Some(vec![
                        serde_json::from_value(json!({
                            "name": "Partial output",
                            "type": "file",
                            "path": "/.tasks/partial-output.txt"
                        }))
                        .unwrap(),
                    ]),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        task.id
    };
    let (registry, recovery) = recovery(SqliteDb::open(&path).unwrap(), TaskOwnership::Lost, None);
    let recovered = recovery.recover_all().await.unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].id, task_id);
    assert_eq!(recovered[0].state, SessionTaskState::Failed);
    assert!(recovered[0].finished_at.is_some());
    assert_eq!(
        recovered[0].worker_id.as_deref(),
        Some("old-owner-instance")
    );
    assert_eq!(recovered[0].attempt, 3);
    assert_eq!(recovered[0].spec, json!({"keep": true}));
    assert_eq!(recovered[0].result_path.as_deref(), Some("result.txt"));
    assert_eq!(recovered[0].artifacts.len(), 1);
    assert_eq!(
        recovered[0].artifacts[0].path.as_deref(),
        Some("/.tasks/partial-output.txt")
    );
    assert!(recovery.recover_all().await.unwrap().is_empty());
    let stored = registry.get(session_id, &task_id).await.unwrap().unwrap();
    assert_eq!(stored.finished_at, recovered[0].finished_at);
    assert_eq!(stored.updated_at, recovered[0].updated_at);
}

#[tokio::test]
async fn alive_unknown_and_unregistered_owners_are_not_recovered() {
    for ownership in [TaskOwnership::Alive, TaskOwnership::Unknown] {
        let (registry, recovery) = recovery(SqliteDb::open_in_memory().unwrap(), ownership, None);
        let session_id = SessionId::new();
        let task = registry
            .create(input(session_id, KIND, SessionTaskState::Running))
            .await
            .unwrap();
        let missing = registry
            .create(input(session_id, "unregistered", SessionTaskState::Running))
            .await
            .unwrap();
        assert!(
            recovery
                .recover_session(session_id)
                .await
                .unwrap()
                .is_empty()
        );
        for original in [task, missing] {
            let stored = recovery
                .get(session_id, &original.id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(stored.state, SessionTaskState::Running);
            assert_eq!(stored.updated_at, original.updated_at);
        }
    }
}

#[tokio::test]
async fn awaiting_input_queued_and_terminal_tasks_are_preserved() {
    let (registry, recovery) = recovery(
        SqliteDb::open_in_memory().unwrap(),
        TaskOwnership::Lost,
        None,
    );
    let session_id = SessionId::new();
    for state in [
        SessionTaskState::Queued,
        SessionTaskState::AwaitingInput,
        SessionTaskState::Succeeded,
        SessionTaskState::Failed,
        SessionTaskState::Canceled,
    ] {
        let task = registry
            .create(input(session_id, KIND, state))
            .await
            .unwrap();
        assert!(recovery.recover_task(&task).await.unwrap().is_none());
        let stored = recovery.get(session_id, &task.id).await.unwrap().unwrap();
        assert_eq!(stored.state, state);
        assert_eq!(stored.updated_at, task.updated_at);
    }
}

#[tokio::test]
async fn cancel_lost_task_settles_once_and_preserves_intent() {
    let (registry, recovery) = recovery(
        SqliteDb::open_in_memory().unwrap(),
        TaskOwnership::Lost,
        None,
    );
    let session_id = SessionId::new();
    let task = registry
        .create(input(session_id, KIND, SessionTaskState::Running))
        .await
        .unwrap();
    let canceled = recovery
        .request_cancel(session_id, &task.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(canceled.state, SessionTaskState::Canceled);
    assert!(canceled.cancel_requested_at.is_some());
    assert!(canceled.finished_at.is_some());
    let repeated = recovery
        .request_cancel(session_id, &task.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(repeated.updated_at, canceled.updated_at);
    assert_eq!(repeated.cancel_requested_at, canceled.cancel_requested_at);
    assert_eq!(repeated.finished_at, canceled.finished_at);
}

#[tokio::test]
async fn stale_heartbeat_or_attempt_cannot_be_settled() {
    let (registry, recovery) = recovery(
        SqliteDb::open_in_memory().unwrap(),
        TaskOwnership::Lost,
        None,
    );
    let session_id = SessionId::new();
    for update in [
        SessionTaskUpdate {
            heartbeat_at: Some(chrono::Utc::now()),
            ..Default::default()
        },
        SessionTaskUpdate {
            increment_attempt: true,
            worker_id: Some("new-owner".into()),
            ..Default::default()
        },
    ] {
        let observed = registry
            .create(input(session_id, KIND, SessionTaskState::Running))
            .await
            .unwrap();
        let current = registry
            .update(session_id, &observed.id, update)
            .await
            .unwrap()
            .unwrap();
        assert!(recovery.recover_task(&observed).await.unwrap().is_none());
        let stored = registry
            .get(session_id, &observed.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.state, SessionTaskState::Running);
        assert_eq!(stored.updated_at, current.updated_at);
        assert_eq!(stored.attempt, current.attempt);
        assert_eq!(stored.worker_id, current.worker_id);
        assert_eq!(stored.heartbeat_at, current.heartbeat_at);
    }
}

#[tokio::test]
async fn list_recovers_before_state_filter_and_get_handles_missing_tasks() {
    let (registry, recovery) = recovery(
        SqliteDb::open_in_memory().unwrap(),
        TaskOwnership::Lost,
        None,
    );
    let session_id = SessionId::new();
    let task = registry
        .create(input(session_id, KIND, SessionTaskState::Running))
        .await
        .unwrap();
    let failed = recovery
        .list(
            session_id,
            Some(&SessionTaskFilter {
                state: Some(SessionTaskState::Failed),
                ..Default::default()
            }),
        )
        .await
        .unwrap();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].id, task.id);
    assert!(
        recovery
            .get(SessionId::new(), &task.id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(recovery.get(session_id, "missing").await.unwrap().is_none());
}

#[cfg(unix)]
#[tokio::test]
async fn validated_dead_pid_is_lost_but_live_pid_is_not_positive_evidence() {
    let mut process = std::process::Command::new("sh")
        .args(["-c", "exit 0"])
        .spawn()
        .unwrap();
    let pid = process.id();
    process.wait().unwrap();
    assert_eq!(process_ownership(pid), TaskOwnership::OwnerLost);
    assert_eq!(
        process_ownership(std::process::id()),
        TaskOwnership::Unknown
    );
    assert_eq!(process_ownership(0), TaskOwnership::Unknown);
    assert_eq!(process_ownership(u32::MAX), TaskOwnership::Unknown);
    let (registry, recovery) = recovery(
        SqliteDb::open_in_memory().unwrap(),
        TaskOwnership::Unknown,
        Some(pid),
    );
    let session_id = SessionId::new();
    let task = registry
        .create(input(session_id, KIND, SessionTaskState::Running))
        .await
        .unwrap();
    let recovered = recovery.get(session_id, &task.id).await.unwrap().unwrap();
    assert_eq!(recovered.state, SessionTaskState::Failed);
}

#[tokio::test(start_paused = true)]
async fn maintenance_runs_at_low_frequency_and_does_not_retain_service() {
    let (registry, recovery) = recovery(
        SqliteDb::open_in_memory().unwrap(),
        TaskOwnership::Lost,
        None,
    );
    let session_id = SessionId::new();
    let task = registry
        .create(input(session_id, KIND, SessionTaskState::Running))
        .await
        .unwrap();
    recovery.start_maintenance();
    recovery.start_maintenance();
    tokio::task::yield_now().await;
    assert_eq!(
        registry
            .get(session_id, &task.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        SessionTaskState::Running
    );
    tokio::time::advance(Duration::from_secs(60)).await;
    tokio::task::yield_now().await;
    assert_eq!(
        registry
            .get(session_id, &task.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        SessionTaskState::Failed
    );
    let weak = Arc::downgrade(&recovery);
    let handle = recovery.maintenance.lock().as_ref().unwrap().abort_handle();
    drop(recovery);
    tokio::task::yield_now().await;
    assert!(weak.upgrade().is_none());
    assert!(handle.is_finished());
}

#[cfg(unix)]
#[tokio::test]
async fn executor_proof_overrides_pid_probe_and_live_pid_remains_unknown() {
    let mut process = std::process::Command::new("sh")
        .args(["-c", "exit 0"])
        .spawn()
        .unwrap();
    let pid = process.id();
    process.wait().unwrap();
    for (ownership, pid, expected) in [
        (TaskOwnership::Alive, pid, SessionTaskState::Running),
        (
            TaskOwnership::Lost,
            std::process::id(),
            SessionTaskState::Failed,
        ),
        (
            TaskOwnership::Unknown,
            std::process::id(),
            SessionTaskState::Running,
        ),
    ] {
        let (registry, recovery) =
            recovery(SqliteDb::open_in_memory().unwrap(), ownership, Some(pid));
        let session_id = SessionId::new();
        let task = registry
            .create(input(session_id, KIND, SessionTaskState::Running))
            .await
            .unwrap();
        assert_eq!(
            recovery
                .get(session_id, &task.id)
                .await
                .unwrap()
                .unwrap()
                .state,
            expected
        );
    }
}

#[tokio::test]
async fn only_guarded_winner_notifies_existing_wake_queue() {
    use everruns_core::SessionWakeQueue;
    use everruns_core::session_task::TaskWakePolicy;

    for policy in [
        TaskWakePolicy::OnTerminal,
        TaskWakePolicy::OnActivity,
        TaskWakePolicy::Silent,
    ] {
        let db = SqliteDb::open_in_memory().unwrap();
        let (registry, first) = recovery(db.clone(), TaskOwnership::Lost, None);
        let (_, second) = recovery(db, TaskOwnership::Lost, None);
        let queue = Arc::new(SessionWakeQueue::new());
        first.set_wake_observer(queue.clone());
        second.set_wake_observer(queue.clone());
        let session_id = SessionId::new();
        let mut create = input(session_id, KIND, SessionTaskState::Running);
        create.wake_policy = policy;
        let observed = registry.create(create).await.unwrap();
        let (a, b) = tokio::join!(
            first.recover_task(&observed),
            second.recover_task(&observed)
        );
        assert_eq!(
            usize::from(a.unwrap().is_some()) + usize::from(b.unwrap().is_some()),
            1
        );
        assert!(first.recover_task(&observed).await.unwrap().is_none());
        first
            .deliver_recovery_wakes(Some(session_id))
            .await
            .unwrap();
        second
            .deliver_recovery_wakes(Some(session_id))
            .await
            .unwrap();
        let wakes = queue.drain(session_id);
        if policy == TaskWakePolicy::Silent {
            assert!(wakes.is_empty());
        } else {
            assert_eq!(wakes.len(), 1);
            assert_eq!(wakes[0].task_id, observed.id);
            assert_eq!(wakes[0].transition, TaskTransition::Terminal);
            assert!(wakes[0].text.contains("failed"));
        }
    }
}

#[tokio::test]
async fn cancel_recovery_notifies_once_and_stale_guard_does_not_notify() {
    use everruns_core::SessionWakeQueue;
    use everruns_core::session_task::TaskWakePolicy;

    let (registry, recovery) = recovery(
        SqliteDb::open_in_memory().unwrap(),
        TaskOwnership::Lost,
        None,
    );
    let queue = Arc::new(SessionWakeQueue::new());
    recovery.set_wake_observer(queue.clone());
    let session_id = SessionId::new();
    let mut create = input(session_id, KIND, SessionTaskState::Running);
    create.wake_policy = TaskWakePolicy::OnTerminal;
    let task = registry.create(create.clone()).await.unwrap();
    assert_eq!(
        recovery
            .request_cancel(session_id, &task.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        SessionTaskState::Canceled
    );
    recovery.request_cancel(session_id, &task.id).await.unwrap();
    let wakes = queue.drain(session_id);
    assert_eq!(wakes.len(), 1);
    assert!(wakes[0].text.contains("canceled"));

    let observed = registry.create(create).await.unwrap();
    registry
        .update(
            session_id,
            &observed.id,
            SessionTaskUpdate {
                increment_attempt: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(recovery.recover_task(&observed).await.unwrap().is_none());
    assert!(!queue.has_pending(session_id));
}

struct RecoveryWakeProbe {
    fail: bool,
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl TaskTransitionObserver for RecoveryWakeProbe {
    async fn on_transition(
        &self,
        task: &SessionTask,
        transition: TaskTransition,
    ) -> anyhow::Result<()> {
        assert_eq!(transition, TaskTransition::Terminal);
        assert!(task.state.is_terminal());
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self.fail {
            anyhow::bail!("recovery wake delivery interrupted");
        }
        Ok(())
    }
}

#[tokio::test]
async fn restart_retries_a_failed_recovery_wake_without_repeating_the_transition() {
    let directory = tempfile::TempDir::new().unwrap();
    let path = directory.path().join("profile.sqlite");
    let session_id = SessionId::new();
    let first_observer = Arc::new(RecoveryWakeProbe {
        fail: true,
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let task_id = {
        let (registry, recovery) =
            recovery(SqliteDb::open(&path).unwrap(), TaskOwnership::Lost, None);
        let task = registry
            .create(input(session_id, KIND, SessionTaskState::Running))
            .await
            .unwrap();
        recovery.set_wake_observer(first_observer.clone());
        let settled = recovery.recover_all().await.unwrap();
        assert_eq!(settled.len(), 1);
        assert_eq!(settled[0].attempt, task.attempt + 1);
        assert_eq!(
            first_observer
                .calls
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        task.id
    };
    let (registry, recovery) = recovery(SqliteDb::open(&path).unwrap(), TaskOwnership::Lost, None);
    let observer = Arc::new(RecoveryWakeProbe {
        fail: false,
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    recovery.set_wake_observer(observer.clone());
    assert!(recovery.recover_all().await.unwrap().is_empty());
    assert_eq!(observer.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(recovery.recover_all().await.unwrap().is_empty());
    assert_eq!(observer.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(
        registry
            .get(session_id, &task_id)
            .await
            .unwrap()
            .unwrap()
            .attempt,
        2
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_dead_owner_does_not_confirm_cancellation_of_surviving_execution() {
    let mut process = std::process::Command::new("sh")
        .args(["-c", "exit 0"])
        .spawn()
        .unwrap();
    let pid = process.id();
    process.wait().unwrap();
    let (registry, recovery) = recovery(
        SqliteDb::open_in_memory().unwrap(),
        TaskOwnership::Unknown,
        Some(pid),
    );
    let session_id = SessionId::new();
    let task = registry
        .create(input(session_id, KIND, SessionTaskState::Running))
        .await
        .unwrap();
    registry.request_cancel(session_id, &task.id).await.unwrap();
    let settled = recovery.get(session_id, &task.id).await.unwrap().unwrap();
    assert_eq!(settled.state, SessionTaskState::Failed);
    assert!(settled.cancel_requested_at.is_some());
    assert!(
        settled
            .error
            .unwrap()
            .message
            .contains("could not be confirmed stopped")
    );
}

#[tokio::test]
async fn inconsistent_attempt_fence_cannot_create_a_recovery_wake() {
    use everruns_contracts::runtime::session_task::SessionTaskRecoveryGuard;
    let (registry, recovery) = recovery(
        SqliteDb::open_in_memory().unwrap(),
        TaskOwnership::Lost,
        None,
    );
    let task = registry
        .create(input(SessionId::new(), KIND, SessionTaskState::Running))
        .await
        .unwrap();
    let updated = registry
        .update(
            task.session_id,
            &task.id,
            SessionTaskUpdate {
                expected_recovery: Some(SessionTaskRecoveryGuard::from_task(&task)),
                expected_attempt: Some(task.attempt + 1),
                increment_attempt: true,
                state: Some(SessionTaskState::Failed),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(
        updated.is_none(),
        "conflicting guards cannot claim a transition"
    );
    assert!(
        LocalSessionTaskRegistry::pending_recovery_wakes(&recovery.db, None)
            .unwrap()
            .is_empty()
    );
    let unchanged = registry
        .get(task.session_id, &task.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.state, task.state);
    assert_eq!(unchanged.attempt, task.attempt);
    assert_eq!(unchanged.updated_at, task.updated_at);
}
