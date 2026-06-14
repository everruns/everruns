// Orphaned session-task reconciler.
//
// Decision: mirrors the leased-resource-cleanup activity end-to-end: same
// durable schedule seeding, same cadence, same activity registration pattern.
// The reaper finds tasks whose worker heartbeat has gone stale and either
// re-attaches them (for kinds that support it, up to `max_attempts`) or fails
// them via the registry so lifecycle invariants, events, and wake_policy all
// fire exactly as they do for any other terminal transition.
//
// Tasks with NULL heartbeat_at are never reaped (foreground subagent tasks
// are covered by EVE-535 spawn handles, not by this reconciler).
//
// Re-attach flow (per candidate):
//   1. registry.get() to obtain the current task snapshot.
//   2. find_task_executor(kind) → executor; skip if absent.
//   3. If executor.can_reattach() and task.attempt < max_attempts:
//      a. registry.update with expected_attempt=task.attempt, increment_attempt,
//         heartbeat_at=now, worker_id="reaper", state_detail="re-attached (attempt N)"
//         (state stays running so the fence is correct).
//      b. If update was accepted (attempt bumped): build a minimal ToolContext
//         and call executor.start(&updated_task, &ctx).
//      c. On start() error: log and fall back to failing as orphaned, fenced
//         on the already-bumped attempt (no further increment).
//   4. Otherwise: fail as orphaned (existing path).

use anyhow::Result;
use everruns_core::session_task::{
    SessionTaskState, SessionTaskUpdate, TaskError, find_task_executor,
};
use everruns_core::traits::ToolContext;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::worker_adapters::WorkerAdapters;

/// How long a heartbeat may be stale before a task is considered orphaned.
const DEFAULT_STALE_AFTER_SECONDS: i64 = 5 * 60; // 5 minutes

/// How many orphaned tasks to process per reaper pass.
const DEFAULT_LIMIT: i64 = 50;

/// Default maximum re-attach attempts before a task is failed as orphaned.
const DEFAULT_MAX_ATTEMPTS: i64 = 3;

/// Durable activity input for the session-task reaper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionTaskReaperInput {
    /// Age (in seconds) after which a stale heartbeat marks a task orphaned.
    pub stale_after_seconds: i64,
    /// Max tasks to reap per pass.
    pub limit: i64,
    /// Maximum re-attach attempts before falling back to orphaned-fail.
    /// Re-attachable kinds are re-started up to this many times total;
    /// on the (max_attempts)th attempt they are failed as orphaned instead.
    #[serde(default = "default_max_attempts")]
    pub max_attempts: i64,
}

fn default_max_attempts() -> i64 {
    DEFAULT_MAX_ATTEMPTS
}

impl Default for SessionTaskReaperInput {
    fn default() -> Self {
        Self {
            stale_after_seconds: DEFAULT_STALE_AFTER_SECONDS,
            limit: DEFAULT_LIMIT,
            max_attempts: DEFAULT_MAX_ATTEMPTS,
        }
    }
}

#[derive(Debug, Serialize)]
struct ReapOutcome {
    session_id: String,
    task_id: String,
    status: String,
    detail: String,
}

#[derive(Debug, Serialize)]
struct ReapSummary {
    candidates: usize,
    reaped: usize,
    reattached: usize,
    skipped: usize,
    outcomes: Vec<ReapOutcome>,
}

/// Execute one orphaned-session-task reconciliation pass.
///
/// For each stale task the reaper either:
/// - re-attaches (start on a new executor, attempt+1) if the kind supports it
///   and the attempt count is below `max_attempts`; or
/// - calls `registry.update` with `state=Failed, error={kind:"orphaned"}` so
///   lifecycle invariants hold and `task.updated` events + wake_policy fire.
pub async fn execute_reaper_activity<A: WorkerAdapters>(
    adapters: &A,
    input: &SessionTaskReaperInput,
) -> Result<serde_json::Value> {
    let stale_after = chrono::Duration::seconds(input.stale_after_seconds);

    let candidates = adapters
        .list_orphaned_session_task_ids(stale_after, input.limit)
        .await?;

    let registry = adapters.reaper_session_task_registry();

    let mut summary = ReapSummary {
        candidates: candidates.len(),
        reaped: 0,
        reattached: 0,
        skipped: 0,
        outcomes: Vec::with_capacity(candidates.len()),
    };

    for (session_id, task_id) in candidates {
        // Fetch the current task snapshot so we can inspect kind and attempt.
        let task = match registry.get(session_id, &task_id).await {
            Ok(Some(t)) => t,
            Ok(None) => {
                summary.skipped += 1;
                summary.outcomes.push(ReapOutcome {
                    session_id: session_id.to_string(),
                    task_id: task_id.clone(),
                    status: "skipped".to_string(),
                    detail: "task not found".to_string(),
                });
                continue;
            }
            Err(e) => {
                warn!(
                    session_id = %session_id,
                    task_id = %task_id,
                    error = %e,
                    "Session task reaper: failed to get task"
                );
                summary.skipped += 1;
                summary.outcomes.push(ReapOutcome {
                    session_id: session_id.to_string(),
                    task_id: task_id.clone(),
                    status: "error".to_string(),
                    detail: e.to_string(),
                });
                continue;
            }
        };

        // Skip tasks that are already terminal (heartbeat query may race).
        if task.state.is_terminal() {
            summary.skipped += 1;
            summary.outcomes.push(ReapOutcome {
                session_id: session_id.to_string(),
                task_id: task_id.clone(),
                status: "skipped".to_string(),
                detail: "already terminal".to_string(),
            });
            continue;
        }

        // Try to find a re-attachable executor for this kind.
        let should_reattach = find_task_executor(&task.kind)
            .is_some_and(|exec| exec.can_reattach())
            && (task.attempt as i64) < input.max_attempts;

        if should_reattach {
            let new_attempt = task.attempt + 1;
            let supersede_update = SessionTaskUpdate {
                // Do not set state — keep running so the fence works correctly.
                worker_id: Some("reaper".to_string()),
                heartbeat_at: Some(chrono::Utc::now()),
                state_detail: Some(format!("re-attached (attempt {new_attempt})")),
                // Atomically guard against double-reap: only apply if the
                // attempt hasn't already been bumped by another worker.
                expected_attempt: Some(task.attempt),
                increment_attempt: true,
                ..Default::default()
            };

            let updated_task = match registry
                .update(session_id, &task_id, supersede_update)
                .await
            {
                Ok(Some(t)) if t.attempt == new_attempt => t,
                Ok(_) => {
                    // Fence fired — another reaper already processed this task.
                    summary.skipped += 1;
                    summary.outcomes.push(ReapOutcome {
                        session_id: session_id.to_string(),
                        task_id: task_id.clone(),
                        status: "skipped".to_string(),
                        detail: "superseded by concurrent reaper".to_string(),
                    });
                    continue;
                }
                Err(e) => {
                    warn!(
                        session_id = %session_id,
                        task_id = %task_id,
                        error = %e,
                        "Session task reaper: failed to supersede task for re-attach"
                    );
                    summary.skipped += 1;
                    summary.outcomes.push(ReapOutcome {
                        session_id: session_id.to_string(),
                        task_id: task_id.clone(),
                        status: "error".to_string(),
                        detail: e.to_string(),
                    });
                    continue;
                }
            };

            // Build a minimal ToolContext for the executor.
            let ctx = ToolContext::new(session_id)
                .with_storage_store_arc(adapters.storage_store())
                .with_session_task_registry(registry.clone())
                .with_egress_service_opt(adapters.egress_service());

            let executor = find_task_executor(&task.kind).expect("checked above");
            match executor.start(&updated_task, &ctx).await {
                Ok(()) => {
                    info!(
                        session_id = %session_id,
                        task_id = %task_id,
                        attempt = new_attempt,
                        kind = %task.kind,
                        "Session task reaper: re-attached task"
                    );
                    summary.reattached += 1;
                    summary.outcomes.push(ReapOutcome {
                        session_id: session_id.to_string(),
                        task_id: task_id.clone(),
                        status: "reattached".to_string(),
                        detail: format!("re-attached at attempt {new_attempt}"),
                    });
                    continue;
                }
                Err(e) => {
                    warn!(
                        session_id = %session_id,
                        task_id = %task_id,
                        attempt = new_attempt,
                        kind = %task.kind,
                        error = %e,
                        "Session task reaper: start() failed after re-attach; falling back to orphaned-fail"
                    );
                    // Fall through to the orphaned-fail path below.
                    // The attempt was already incremented by the supersede update,
                    // so we don't need to increment again — but we do need to set
                    // the failed state. Use expected_attempt=new_attempt so the
                    // fence matches the current attempt (we just bumped it).
                    let fail_update = SessionTaskUpdate {
                        state: Some(SessionTaskState::Failed),
                        error: Some(TaskError {
                            kind: "orphaned".to_string(),
                            message: format!("worker heartbeat stopped; re-attach failed: {e}"),
                        }),
                        expected_attempt: Some(new_attempt),
                        ..Default::default()
                    };
                    match registry.update(session_id, &task_id, fail_update).await {
                        Ok(Some(t)) if t.state == SessionTaskState::Failed => {
                            summary.reaped += 1;
                            summary.outcomes.push(ReapOutcome {
                                session_id: session_id.to_string(),
                                task_id: task_id.clone(),
                                status: "reaped".to_string(),
                                detail: format!("marked failed: re-attach error: {e}"),
                            });
                        }
                        Ok(_) => {
                            summary.skipped += 1;
                            summary.outcomes.push(ReapOutcome {
                                session_id: session_id.to_string(),
                                task_id: task_id.clone(),
                                status: "skipped".to_string(),
                                detail: "already terminal after re-attach failure".to_string(),
                            });
                        }
                        Err(fe) => {
                            warn!(
                                session_id = %session_id,
                                task_id = %task_id,
                                error = %fe,
                                "Session task reaper: failed to mark task as orphaned after re-attach failure"
                            );
                            summary.skipped += 1;
                            summary.outcomes.push(ReapOutcome {
                                session_id: session_id.to_string(),
                                task_id: task_id.clone(),
                                status: "error".to_string(),
                                detail: fe.to_string(),
                            });
                        }
                    }
                    continue;
                }
            }
        }

        // Non-reattachable or attempts exhausted: fail as orphaned.
        let update = SessionTaskUpdate {
            state: Some(SessionTaskState::Failed),
            error: Some(TaskError {
                kind: "orphaned".to_string(),
                message: "worker heartbeat stopped".to_string(),
            }),
            // Guard against a concurrent reaper double-bumping: only apply if
            // the attempt still matches the snapshot we inspected.
            expected_attempt: Some(task.attempt),
            // Supersede the executor's attempt so its fenced writes
            // (heartbeats, progress, messages) are rejected from now on.
            increment_attempt: true,
            ..Default::default()
        };

        match registry.update(session_id, &task_id, update).await {
            Ok(Some(task)) if task.state == SessionTaskState::Failed => {
                info!(
                    session_id = %session_id,
                    task_id = %task_id,
                    "Session task reaper: marked orphaned task failed"
                );
                summary.reaped += 1;
                summary.outcomes.push(ReapOutcome {
                    session_id: session_id.to_string(),
                    task_id: task_id.clone(),
                    status: "reaped".to_string(),
                    detail: "marked failed: orphaned".to_string(),
                });
            }
            Ok(_) => {
                // Task was already terminal or not found — skip.
                summary.skipped += 1;
                summary.outcomes.push(ReapOutcome {
                    session_id: session_id.to_string(),
                    task_id: task_id.clone(),
                    status: "skipped".to_string(),
                    detail: "already terminal or missing".to_string(),
                });
            }
            Err(e) => {
                warn!(
                    session_id = %session_id,
                    task_id = %task_id,
                    error = %e,
                    "Session task reaper: failed to update task"
                );
                summary.skipped += 1;
                summary.outcomes.push(ReapOutcome {
                    session_id: session_id.to_string(),
                    task_id: task_id.clone(),
                    status: "error".to_string(),
                    detail: e.to_string(),
                });
            }
        }
    }

    info!(
        candidates = summary.candidates,
        reaped = summary.reaped,
        reattached = summary.reattached,
        skipped = summary.skipped,
        "Session task reaper pass completed"
    );

    Ok(serde_json::to_value(summary)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use everruns_core::session_task::{
        CreateSessionTask, NewTaskMessage, SessionTask, SessionTaskFilter, SessionTaskRegistry,
        SessionTaskState, TaskExecutor, TaskLinks, TaskMessage, TaskWakePolicy, apply_task_update,
        new_session_task,
    };
    use everruns_core::{Result as CoreResult, SessionId};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    // -------------------------------------------------------------------------
    // In-memory mock registry (no server crate dependency)
    // -------------------------------------------------------------------------

    #[derive(Default, Clone)]
    struct MockRegistry {
        tasks: Arc<Mutex<HashMap<String, SessionTask>>>,
    }

    #[async_trait]
    impl SessionTaskRegistry for MockRegistry {
        async fn create(&self, input: CreateSessionTask) -> CoreResult<SessionTask> {
            let task = new_session_task(input, chrono::Utc::now());
            self.tasks
                .lock()
                .unwrap()
                .insert(task.id.clone(), task.clone());
            Ok(task)
        }

        async fn get(
            &self,
            _session_id: SessionId,
            task_id: &str,
        ) -> CoreResult<Option<SessionTask>> {
            Ok(self.tasks.lock().unwrap().get(task_id).cloned())
        }

        async fn list(
            &self,
            _session_id: SessionId,
            _filter: Option<&SessionTaskFilter>,
        ) -> CoreResult<Vec<SessionTask>> {
            Ok(self.tasks.lock().unwrap().values().cloned().collect())
        }

        async fn update(
            &self,
            _session_id: SessionId,
            task_id: &str,
            update: SessionTaskUpdate,
        ) -> CoreResult<Option<SessionTask>> {
            let mut tasks = self.tasks.lock().unwrap();
            let Some(task) = tasks.get_mut(task_id) else {
                return Ok(None);
            };
            apply_task_update(task, update, chrono::Utc::now());
            Ok(Some(task.clone()))
        }

        async fn request_cancel(
            &self,
            _session_id: SessionId,
            _task_id: &str,
        ) -> CoreResult<Option<SessionTask>> {
            Ok(None)
        }

        async fn record_message(
            &self,
            _session_id: SessionId,
            _task_id: &str,
            _message: NewTaskMessage,
        ) -> CoreResult<TaskMessage> {
            Err(anyhow::anyhow!("not implemented").into())
        }

        async fn list_messages(
            &self,
            _session_id: SessionId,
            _task_id: &str,
            _limit: Option<u32>,
            _after_id: Option<&str>,
        ) -> CoreResult<Vec<TaskMessage>> {
            Ok(vec![])
        }
    }

    // -------------------------------------------------------------------------
    // Test executor: records start() calls, optionally fails them.
    // -------------------------------------------------------------------------

    const TEST_REATTACHABLE_KIND: &str = "test_reattachable";
    const TEST_NON_REATTACHABLE_KIND: &str = "test_non_reattachable";
    const TEST_START_FAIL_KIND: &str = "test_start_fail";

    struct TestReattachableExecutor {
        start_calls: Arc<Mutex<Vec<i32>>>, // records attempt numbers seen by start()
    }

    #[async_trait]
    impl TaskExecutor for TestReattachableExecutor {
        fn kind(&self) -> &str {
            TEST_REATTACHABLE_KIND
        }

        fn can_reattach(&self) -> bool {
            true
        }

        async fn start(
            &self,
            task: &SessionTask,
            _context: &everruns_core::traits::ToolContext,
        ) -> CoreResult<()> {
            self.start_calls.lock().unwrap().push(task.attempt);
            Ok(())
        }

        async fn cancel(
            &self,
            _task: &SessionTask,
            _context: &everruns_core::traits::ToolContext,
        ) -> CoreResult<()> {
            Ok(())
        }
    }

    struct TestNonReattachableExecutor;

    #[async_trait]
    impl TaskExecutor for TestNonReattachableExecutor {
        fn kind(&self) -> &str {
            TEST_NON_REATTACHABLE_KIND
        }

        async fn cancel(
            &self,
            _task: &SessionTask,
            _context: &everruns_core::traits::ToolContext,
        ) -> CoreResult<()> {
            Ok(())
        }
    }

    struct TestStartFailExecutor;

    #[async_trait]
    impl TaskExecutor for TestStartFailExecutor {
        fn kind(&self) -> &str {
            TEST_START_FAIL_KIND
        }

        fn can_reattach(&self) -> bool {
            true
        }

        async fn start(
            &self,
            _task: &SessionTask,
            _context: &everruns_core::traits::ToolContext,
        ) -> CoreResult<()> {
            Err(everruns_core::error::AgentLoopError::tool(
                "simulated start failure",
            ))
        }

        async fn cancel(
            &self,
            _task: &SessionTask,
            _context: &everruns_core::traits::ToolContext,
        ) -> CoreResult<()> {
            Ok(())
        }
    }

    // Minimal in-memory storage for test ToolContext construction.
    #[derive(Default)]
    struct MockStorageStore;

    #[async_trait]
    impl everruns_core::traits::SessionStorageStore for MockStorageStore {
        async fn set_value(
            &self,
            _session_id: SessionId,
            _key: &str,
            _value: &str,
        ) -> CoreResult<()> {
            Ok(())
        }

        async fn get_value(
            &self,
            _session_id: SessionId,
            _key: &str,
        ) -> CoreResult<Option<String>> {
            Ok(None)
        }

        async fn delete_value(&self, _session_id: SessionId, _key: &str) -> CoreResult<bool> {
            Ok(false)
        }

        async fn list_keys(
            &self,
            _session_id: SessionId,
        ) -> CoreResult<Vec<everruns_core::KeyInfo>> {
            Ok(vec![])
        }

        async fn set_secret(
            &self,
            _session_id: SessionId,
            _name: &str,
            _value: &str,
        ) -> CoreResult<()> {
            Ok(())
        }

        async fn get_secret(
            &self,
            _session_id: SessionId,
            _name: &str,
        ) -> CoreResult<Option<String>> {
            Ok(None)
        }

        async fn delete_secret(&self, _session_id: SessionId, _name: &str) -> CoreResult<bool> {
            Ok(false)
        }

        async fn list_secrets(
            &self,
            _session_id: SessionId,
        ) -> CoreResult<Vec<everruns_core::SecretInfo>> {
            Ok(vec![])
        }
    }

    // Helper: run the reaper logic inline, calling executors directly via the
    // provided lookup function instead of inventory, so tests don't need
    // `inventory::submit!` in a lib-test context.
    async fn run_reaper_with_executor_lookup(
        orphans: Vec<(SessionId, String)>,
        registry: Arc<MockRegistry>,
        input: &SessionTaskReaperInput,
        // executor_for(kind) → None means "not found / non-reattachable"
        executor_for: impl Fn(&str) -> Option<Arc<dyn TaskExecutor>>,
    ) -> serde_json::Value {
        let storage: Arc<dyn everruns_core::traits::SessionStorageStore> =
            Arc::new(MockStorageStore);

        let mut reaped = 0usize;
        let mut reattached = 0usize;
        let mut skipped = 0usize;

        for (session_id, task_id) in &orphans {
            // Get current task snapshot.
            let task = match registry.get(*session_id, task_id).await {
                Ok(Some(t)) => t,
                _ => {
                    skipped += 1;
                    continue;
                }
            };

            if task.state.is_terminal() {
                skipped += 1;
                continue;
            }

            let should_reattach = executor_for(&task.kind).is_some_and(|exec| exec.can_reattach())
                && (task.attempt as i64) < input.max_attempts;

            if should_reattach {
                let new_attempt = task.attempt + 1;
                let supersede = SessionTaskUpdate {
                    worker_id: Some("reaper".to_string()),
                    heartbeat_at: Some(chrono::Utc::now()),
                    state_detail: Some(format!("re-attached (attempt {new_attempt})")),
                    expected_attempt: Some(task.attempt),
                    increment_attempt: true,
                    ..Default::default()
                };
                let updated_task = match registry.update(*session_id, task_id, supersede).await {
                    Ok(Some(t)) if t.attempt == new_attempt => t,
                    _ => {
                        skipped += 1;
                        continue;
                    }
                };

                let ctx = everruns_core::traits::ToolContext::new(*session_id)
                    .with_storage_store_arc(storage.clone())
                    .with_session_task_registry(registry.clone());

                let executor = executor_for(&task.kind).unwrap();
                match executor.start(&updated_task, &ctx).await {
                    Ok(()) => {
                        reattached += 1;
                        continue;
                    }
                    Err(e) => {
                        // Fall back to orphaned-fail.
                        let fail = SessionTaskUpdate {
                            state: Some(SessionTaskState::Failed),
                            error: Some(TaskError {
                                kind: "orphaned".to_string(),
                                message: format!("worker heartbeat stopped; re-attach failed: {e}"),
                            }),
                            expected_attempt: Some(new_attempt),
                            ..Default::default()
                        };
                        match registry.update(*session_id, task_id, fail).await {
                            Ok(Some(t)) if t.state == SessionTaskState::Failed => {
                                reaped += 1;
                            }
                            _ => {
                                skipped += 1;
                            }
                        }
                        continue;
                    }
                }
            }

            // Non-reattachable or exhausted: fail as orphaned.
            let update = SessionTaskUpdate {
                state: Some(SessionTaskState::Failed),
                error: Some(TaskError {
                    kind: "orphaned".to_string(),
                    message: "worker heartbeat stopped".to_string(),
                }),
                increment_attempt: true,
                ..Default::default()
            };
            match registry.update(*session_id, task_id, update).await {
                Ok(Some(task)) if task.state == SessionTaskState::Failed => {
                    reaped += 1;
                }
                _ => {
                    skipped += 1;
                }
            }
        }

        serde_json::json!({
            "candidates": orphans.len(),
            "reaped": reaped,
            "reattached": reattached,
            "skipped": skipped,
        })
    }

    // Legacy helper: original tests that don't need executor lookup.
    async fn run_reaper(
        orphans: Vec<(SessionId, String)>,
        registry: Arc<MockRegistry>,
        input: &SessionTaskReaperInput,
    ) -> serde_json::Value {
        // No executor registered for the background_tool kind in these tests.
        run_reaper_with_executor_lookup(orphans, registry, input, |_| None).await
    }

    // -------------------------------------------------------------------------
    // Tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn reaper_fails_stale_heartbeat_tasks() {
        let registry = Arc::new(MockRegistry::default());
        let session_id = SessionId::new();

        // Seed a running task.
        let task = registry
            .create(CreateSessionTask {
                session_id,
                id: None,
                kind: "background_tool".to_string(),
                display_name: "Orphaned task".to_string(),
                spec: serde_json::json!({}),
                state: SessionTaskState::Running,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();

        // Simulate: list_orphaned returns this task (heartbeat logic tested in server crate).
        let orphans = vec![(session_id, task.id.clone())];
        let input = SessionTaskReaperInput::default();
        let result = run_reaper(orphans, registry.clone(), &input).await;

        assert_eq!(result["reaped"], 1);
        assert_eq!(result["skipped"], 0);

        let updated = registry.get(session_id, &task.id).await.unwrap().unwrap();
        assert_eq!(updated.state, SessionTaskState::Failed);
        assert_eq!(updated.error.as_ref().unwrap().kind, "orphaned");
        assert_eq!(
            updated.attempt, 2,
            "orphan reap must supersede the executor's attempt"
        );
    }

    #[tokio::test]
    async fn reaper_skips_already_terminal_tasks() {
        let registry = Arc::new(MockRegistry::default());
        let session_id = SessionId::new();

        // Seed a task that is already succeeded.
        let task = registry
            .create(CreateSessionTask {
                session_id,
                id: None,
                kind: "background_tool".to_string(),
                display_name: "Already done".to_string(),
                spec: serde_json::json!({}),
                state: SessionTaskState::Succeeded,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();

        // Simulate orphan list still containing this task (race condition in real code).
        let orphans = vec![(session_id, task.id.clone())];
        let input = SessionTaskReaperInput::default();
        let result = run_reaper(orphans, registry.clone(), &input).await;

        // apply_task_update won't move Succeeded→Failed, so it stays succeeded.
        // Our run_reaper counts it as skipped when state isn't Failed after update.
        let updated = registry.get(session_id, &task.id).await.unwrap().unwrap();
        // State should remain Succeeded (terminal→terminal transition is a no-op).
        assert_eq!(updated.state, SessionTaskState::Succeeded);
        assert_eq!(result["candidates"], 1);
    }

    #[tokio::test]
    async fn empty_orphan_list_produces_zero_reap() {
        let registry = Arc::new(MockRegistry::default());
        let input = SessionTaskReaperInput::default();
        let result = run_reaper(vec![], registry, &input).await;

        assert_eq!(result["candidates"], 0);
        assert_eq!(result["reaped"], 0);
        assert_eq!(result["skipped"], 0);
    }

    // -------------------------------------------------------------------------
    // Re-attach tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn reaper_reattaches_reattachable_kind() {
        let registry = Arc::new(MockRegistry::default());
        let session_id = SessionId::new();
        let start_calls = Arc::new(Mutex::new(Vec::<i32>::new()));
        let start_calls_clone = start_calls.clone();

        let task = registry
            .create(CreateSessionTask {
                session_id,
                id: None,
                kind: TEST_REATTACHABLE_KIND.to_string(),
                display_name: "Re-attachable".to_string(),
                spec: serde_json::json!({}),
                state: SessionTaskState::Running,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();
        assert_eq!(task.attempt, 1);

        let orphans = vec![(session_id, task.id.clone())];
        let input = SessionTaskReaperInput::default();
        let result =
            run_reaper_with_executor_lookup(orphans, registry.clone(), &input, move |kind| {
                if kind == TEST_REATTACHABLE_KIND {
                    Some(Arc::new(TestReattachableExecutor {
                        start_calls: start_calls_clone.clone(),
                    }) as Arc<dyn TaskExecutor>)
                } else {
                    None
                }
            })
            .await;

        assert_eq!(result["reattached"], 1, "should re-attach");
        assert_eq!(result["reaped"], 0, "should not fail as orphaned");

        let updated = registry.get(session_id, &task.id).await.unwrap().unwrap();
        // Attempt bumped, state still running.
        assert_eq!(updated.attempt, 2, "attempt must be bumped on re-attach");
        assert_eq!(updated.state, SessionTaskState::Running);

        // start() was called with the new attempt.
        let calls = start_calls.lock().unwrap();
        assert_eq!(*calls, vec![2i32]);
    }

    #[tokio::test]
    async fn reaper_fails_non_reattachable_kind_as_orphaned() {
        let registry = Arc::new(MockRegistry::default());
        let session_id = SessionId::new();

        let task = registry
            .create(CreateSessionTask {
                session_id,
                id: None,
                kind: TEST_NON_REATTACHABLE_KIND.to_string(),
                display_name: "Non-reattachable".to_string(),
                spec: serde_json::json!({}),
                state: SessionTaskState::Running,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();

        let orphans = vec![(session_id, task.id.clone())];
        let input = SessionTaskReaperInput::default();
        let result = run_reaper_with_executor_lookup(orphans, registry.clone(), &input, |kind| {
            if kind == TEST_NON_REATTACHABLE_KIND {
                Some(Arc::new(TestNonReattachableExecutor) as Arc<dyn TaskExecutor>)
            } else {
                None
            }
        })
        .await;

        assert_eq!(result["reaped"], 1, "should fail as orphaned");
        assert_eq!(result["reattached"], 0);

        let updated = registry.get(session_id, &task.id).await.unwrap().unwrap();
        assert_eq!(updated.state, SessionTaskState::Failed);
        assert_eq!(updated.error.as_ref().unwrap().kind, "orphaned");
        assert_eq!(updated.attempt, 2);
    }

    #[tokio::test]
    async fn reaper_fails_as_orphaned_when_attempts_exhausted() {
        let registry = Arc::new(MockRegistry::default());
        let session_id = SessionId::new();

        // Create a task and manually bump its attempt to 3 (== max_attempts).
        let task = registry
            .create(CreateSessionTask {
                session_id,
                id: None,
                kind: TEST_REATTACHABLE_KIND.to_string(),
                display_name: "Exhausted".to_string(),
                spec: serde_json::json!({}),
                state: SessionTaskState::Running,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();
        // Simulate two prior re-attaches (attempt = 3).
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
        let task = registry.get(session_id, &task.id).await.unwrap().unwrap();
        assert_eq!(task.attempt, 3);

        let orphans = vec![(session_id, task.id.clone())];
        // max_attempts = 3: attempt 3 >= 3 so no re-attach.
        let input = SessionTaskReaperInput {
            max_attempts: 3,
            ..Default::default()
        };
        let result = run_reaper_with_executor_lookup(orphans, registry.clone(), &input, |kind| {
            if kind == TEST_REATTACHABLE_KIND {
                Some(Arc::new(TestReattachableExecutor {
                    start_calls: Arc::new(Mutex::new(vec![])),
                }) as Arc<dyn TaskExecutor>)
            } else {
                None
            }
        })
        .await;

        assert_eq!(
            result["reaped"], 1,
            "should fail as orphaned when exhausted"
        );
        assert_eq!(result["reattached"], 0);

        let updated = registry.get(session_id, &task.id).await.unwrap().unwrap();
        assert_eq!(updated.state, SessionTaskState::Failed);
        assert_eq!(updated.error.as_ref().unwrap().kind, "orphaned");
    }

    #[tokio::test]
    async fn reaper_falls_back_to_orphaned_fail_when_start_errors() {
        let registry = Arc::new(MockRegistry::default());
        let session_id = SessionId::new();

        let task = registry
            .create(CreateSessionTask {
                session_id,
                id: None,
                kind: TEST_START_FAIL_KIND.to_string(),
                display_name: "Fails on start".to_string(),
                spec: serde_json::json!({}),
                state: SessionTaskState::Running,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();

        let orphans = vec![(session_id, task.id.clone())];
        let input = SessionTaskReaperInput::default();
        let result = run_reaper_with_executor_lookup(orphans, registry.clone(), &input, |kind| {
            if kind == TEST_START_FAIL_KIND {
                Some(Arc::new(TestStartFailExecutor) as Arc<dyn TaskExecutor>)
            } else {
                None
            }
        })
        .await;

        // Falls back to orphaned-fail.
        assert_eq!(
            result["reaped"], 1,
            "should fail as orphaned after start error"
        );
        assert_eq!(result["reattached"], 0);

        let updated = registry.get(session_id, &task.id).await.unwrap().unwrap();
        assert_eq!(updated.state, SessionTaskState::Failed);
        assert_eq!(updated.error.as_ref().unwrap().kind, "orphaned");
        // Attempt bumped once (1 -> 2) by the re-attach supersede; the fail
        // update is fenced on that bumped attempt and does not bump again.
        assert_eq!(updated.attempt, 2);
    }
}
