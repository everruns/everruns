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
use everruns_core::tool_context::ToolContext;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::worker_adapters::{AdapterSessionFileStore, WorkerAdapters};

/// How long a heartbeat may be stale before a task is considered orphaned.
const DEFAULT_STALE_AFTER_SECONDS: i64 = 5 * 60; // 5 minutes

/// How many orphaned tasks to process per reaper pass.
const DEFAULT_LIMIT: i64 = 50;

/// Default maximum re-attach attempts before a task is failed as orphaned.
const DEFAULT_MAX_ATTEMPTS: i64 = 3;

/// Default retention TTL for terminal tasks (30 days). Terminal task records,
/// their messages, and their result_path artifacts are pruned once their
/// `finished_at` ages past this. Configurable via `SESSION_TASK_RETENTION_TTL_SECONDS`
/// through the same env path the reaper interval uses. A value of 0 disables
/// retention pruning entirely (records live forever — the pre-EVE-580 behavior).
//
// Extension point: this is a single global TTL for v1. A future per-org
// override (NOT this issue) would resolve the effective TTL per task's owning
// org here / in the prune query rather than using one constant.
const DEFAULT_RETENTION_TTL_SECONDS: i64 = 30 * 24 * 60 * 60;

/// How many terminal tasks to prune per reaper pass. Bounds the data-deletion
/// work so a large backlog can't wedge the tick or blow memory; a backlog
/// drains across successive ticks.
const DEFAULT_RETENTION_LIMIT: i64 = 100;

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
    /// Retention TTL (seconds) for terminal task records. Terminal tasks whose
    /// `finished_at` is older than this are pruned (rows + messages +
    /// artifacts). `0` disables retention pruning (EVE-580).
    #[serde(default = "default_retention_ttl_seconds")]
    pub retention_ttl_seconds: i64,
    /// Max terminal tasks to prune per pass (bounds data-deletion work).
    #[serde(default = "default_retention_limit")]
    pub retention_limit: i64,
}

fn default_max_attempts() -> i64 {
    DEFAULT_MAX_ATTEMPTS
}

fn default_retention_ttl_seconds() -> i64 {
    DEFAULT_RETENTION_TTL_SECONDS
}

fn default_retention_limit() -> i64 {
    DEFAULT_RETENTION_LIMIT
}

impl Default for SessionTaskReaperInput {
    fn default() -> Self {
        Self {
            stale_after_seconds: DEFAULT_STALE_AFTER_SECONDS,
            limit: DEFAULT_LIMIT,
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            retention_ttl_seconds: DEFAULT_RETENTION_TTL_SECONDS,
            retention_limit: DEFAULT_RETENTION_LIMIT,
        }
    }
}

impl SessionTaskReaperInput {
    /// Build the reaper input, overriding the retention TTL from
    /// `SESSION_TASK_RETENTION_TTL_SECONDS` when set (EVE-580). This is the
    /// same configuration path the schedule bootstrap uses to seed the durable
    /// activity input, so operators tune retention without code changes. A
    /// value of 0 disables retention pruning; invalid values fall back to the
    /// default with a warning.
    pub fn from_env() -> Self {
        let mut input = Self::default();
        if let Ok(v) = std::env::var("SESSION_TASK_RETENTION_TTL_SECONDS") {
            match v.parse::<i64>() {
                Ok(secs) if secs >= 0 => input.retention_ttl_seconds = secs,
                _ => warn!(
                    value = %v,
                    "Invalid SESSION_TASK_RETENTION_TTL_SECONDS; using default"
                ),
            }
        }
        input
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
    /// Terminal tasks pruned by the retention pass this tick (EVE-580).
    pruned: usize,
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

    // Reconcile orphans through the shared loop so tests exercise this exact
    // code path. Production supplies the global inventory executor lookup and an
    // adapter-backed ToolContext; tests inject their own.
    let mut summary = reconcile_orphans(
        candidates,
        &registry,
        input,
        find_task_executor,
        |session_id| {
            ToolContext::with_stores(
                session_id,
                std::sync::Arc::new(AdapterSessionFileStore::new(adapters.clone())),
                adapters.storage_store(),
            )
            .with_session_task_registry(registry.clone())
            .with_egress_service_opt(adapters.egress_service())
        },
    )
    .await;

    // Retention pass (EVE-580): prune terminal task records, their messages,
    // and their result_path artifacts once finished_at ages past the TTL.
    summary.pruned = run_retention_pass(input, |ttl, limit| {
        adapters.prune_terminal_session_tasks(ttl, limit)
    })
    .await;

    info!(
        candidates = summary.candidates,
        reaped = summary.reaped,
        reattached = summary.reattached,
        skipped = summary.skipped,
        pruned = summary.pruned,
        "Session task reaper pass completed"
    );

    Ok(serde_json::to_value(summary)?)
}

/// Reconcile one batch of orphaned tasks: re-attach re-attachable kinds (up to
/// `max_attempts`) or fail the rest as orphaned, so lifecycle invariants,
/// `task.updated` events, and wake_policy fire exactly as for any other
/// terminal transition. Factored out of `execute_reaper_activity` so production
/// and tests drive the identical loop — production passes the global
/// `find_task_executor` and an adapter-backed context builder; tests inject
/// their own.
async fn reconcile_orphans<F, C>(
    candidates: Vec<(everruns_core::SessionId, String)>,
    registry: &std::sync::Arc<dyn everruns_core::session_task::SessionTaskRegistry>,
    input: &SessionTaskReaperInput,
    executor_for: F,
    make_reattach_ctx: C,
) -> ReapSummary
where
    F: Fn(&str) -> Option<std::sync::Arc<dyn everruns_core::session_task::TaskExecutor>>,
    C: Fn(everruns_core::SessionId) -> ToolContext,
{
    let mut summary = ReapSummary {
        candidates: candidates.len(),
        reaped: 0,
        reattached: 0,
        skipped: 0,
        pruned: 0,
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
        let should_reattach = executor_for(&task.kind)
            .is_some_and(|exec| exec.can_reattach_task(&task))
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

            // Build a minimal ToolContext for the executor. Background-tool
            // reattach needs the session file store to persist fresh artifacts.
            let ctx = make_reattach_ctx(session_id);

            let executor = executor_for(&task.kind).expect("checked above");
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

    summary
}

/// Run the retention pass of one reaper tick (EVE-580): prune terminal task
/// records, their messages, and their result_path artifacts once `finished_at`
/// ages past the configured TTL. The prune itself is a bounded, by-age,
/// data-destruction operation keyed strictly on terminal state +
/// `finished_at < cutoff` (enforced in the storage backend), so it can never
/// touch live/queued/running tasks. A TTL of 0 disables retention entirely.
/// Failures are logged but never fail the reaper tick. Returns the number of
/// tasks pruned this pass.
///
/// `prune` is injected so the retention logic is testable without a full
/// `WorkerAdapters` mock; in production it forwards to
/// `adapters.prune_terminal_session_tasks`.
async fn run_retention_pass<F, Fut, E>(input: &SessionTaskReaperInput, prune: F) -> usize
where
    F: FnOnce(chrono::Duration, i64) -> Fut,
    Fut: std::future::Future<Output = std::result::Result<usize, E>>,
    E: std::fmt::Display,
{
    if input.retention_ttl_seconds <= 0 {
        return 0;
    }
    let ttl = chrono::Duration::seconds(input.retention_ttl_seconds);
    // Never hand a non-positive batch size to the destructive prune: a
    // misconfigured/legacy input must not become an unbounded delete (Postgres
    // treats `LIMIT <= 0` as unlimited). The storage backend also clamps and
    // caps; this keeps the worker path self-bounding. EVE-580 review.
    let limit = input.retention_limit.max(1);
    match prune(ttl, limit).await {
        Ok(pruned) => {
            if pruned > 0 {
                info!(
                    pruned,
                    ttl_seconds = input.retention_ttl_seconds,
                    "Session task reaper: pruned terminal tasks past retention TTL"
                );
            }
            pruned
        }
        Err(e) => {
            warn!(error = %e, "Session task reaper: retention prune failed (best-effort)");
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use everruns_core::session_task::{
        CreateSessionTask, NewTaskMessage, SessionTask, SessionTaskFilter, SessionTaskRegistry,
        SessionTaskState, TASK_KIND_SUBAGENT, TaskExecutor, TaskLinks, TaskMessage, TaskWakePolicy,
        apply_task_update, new_session_task,
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
            context: &everruns_core::tool_context::ToolContext,
        ) -> CoreResult<()> {
            assert!(
                context.file_store.is_some(),
                "re-attach context must include a session file store"
            );
            self.start_calls.lock().unwrap().push(task.attempt);
            Ok(())
        }

        async fn cancel(
            &self,
            _task: &SessionTask,
            _context: &everruns_core::tool_context::ToolContext,
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
            _context: &everruns_core::tool_context::ToolContext,
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
            _context: &everruns_core::tool_context::ToolContext,
        ) -> CoreResult<()> {
            Err(everruns_core::error::AgentLoopError::tool(
                "simulated start failure",
            ))
        }

        async fn cancel(
            &self,
            _task: &SessionTask,
            _context: &everruns_core::tool_context::ToolContext,
        ) -> CoreResult<()> {
            Ok(())
        }
    }

    // Minimal in-memory storage for test ToolContext construction.
    #[derive(Default)]
    struct MockStorageStore;

    #[async_trait]
    impl everruns_core::session_services::SessionStorageStore for MockStorageStore {
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
        ) -> CoreResult<Vec<everruns_core::session_services::KeyInfo>> {
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
        ) -> CoreResult<Vec<everruns_core::session_services::SecretInfo>> {
            Ok(vec![])
        }
    }

    #[derive(Default)]
    struct MockFileStore;

    #[async_trait]
    impl everruns_core::session_files::SessionFileSystem for MockFileStore {
        async fn read_file(
            &self,
            _session_id: SessionId,
            _path: &str,
        ) -> CoreResult<Option<everruns_core::session_file::SessionFile>> {
            Ok(None)
        }

        async fn write_file(
            &self,
            _session_id: SessionId,
            path: &str,
            content: &str,
            encoding: &str,
        ) -> CoreResult<everruns_core::session_file::SessionFile> {
            let now = chrono::Utc::now();
            Ok(everruns_core::session_file::SessionFile {
                id: uuid::Uuid::new_v4(),
                session_id: _session_id.uuid(),
                path: path.to_string(),
                name: everruns_core::session_file::FileInfo::name_from_path(path),
                content: Some(content.to_string()),
                encoding: encoding.to_string(),
                is_directory: false,
                is_readonly: false,
                created_at: chrono::Utc::now(),
                updated_at: now,
                size_bytes: content.len() as i64,
            })
        }

        async fn delete_file(
            &self,
            _session_id: SessionId,
            _path: &str,
            _recursive: bool,
        ) -> CoreResult<bool> {
            Ok(false)
        }

        async fn list_directory(
            &self,
            _session_id: SessionId,
            _path: &str,
        ) -> CoreResult<Vec<everruns_core::session_file::FileInfo>> {
            Ok(vec![])
        }

        async fn stat_file(
            &self,
            _session_id: SessionId,
            _path: &str,
        ) -> CoreResult<Option<everruns_core::session_file::FileStat>> {
            Ok(None)
        }

        async fn grep_files(
            &self,
            _session_id: SessionId,
            _pattern: &str,
            _path_pattern: Option<&str>,
        ) -> CoreResult<Vec<everruns_core::session_file::GrepMatch>> {
            Ok(vec![])
        }

        async fn create_directory(
            &self,
            session_id: SessionId,
            path: &str,
        ) -> CoreResult<everruns_core::session_file::FileInfo> {
            let now = chrono::Utc::now();
            Ok(everruns_core::session_file::FileInfo {
                id: uuid::Uuid::new_v4(),
                session_id: session_id.uuid(),
                path: path.to_string(),
                name: everruns_core::session_file::FileInfo::name_from_path(path),
                is_directory: true,
                is_readonly: false,
                size_bytes: 0,
                created_at: now,
                updated_at: now,
            })
        }

        fn is_mount_resolver(&self) -> bool {
            false
        }
    }

    // Drive the production `reconcile_orphans` loop with an injected executor
    // lookup (so tests don't need `inventory::submit!` in a lib-test context)
    // and in-memory stores for the re-attach ToolContext. This exercises the
    // real reaper code path rather than a reimplementation.
    async fn run_reaper_with_executor_lookup(
        orphans: Vec<(SessionId, String)>,
        registry: Arc<MockRegistry>,
        input: &SessionTaskReaperInput,
        // executor_for(kind) → None means "not found / non-reattachable"
        executor_for: impl Fn(&str) -> Option<Arc<dyn TaskExecutor>>,
    ) -> serde_json::Value {
        let storage: Arc<dyn everruns_core::session_services::SessionStorageStore> =
            Arc::new(MockStorageStore);
        let file_store: Arc<dyn everruns_core::session_files::SessionFileSystem> =
            Arc::new(MockFileStore);
        let registry_dyn: Arc<dyn SessionTaskRegistry> = registry;

        let summary =
            reconcile_orphans(orphans, &registry_dyn, input, executor_for, |session_id| {
                ToolContext::with_stores(session_id, file_store.clone(), storage.clone())
                    .with_session_task_registry(registry_dyn.clone())
            })
            .await;

        serde_json::json!({
            "candidates": summary.candidates,
            "reaped": summary.reaped,
            "reattached": summary.reattached,
            "skipped": summary.skipped,
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
    async fn reaper_fails_orphaned_depth_two_subagent_task() {
        let registry = Arc::new(MockRegistry::default());
        let root_session_id = SessionId::new();
        let child_session_id = SessionId::new();
        let grandchild_session_id = SessionId::new();

        let parent_task = registry
            .create(CreateSessionTask {
                session_id: root_session_id,
                id: None,
                kind: TASK_KIND_SUBAGENT.to_string(),
                display_name: "Child".to_string(),
                spec: serde_json::json!({"mode": "background"}),
                state: SessionTaskState::Running,
                links: TaskLinks {
                    child_session_id: Some(child_session_id),
                    ..Default::default()
                },
                wake_policy: TaskWakePolicy::OnTerminal,
            })
            .await
            .unwrap();

        let grandchild_task = registry
            .create(CreateSessionTask {
                session_id: child_session_id,
                id: None,
                kind: TASK_KIND_SUBAGENT.to_string(),
                display_name: "Grandchild".to_string(),
                spec: serde_json::json!({"mode": "background"}),
                state: SessionTaskState::Running,
                links: TaskLinks {
                    child_session_id: Some(grandchild_session_id),
                    ..Default::default()
                },
                wake_policy: TaskWakePolicy::OnTerminal,
            })
            .await
            .unwrap();

        // Simulate the nested background watcher disappearing after it had
        // heartbeated. The storage query that detects stale heartbeats is
        // covered elsewhere; this proves the depth-2 subagent task goes through
        // the same terminal update path as any other orphan.
        let orphans = vec![(child_session_id, grandchild_task.id.clone())];
        let input = SessionTaskReaperInput::default();
        let result = run_reaper(orphans, registry.clone(), &input).await;

        assert_eq!(result["reaped"], 1);
        assert_eq!(result["reattached"], 0);
        assert_eq!(result["skipped"], 0);

        let updated_grandchild = registry
            .get(child_session_id, &grandchild_task.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated_grandchild.kind, TASK_KIND_SUBAGENT);
        assert_eq!(updated_grandchild.state, SessionTaskState::Failed);
        assert_eq!(
            updated_grandchild.error.as_ref().map(|e| e.kind.as_str()),
            Some("orphaned")
        );
        assert_eq!(
            updated_grandchild.attempt, 2,
            "orphan reap must fence the stale nested watcher attempt"
        );

        let unchanged_parent = registry
            .get(root_session_id, &parent_task.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(unchanged_parent.state, SessionTaskState::Running);
        assert_eq!(unchanged_parent.attempt, 1);
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
    // Retention config (EVE-580)
    // -------------------------------------------------------------------------

    #[test]
    fn default_input_enables_retention_with_thirty_day_ttl() {
        let input = SessionTaskReaperInput::default();
        assert_eq!(input.retention_ttl_seconds, 30 * 24 * 60 * 60);
        assert_eq!(input.retention_limit, 100);
    }

    #[test]
    fn retention_fields_round_trip_through_serde_and_default_when_absent() {
        // New fields serialize and deserialize.
        let input = SessionTaskReaperInput {
            retention_ttl_seconds: 1234,
            retention_limit: 7,
            ..Default::default()
        };
        let json = serde_json::to_value(&input).unwrap();
        assert_eq!(json["retention_ttl_seconds"], 1234);
        assert_eq!(json["retention_limit"], 7);

        // A legacy stored input without the retention fields deserializes with
        // the defaults applied (so existing durable schedules keep working).
        let legacy: SessionTaskReaperInput = serde_json::from_value(serde_json::json!({
            "stale_after_seconds": 300,
            "limit": 50,
        }))
        .unwrap();
        assert_eq!(legacy.retention_ttl_seconds, 30 * 24 * 60 * 60);
        assert_eq!(legacy.retention_limit, 100);
        assert_eq!(legacy.max_attempts, 3);
    }

    #[tokio::test]
    async fn retention_pass_invokes_prune_with_configured_ttl_and_limit() {
        let input = SessionTaskReaperInput {
            retention_ttl_seconds: 3600,
            retention_limit: 25,
            ..Default::default()
        };
        let seen = Arc::new(Mutex::new(None));
        let seen_clone = seen.clone();
        let pruned = run_retention_pass(&input, move |ttl, limit| {
            *seen_clone.lock().unwrap() = Some((ttl, limit));
            async move { std::result::Result::<usize, &str>::Ok(4) }
        })
        .await;

        assert_eq!(pruned, 4, "returns the prune count");
        let (ttl, limit) = seen.lock().unwrap().unwrap();
        assert_eq!(ttl, chrono::Duration::seconds(3600));
        assert_eq!(limit, 25);
    }

    #[tokio::test]
    async fn retention_pass_clamps_non_positive_limit_to_one() {
        // A misconfigured non-positive batch must never reach the destructive
        // prune as `<= 0` (Postgres treats LIMIT <= 0 as unlimited). EVE-580.
        for bad in [0_i64, -1, i64::MIN] {
            let input = SessionTaskReaperInput {
                retention_ttl_seconds: 3600,
                retention_limit: bad,
                ..Default::default()
            };
            let seen = Arc::new(Mutex::new(None));
            let seen_clone = seen.clone();
            run_retention_pass(&input, move |_ttl, limit| {
                *seen_clone.lock().unwrap() = Some(limit);
                async move { std::result::Result::<usize, &str>::Ok(0) }
            })
            .await;
            assert_eq!(
                seen.lock().unwrap().unwrap(),
                1,
                "non-positive retention_limit {bad} must be clamped to a positive batch"
            );
        }
    }

    #[tokio::test]
    async fn retention_pass_skips_prune_when_ttl_zero() {
        let input = SessionTaskReaperInput {
            retention_ttl_seconds: 0,
            ..Default::default()
        };
        let called = Arc::new(Mutex::new(false));
        let called_clone = called.clone();
        let pruned = run_retention_pass(&input, move |_ttl, _limit| {
            *called_clone.lock().unwrap() = true;
            async move { std::result::Result::<usize, &str>::Ok(99) }
        })
        .await;

        assert_eq!(pruned, 0, "TTL=0 disables retention");
        assert!(!*called.lock().unwrap(), "prune must not be invoked");
    }

    #[tokio::test]
    async fn retention_pass_swallows_prune_errors() {
        let input = SessionTaskReaperInput::default();
        let pruned = run_retention_pass(&input, |_ttl, _limit| async move {
            std::result::Result::<usize, &str>::Err("boom")
        })
        .await;
        assert_eq!(pruned, 0, "prune errors never fail the reaper tick");
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
