use super::ownership::{
    BACKGROUND_OWNER_PREFIX, BackgroundTaskRegistration, active_background_tasks, background_owner,
    watch_background_task,
};
use super::test_support::{InMemoryTaskRegistry, make_reattach_task};
use super::*;
use everruns_core::session_task::{SessionTaskRegistry, SessionTaskState, TaskExecutor};

fn owned_task() -> SessionTask {
    let mut task = make_reattach_task(
        json!({"tool": "test_background", "arguments": {}, "reattachable": true}),
    );
    task.worker_id = Some(background_owner().to_owned());
    task
}

#[test]
fn active_registration_and_drop_define_ownership() {
    let task = owned_task();
    assert_eq!(background_task_ownership(&task), TaskOwnership::Lost);
    let registration = BackgroundTaskRegistration::register(&task).unwrap();
    assert_eq!(background_task_ownership(&task), TaskOwnership::Alive);
    assert!(BackgroundTaskRegistration::register(&task).is_err());
    assert_eq!(background_task_ownership(&task), TaskOwnership::Alive);
    drop(registration);
    assert_eq!(background_task_ownership(&task), TaskOwnership::Lost);
}

#[test]
fn registration_is_scoped_to_session_task_and_attempt() {
    let task = owned_task();
    let registration = BackgroundTaskRegistration::register(&task).unwrap();
    let mut other = task.clone();
    other.session_id = SessionId::new();
    assert_eq!(background_task_ownership(&other), TaskOwnership::Lost);
    other = task.clone();
    other.id.push_str("-different");
    assert_eq!(background_task_ownership(&other), TaskOwnership::Lost);
    other = task.clone();
    other.attempt += 1;
    assert_eq!(background_task_ownership(&other), TaskOwnership::Lost);
    let next_attempt = BackgroundTaskRegistration::register(&other).unwrap();
    assert_eq!(background_task_ownership(&other), TaskOwnership::Alive);
    drop(registration);
    assert_eq!(background_task_ownership(&task), TaskOwnership::Lost);
    assert_eq!(background_task_ownership(&other), TaskOwnership::Alive);
    drop(next_attempt);
}

#[test]
fn foreign_instance_with_same_pid_is_unknown_even_with_registration() {
    let mut task = owned_task();
    let _registration = BackgroundTaskRegistration::register(&task).unwrap();
    task.worker_id = Some(format!(
        "{BACKGROUND_OWNER_PREFIX}{}:{}",
        std::process::id(),
        uuid::Uuid::now_v7().simple()
    ));
    assert_eq!(background_task_ownership(&task), TaskOwnership::Unknown);
    assert_eq!(
        background_task_owner_process(&task),
        Some(std::process::id())
    );
    task.worker_id = None;
    assert_eq!(background_task_ownership(&task), TaskOwnership::Unknown);
    assert_eq!(background_task_owner_process(&task), None);
    task.worker_id = Some("remote-worker".into());
    assert_eq!(background_task_ownership(&task), TaskOwnership::Unknown);
    assert_eq!(background_task_owner_process(&task), None);
}

#[test]
fn pid_is_returned_only_for_a_canonical_background_marker() {
    let mut task = owned_task();
    assert_eq!(
        background_task_owner_process(&task),
        Some(std::process::id())
    );
    let token = uuid::Uuid::now_v7().simple().to_string();
    for marker in [
        format!("{BACKGROUND_OWNER_PREFIX}0:{token}"),
        format!("{BACKGROUND_OWNER_PREFIX}01:{token}"),
        format!("{BACKGROUND_OWNER_PREFIX}4294967295:{token}"),
        format!("{BACKGROUND_OWNER_PREFIX}-1:{token}"),
        format!("{BACKGROUND_OWNER_PREFIX}1:{token}:extra"),
        format!("{BACKGROUND_OWNER_PREFIX}1:{}", uuid::Uuid::nil().simple()),
        format!("{BACKGROUND_OWNER_PREFIX}1:{}", uuid::Uuid::now_v7()),
        format!("{BACKGROUND_OWNER_PREFIX}1:not-a-token"),
        format!("background-1-{token}"),
    ] {
        task.worker_id = Some(marker);
        assert_eq!(background_task_owner_process(&task), None);
        assert_eq!(background_task_ownership(&task), TaskOwnership::Unknown);
    }
    task.worker_id = Some(background_owner().to_owned());
    task.kind = "subagent".into();
    assert_eq!(background_task_owner_process(&task), None);
    assert_eq!(background_task_ownership(&task), TaskOwnership::Unknown);
}

#[test]
fn executor_reports_the_scoped_registration() {
    let executor = crate::capabilities::background_execution::BackgroundToolTaskExecutor;
    let task = owned_task();
    assert_eq!(executor.ownership(&task), TaskOwnership::Lost);
    assert_eq!(
        executor.local_owner_process(&task),
        Some(std::process::id())
    );
    let _registration = BackgroundTaskRegistration::register(&task).unwrap();
    assert_eq!(executor.ownership(&task), TaskOwnership::Alive);
}

#[tokio::test]
async fn claim_persists_owner_with_a_live_registration() {
    let registry = InMemoryTaskRegistry::default();
    let mut task = owned_task();
    task.worker_id = None;
    registry
        .tasks
        .lock()
        .unwrap()
        .insert(task.id.clone(), task.clone());
    let registration = claim_background_task(&registry, &task).await.unwrap();
    let claimed = registry
        .get(task.session_id, &task.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.worker_id.as_deref(), Some(background_owner()));
    assert!(claimed.heartbeat_at.is_some());
    assert_eq!(background_task_ownership(&claimed), TaskOwnership::Alive);
    assert!(claim_background_task(&registry, &claimed).await.is_err());
    drop(registration);
    assert_eq!(background_task_ownership(&claimed), TaskOwnership::Lost);
}

#[tokio::test]
async fn stale_or_missing_claim_releases_registration() {
    let registry = InMemoryTaskRegistry::default();
    let task = owned_task();
    let mut newer = task.clone();
    newer.attempt += 1;
    newer.worker_id = Some("foreign-worker".into());
    registry
        .tasks
        .lock()
        .unwrap()
        .insert(task.id.clone(), newer.clone());
    assert!(claim_background_task(&registry, &task).await.is_err());
    assert_eq!(background_task_ownership(&task), TaskOwnership::Lost);
    let unchanged = registry
        .get(task.session_id, &task.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.attempt, newer.attempt);
    assert_eq!(unchanged.worker_id, newer.worker_id);
    registry.tasks.lock().unwrap().remove(&task.id);
    assert!(claim_background_task(&registry, &task).await.is_err());
    assert_eq!(background_task_ownership(&task), TaskOwnership::Lost);
}

#[tokio::test]
async fn cancellation_between_create_and_claim_settles_unstarted_task() {
    let registry = InMemoryTaskRegistry::default();
    let mut task = owned_task();
    task.worker_id = None;
    registry
        .tasks
        .lock()
        .unwrap()
        .insert(task.id.clone(), task.clone());
    registry
        .request_cancel(task.session_id, &task.id)
        .await
        .unwrap();
    assert!(claim_background_task(&registry, &task).await.is_err());
    let current = registry
        .get(task.session_id, &task.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.state, SessionTaskState::Canceled);
    assert_eq!(current.attempt, task.attempt + 1);
    assert!(current.cancel_requested_at.is_some());
    assert!(!active_background_tasks().lock().unwrap().contains(&(
        task.session_id,
        task.id.clone(),
        task.attempt
    )));
}

#[tokio::test]
async fn cancellation_is_checked_before_the_first_heartbeat_delay() {
    let registry = InMemoryTaskRegistry::default();
    let mut task = owned_task();
    task.cancel_requested_at = Some(chrono::Utc::now());
    let _registration = BackgroundTaskRegistration::register(&task).unwrap();
    registry
        .tasks
        .lock()
        .unwrap()
        .insert(task.id.clone(), task.clone());
    tokio::time::timeout(
        std::time::Duration::from_millis(100),
        watch_background_task(&registry, task.session_id, &task.id, task.attempt),
    )
    .await
    .expect("cancellation must not wait for the heartbeat interval");
}

#[tokio::test]
async fn cancellation_before_execution_never_polls_the_tool() {
    let registry = InMemoryTaskRegistry::default();
    let mut task = owned_task();
    task.cancel_requested_at = Some(chrono::Utc::now());
    let _registration = BackgroundTaskRegistration::register(&task).unwrap();
    registry
        .tasks
        .lock()
        .unwrap()
        .insert(task.id.clone(), task.clone());
    let result =
        run_owned_background_task(&registry, task.session_id, &task.id, task.attempt, async {
            panic!("canceled tool was polled")
        })
        .await;
    assert!(is_canceled_outcome(&result));
}

#[tokio::test]
async fn terminal_claim_releases_registration() {
    let registry = InMemoryTaskRegistry::default();
    let mut task = owned_task();
    task.state = SessionTaskState::Succeeded;
    registry
        .tasks
        .lock()
        .unwrap()
        .insert(task.id.clone(), task.clone());
    assert!(claim_background_task(&registry, &task).await.is_err());
    assert_eq!(background_task_ownership(&task), TaskOwnership::Lost);
}

#[tokio::test]
async fn completion_and_cooperative_cancellation_drop_registration() {
    let task = owned_task();
    let registration = BackgroundTaskRegistration::register(&task).unwrap();
    let (cancel, canceled) = tokio::sync::oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        let _registration = registration;
        canceled.await.unwrap();
    });
    assert_eq!(background_task_ownership(&task), TaskOwnership::Alive);
    cancel.send(()).unwrap();
    handle.await.unwrap();
    assert_eq!(background_task_ownership(&task), TaskOwnership::Lost);

    let registration = BackgroundTaskRegistration::register(&task).unwrap();
    tokio::spawn(async move {
        let _registration = registration;
    })
    .await
    .unwrap();
    assert_eq!(background_task_ownership(&task), TaskOwnership::Lost);
}

#[tokio::test]
async fn abort_before_first_poll_and_panic_drop_registration() {
    let task = owned_task();
    let registration = BackgroundTaskRegistration::register(&task).unwrap();
    let handle = tokio::spawn(async move {
        let _registration = registration;
        std::future::pending::<()>().await;
    });
    assert_eq!(background_task_ownership(&task), TaskOwnership::Alive);
    handle.abort();
    assert!(handle.await.unwrap_err().is_cancelled());
    assert_eq!(background_task_ownership(&task), TaskOwnership::Lost);

    let registration = BackgroundTaskRegistration::register(&task).unwrap();
    let handle = tokio::spawn(async move {
        let _registration = registration;
        panic!("ownership cleanup test");
    });
    assert!(handle.await.unwrap_err().is_panic());
    assert_eq!(background_task_ownership(&task), TaskOwnership::Lost);
}

struct HeldBackgroundTool {
    started: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}

#[async_trait]
impl Tool for HeldBackgroundTool {
    fn name(&self) -> &str {
        "held_background"
    }
    fn description(&self) -> &str {
        "Lifecycle test fixture"
    }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object"})
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::success(json!({"ok": true}))
    }

    fn hints(&self) -> everruns_contracts::tool_types::ToolHints {
        everruns_contracts::tool_types::ToolHints::default()
            .with_supports_background(true)
            .with_readonly(true)
    }

    fn as_background_executable(
        &self,
    ) -> Option<&dyn everruns_core::background::BackgroundExecutableTool> {
        Some(self)
    }
}

#[async_trait]
impl everruns_core::background::BackgroundExecutableTool for HeldBackgroundTool {
    async fn execute_background(
        &self,
        _arguments: Value,
        _context: ToolContext,
        _sink: Arc<dyn BackgroundEventSink>,
    ) -> std::result::Result<BackgroundOutcome, ToolExecutionResult> {
        self.started.notify_one();
        self.release.notified().await;
        Ok(BackgroundOutcome {
            summary: "finished".into(),
            result: json!({"ok": true}),
            raw_output: None,
        })
    }
}

fn held_context() -> (
    ToolContext,
    Arc<InMemoryTaskRegistry>,
    Arc<tokio::sync::Notify>,
    Arc<tokio::sync::Notify>,
) {
    let tasks = Arc::new(InMemoryTaskRegistry::default());
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let mut tools = ToolRegistry::new();
    tools.register(HeldBackgroundTool {
        started: started.clone(),
        release: release.clone(),
    });
    let context = ToolContext::with_stores(
        SessionId::new(),
        Arc::new(super::test_support::TestFileStore::default()),
        Arc::new(super::test_support::NoopStorageStore),
    )
    .with_tool_registry(Arc::new(tools))
    .with_nested_tool_policy(super::test_support::allow_all_policy())
    .with_session_task_registry(tasks.clone());
    (context, tasks, started, release)
}

async fn await_finished(tasks: &InMemoryTaskRegistry, task: &SessionTask) {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let finished = tasks.get(task.session_id, &task.id).await.unwrap().unwrap();
            if finished.state.is_terminal()
                && background_task_ownership(&finished) == TaskOwnership::Lost
            {
                assert_eq!(finished.state, SessionTaskState::Succeeded);
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn normal_spawn_holds_registration_until_work_finishes() {
    let (context, tasks, started, release) = held_context();
    let result = SpawnBackgroundTool
        .execute_with_context(
            json!({
                "tool": "held_background", "args": {}, "signal_on_completion": false
            }),
            &context,
        )
        .await;
    let task_id = match result {
        ToolExecutionResult::Success(value) => value["task_id"].as_str().unwrap().to_owned(),
        other => panic!("unexpected spawn result: {other:?}"),
    };
    let task = tasks
        .get(context.session_id, &task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.worker_id.as_deref(), Some(background_owner()));
    assert_eq!(background_task_ownership(&task), TaskOwnership::Alive);
    tokio::time::timeout(std::time::Duration::from_secs(3), started.notified())
        .await
        .unwrap();
    release.notify_one();
    await_finished(&tasks, &task).await;
}

#[tokio::test]
async fn reattach_claims_new_attempt_and_holds_registration_until_finished() {
    let (context, tasks, started, release) = held_context();
    let mut task = owned_task();
    task.session_id = context.session_id;
    task.attempt = 2;
    task.worker_id = Some("old-worker".into());
    task.spec = json!({"tool": "held_background", "arguments": {}, "signal_on_completion": false});
    tasks
        .tasks
        .lock()
        .unwrap()
        .insert(task.id.clone(), task.clone());
    reattach_background_run_with_registry(&task, &context, context.tool_registry.clone().unwrap())
        .await
        .unwrap();
    let claimed = tasks.get(task.session_id, &task.id).await.unwrap().unwrap();
    assert_eq!(claimed.attempt, 2);
    assert_eq!(claimed.worker_id.as_deref(), Some(background_owner()));
    assert_eq!(background_task_ownership(&claimed), TaskOwnership::Alive);
    tokio::time::timeout(std::time::Duration::from_secs(3), started.notified())
        .await
        .unwrap();
    release.notify_one();
    await_finished(&tasks, &claimed).await;
}

#[tokio::test]
async fn heartbeat_stops_without_overwriting_a_foreign_owner() {
    let tasks = InMemoryTaskRegistry::default();
    let task = owned_task();
    let _registration = BackgroundTaskRegistration::register(&task).unwrap();
    let mut foreign = task.clone();
    foreign.worker_id = Some("remote-worker".into());
    tasks
        .tasks
        .lock()
        .unwrap()
        .insert(task.id.clone(), foreign.clone());
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        watch_background_task(&tasks, task.session_id, &task.id, task.attempt),
    )
    .await
    .unwrap();
    let unchanged = tasks.get(task.session_id, &task.id).await.unwrap().unwrap();
    assert_eq!(unchanged.worker_id, foreign.worker_id);
    assert_eq!(unchanged.heartbeat_at, foreign.heartbeat_at);
}

#[tokio::test]
async fn stale_sink_cannot_overwrite_a_new_attempt() {
    let (context, tasks, _, _) = held_context();
    let mut task = owned_task();
    task.session_id = context.session_id;
    let old_attempt = task.attempt;
    task.attempt += 1;
    task.worker_id = Some("new-worker".into());
    tasks
        .tasks
        .lock()
        .unwrap()
        .insert(task.id.clone(), task.clone());
    let sink = SessionBackgroundSink::new(
        context,
        "stale-run".into(),
        "test".into(),
        "held_background".into(),
        "test.log".into(),
        "test.result".into(),
        false,
        Some(task.id.clone()),
        old_attempt,
    );
    sink.status("stale status").await.unwrap();
    let unchanged = tasks.get(task.session_id, &task.id).await.unwrap().unwrap();
    assert_eq!(unchanged.attempt, task.attempt);
    assert_eq!(unchanged.worker_id, task.worker_id);
    assert_eq!(unchanged.progress, task.progress);
    assert_eq!(unchanged.state, task.state);
}
