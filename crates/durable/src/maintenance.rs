//! Stale-task reaping: return abandoned tasks to the queue and terminalize
//! the workflows whose tasks died.
//!
//! Decision: the reaper is engine logic, so it lives here and not in each
//! host. A host runs one [`StaleTaskReaper`] (or calls [`reap_stale_tasks`]
//! from its own loop) and plugs in a [`ReapHandler`] for what a dead or sealed
//! task means in its domain, for example failing an agent turn and putting
//! the session back to idle. The engine part is fixed:
//!
//! - a dead task (attempts exhausted) records `ActivityFailed`, then fails its
//!   workflow with a compare-and-set ([`EventLog::try_fail_workflow`]). Only
//!   the reaper that wins the transition records `WorkflowFailed` and calls
//!   [`ReapHandler::workflow_failed`], so concurrent reapers on several
//!   replicas notify the host once.
//! - a sealed task (no forward progress across N recoveries, EVE-534) records
//!   `ActivityFailed`, marks its workflow `Failed`, then calls
//!   [`ReapHandler::task_sealed`].
//!
//! Standalone tasks (no workflow) are reaped the same way; there is no
//! workflow to fail, so the handler hears only about seals.
//!
//! [`EventLog::try_fail_workflow`]: crate::persistence::EventLog::try_fail_workflow

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tracing::{error, info, warn};

use crate::persistence::{
    DeadTaskInfo, ReclaimResult, RequeuedWorkflow, SealedTaskInfo, StoreError, WorkflowEventStore,
    WorkflowStatus,
};
use crate::task_events::{record_activity_failed, record_workflow_failed};
use crate::workflow::WorkflowError;

/// Error recorded for a dead task that carries no error of its own.
const UNRESPONSIVE_WORKER_ERROR: &str =
    "Worker became unresponsive after exhausting all retry attempts";

/// Host hooks for what reaped tasks mean to the application.
///
/// Every method has a do-nothing default, so a host overrides only what it
/// needs. The hooks run inline in the reap pass: keep them bounded, and spawn
/// slow side effects (such as reporting to an external service).
#[async_trait]
pub trait ReapHandler: Send + Sync {
    /// A dead task's workflow was moved to `Failed` by this reap pass.
    async fn workflow_failed(&self, dead: &DeadTaskInfo, error: &str) {
        let _ = (dead, error);
    }

    /// A task was sealed by the no-progress guard and its workflow failed.
    async fn task_sealed(&self, sealed: &SealedTaskInfo) {
        let _ = sealed;
    }

    /// The reap pass itself failed (the store was unreachable, say).
    fn reap_failed(&self, error: &StoreError) {
        let _ = error;
    }
}

/// A [`ReapHandler`] that adds nothing to the engine's own bookkeeping.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopReapHandler;

#[async_trait]
impl ReapHandler for NoopReapHandler {}

/// Reclaim stale tasks once and settle the dead and sealed ones.
///
/// Returns what the store reclaimed, so a caller can log or count it.
pub async fn reap_stale_tasks<S: WorkflowEventStore + ?Sized>(
    store: &S,
    stale_threshold: Duration,
    handler: &dyn ReapHandler,
) -> Result<ReclaimResult, StoreError> {
    let result = store.reclaim_stale_tasks(stale_threshold).await?;
    if !result.reclaimed_ids.is_empty() {
        info!(
            count = result.reclaimed_ids.len(),
            task_ids = ?result.reclaimed_ids,
            "Reclaimed stale tasks"
        );
    }
    for dead in &result.dead_tasks {
        settle_dead_task(store, dead, handler).await;
    }
    for sealed in &result.sealed_tasks {
        settle_sealed_task(store, sealed, handler).await;
    }
    Ok(result)
}

async fn settle_dead_task<S: WorkflowEventStore + ?Sized>(
    store: &S,
    dead: &DeadTaskInfo,
    handler: &dyn ReapHandler,
) {
    let error_msg = dead
        .last_error
        .clone()
        .unwrap_or_else(|| UNRESPONSIVE_WORKER_ERROR.to_string());
    info!(
        task_id = %dead.task_id,
        workflow_id = ?dead.workflow_id,
        activity_id = %dead.activity_id,
        "Notifying workflow of dead task"
    );
    record_activity_failed(
        store,
        dead.workflow_id,
        dead.activity_id.clone(),
        error_msg.clone(),
        false, // every attempt is spent
    )
    .await;

    let Some(workflow_id) = dead.workflow_id else {
        return;
    };
    match store
        .try_fail_workflow(workflow_id, WorkflowError::new(error_msg.clone()))
        .await
    {
        Ok(true) => {
            record_workflow_failed(store, workflow_id, error_msg.clone()).await;
            handler.workflow_failed(dead, &error_msg).await;
        }
        // Already terminal, or another reaper won the transition.
        Ok(false) => {}
        Err(error) => {
            error!(%workflow_id, %error, "Failed to terminalize exhausted workflow");
        }
    }
}

async fn settle_sealed_task<S: WorkflowEventStore + ?Sized>(
    store: &S,
    sealed: &SealedTaskInfo,
    handler: &dyn ReapHandler,
) {
    warn!(
        task_id = %sealed.task_id,
        workflow_id = ?sealed.workflow_id,
        reason = %sealed.reason,
        no_progress_count = sealed.no_progress_count,
        "Sealing non-progressing task"
    );
    let error_msg = format!(
        "task sealed: no_progress ({} recoveries)",
        sealed.no_progress_count
    );
    record_activity_failed(
        store,
        sealed.workflow_id,
        sealed.activity_id.clone(),
        error_msg.clone(),
        false,
    )
    .await;
    if let Some(workflow_id) = sealed.workflow_id
        && let Err(error) = store
            .update_workflow_status(
                workflow_id,
                WorkflowStatus::Failed,
                None,
                Some(WorkflowError::new(error_msg)),
            )
            .await
    {
        warn!(%workflow_id, %error, "Failed to mark sealed workflow failed");
    }
    handler.task_sealed(sealed).await;
}

/// Timing of a [`StaleTaskReaper`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReaperConfig {
    /// A claimed task whose last heartbeat is older than this is reclaimed.
    pub stale_threshold: Duration,
    /// Time between reap passes.
    pub interval: Duration,
    /// Also resume runs of this workflow type stranded between two steps
    /// (see [`requeue_stranded_workflows`]). `None`, the default, sweeps
    /// nothing: a workflow type may legitimately wait with no task.
    pub stranded_workflow_type: Option<&'static str>,
    /// How long a stranded run's last step must have been complete before a
    /// pass resumes it. A hand-off in separate writes is not mistaken for a
    /// stranded run while it is still under way.
    pub stranded_after: Duration,
}

impl Default for ReaperConfig {
    fn default() -> Self {
        Self {
            stale_threshold: Duration::from_secs(30),
            interval: Duration::from_secs(10),
            stranded_workflow_type: None,
            stranded_after: Duration::from_secs(60),
        }
    }
}

/// Most stranded runs one pass resumes.
const STRANDED_SWEEP_LIMIT: usize = 100;

/// Resume the runs of `workflow_type` stranded between two steps: Running,
/// with no pending or claimed task, and a last task that completed more than
/// `stranded_after` ago. Each one's last step is enqueued again
/// ([`TaskQueue::requeue_stranded_workflows`]), so its turn continues.
///
/// A step hands off to the next atomically
/// ([`TaskQueue::complete_task_and_hand_off`]), so this finds only runs a
/// client handed off in separate writes and lost in between.
///
/// [`TaskQueue::requeue_stranded_workflows`]: crate::TaskQueue::requeue_stranded_workflows
/// [`TaskQueue::complete_task_and_hand_off`]: crate::TaskQueue::complete_task_and_hand_off
pub async fn requeue_stranded_workflows<S: WorkflowEventStore + ?Sized>(
    store: &S,
    workflow_type: &str,
    stranded_after: Duration,
) -> Result<Vec<RequeuedWorkflow>, StoreError> {
    let requeued = store
        .requeue_stranded_workflows(workflow_type, stranded_after, STRANDED_SWEEP_LIMIT)
        .await?;
    for run in &requeued {
        warn!(
            workflow_id = %run.workflow_id,
            task_id = %run.task_id,
            activity_type = %run.activity_type,
            "Resumed a workflow stranded between steps"
        );
    }
    Ok(requeued)
}

/// Runs [`reap_stale_tasks`] on an interval.
///
/// Safe to run on every replica: reclaim is row-locked in the store and the
/// workflow transition is a compare-and-set, so each dead task is settled once.
pub struct StaleTaskReaper {
    store: Arc<dyn WorkflowEventStore>,
    config: ReaperConfig,
    handler: Arc<dyn ReapHandler>,
}

impl StaleTaskReaper {
    /// Create a reaper over `store` that reports to `handler`.
    pub fn new(
        store: Arc<dyn WorkflowEventStore>,
        config: ReaperConfig,
        handler: Arc<dyn ReapHandler>,
    ) -> Self {
        Self {
            store,
            config,
            handler,
        }
    }

    /// One reap pass. A store failure is logged and passed to
    /// [`ReapHandler::reap_failed`].
    pub async fn reap_once(&self) -> Option<ReclaimResult> {
        if let Some(workflow_type) = self.config.stranded_workflow_type
            && let Err(error) = requeue_stranded_workflows(
                self.store.as_ref(),
                workflow_type,
                self.config.stranded_after,
            )
            .await
        {
            error!(%error, "Failed to resume stranded workflows");
            self.handler.reap_failed(&error);
        }
        match reap_stale_tasks(
            self.store.as_ref(),
            self.config.stale_threshold,
            self.handler.as_ref(),
        )
        .await
        {
            Ok(result) => Some(result),
            Err(error) => {
                error!(%error, "Failed to reclaim stale tasks");
                self.handler.reap_failed(&error);
                None
            }
        }
    }

    /// Reap on the configured interval, forever.
    pub async fn run(&self) {
        info!(
            stale_threshold_secs = self.config.stale_threshold.as_secs(),
            reclaim_interval_secs = self.config.interval.as_secs(),
            "Started stale task reclamation background task"
        );
        let mut interval = tokio::time::interval(self.config.interval);
        loop {
            interval.tick().await;
            self.reap_once().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::{
        EventLog, InMemoryWorkflowEventStore, TaskDefinition, TaskQueue, TaskStatus, WorkerInfo,
        WorkerRegistry,
    };
    use crate::reliability::RetryPolicy;
    use crate::workflow::ActivityOptions;
    use parking_lot::Mutex;
    use serde_json::json;
    use uuid::Uuid;

    #[derive(Default)]
    struct Recording {
        failed: Mutex<Vec<(Uuid, String)>>,
        sealed: Mutex<Vec<Uuid>>,
        errors: Mutex<usize>,
    }

    #[async_trait]
    impl ReapHandler for Recording {
        async fn workflow_failed(&self, dead: &DeadTaskInfo, error: &str) {
            self.failed.lock().push((dead.task_id, error.to_string()));
        }
        async fn task_sealed(&self, sealed: &SealedTaskInfo) {
            self.sealed.lock().push(sealed.task_id);
        }
        fn reap_failed(&self, _: &StoreError) {
            *self.errors.lock() += 1;
        }
    }

    async fn store_with_worker() -> Arc<InMemoryWorkflowEventStore> {
        let store = Arc::new(InMemoryWorkflowEventStore::new());
        store
            .register_worker(WorkerInfo::new("w1", ["step"]))
            .await
            .unwrap();
        store
    }

    async fn running_workflow(store: &InMemoryWorkflowEventStore, workflow_id: Uuid) {
        store
            .create_workflow(workflow_id, "wf", json!({}), None)
            .await
            .unwrap();
        store
            .update_workflow_status(workflow_id, WorkflowStatus::Running, None, None)
            .await
            .unwrap();
    }

    /// Enqueue and claim a one-attempt task, so the next reap finds it dead.
    async fn claimed_single_attempt_task(
        store: &InMemoryWorkflowEventStore,
        workflow_id: Option<Uuid>,
    ) -> Uuid {
        let task_id = store
            .enqueue_task(TaskDefinition {
                workflow_id,
                activity_id: "step-1".into(),
                activity_type: "step".into(),
                input: json!({}),
                options: ActivityOptions {
                    retry_policy: RetryPolicy::no_retry(),
                    ..ActivityOptions::default()
                },
            })
            .await
            .unwrap();
        let claimed = store
            .claim_task("w1", &["step".to_string()], 1)
            .await
            .unwrap();
        assert_eq!(claimed.len(), 1);
        task_id
    }

    #[tokio::test]
    async fn dead_task_fails_its_workflow_and_notifies_once() {
        let store = store_with_worker().await;
        let workflow_id = Uuid::now_v7();
        running_workflow(&store, workflow_id).await;
        let task_id = claimed_single_attempt_task(&store, Some(workflow_id)).await;
        tokio::time::sleep(Duration::from_millis(5)).await;

        let handler = Recording::default();
        let result = reap_stale_tasks(store.as_ref(), Duration::ZERO, &handler)
            .await
            .unwrap();

        assert_eq!(result.dead_tasks.len(), 1);
        assert_eq!(
            store.get_task(task_id).await.unwrap().status,
            TaskStatus::Dead
        );
        assert_eq!(
            store.get_workflow_status(workflow_id).await.unwrap(),
            WorkflowStatus::Failed
        );
        let failed = handler.failed.lock().clone();
        assert_eq!(
            failed,
            vec![(task_id, UNRESPONSIVE_WORKER_ERROR.to_string())]
        );

        // A second pass finds nothing to settle and does not notify again.
        let again = reap_stale_tasks(store.as_ref(), Duration::ZERO, &handler)
            .await
            .unwrap();
        assert!(again.dead_tasks.is_empty());
        assert_eq!(handler.failed.lock().len(), 1);
    }

    #[tokio::test]
    async fn dead_task_of_terminal_workflow_does_not_notify() {
        let store = store_with_worker().await;
        let workflow_id = Uuid::now_v7();
        running_workflow(&store, workflow_id).await;
        claimed_single_attempt_task(&store, Some(workflow_id)).await;
        store
            .update_workflow_status(workflow_id, WorkflowStatus::Completed, None, None)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(5)).await;

        let handler = Recording::default();
        let result = reap_stale_tasks(store.as_ref(), Duration::ZERO, &handler)
            .await
            .unwrap();

        assert_eq!(result.dead_tasks.len(), 1);
        assert!(
            handler.failed.lock().is_empty(),
            "the workflow already ended"
        );
        assert_eq!(
            store.get_workflow_status(workflow_id).await.unwrap(),
            WorkflowStatus::Completed
        );
    }

    #[tokio::test]
    async fn dead_standalone_task_has_no_workflow_to_fail() {
        let store = store_with_worker().await;
        let task_id = claimed_single_attempt_task(&store, None).await;
        tokio::time::sleep(Duration::from_millis(5)).await;

        let handler = Recording::default();
        let result = reap_stale_tasks(store.as_ref(), Duration::ZERO, &handler)
            .await
            .unwrap();

        assert_eq!(result.dead_tasks[0].task_id, task_id);
        assert!(handler.failed.lock().is_empty());
    }

    #[tokio::test]
    async fn fresh_task_is_left_alone() {
        let store = store_with_worker().await;
        let task_id = claimed_single_attempt_task(&store, None).await;

        let handler = Recording::default();
        let result = reap_stale_tasks(store.as_ref(), Duration::from_secs(60), &handler)
            .await
            .unwrap();

        assert!(result.reclaimed_ids.is_empty() && result.dead_tasks.is_empty());
        assert_eq!(
            store.get_task(task_id).await.unwrap().status,
            TaskStatus::Claimed
        );
    }

    #[tokio::test]
    async fn sealed_task_fails_its_workflow_and_notifies() {
        let store = store_with_worker().await;
        let workflow_id = Uuid::now_v7();
        running_workflow(&store, workflow_id).await;
        let task_id = store
            .enqueue_task(TaskDefinition {
                workflow_id: Some(workflow_id),
                activity_id: "step-1".into(),
                activity_type: "step".into(),
                input: json!({}),
                options: ActivityOptions {
                    retry_policy: RetryPolicy::default().with_max_attempts(100),
                    ..ActivityOptions::default()
                },
            })
            .await
            .unwrap();

        // Crash-loop without recording progress until the guard seals it.
        let handler = Recording::default();
        let threshold = crate::persistence::no_progress_seal_threshold_from_env();
        for _ in 0..threshold {
            store
                .claim_task("w1", &["step".to_string()], 1)
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(2)).await;
            reap_stale_tasks(store.as_ref(), Duration::ZERO, &handler)
                .await
                .unwrap();
        }

        assert_eq!(handler.sealed.lock().clone(), vec![task_id]);
        assert_eq!(
            store.get_workflow_status(workflow_id).await.unwrap(),
            WorkflowStatus::Failed
        );
    }

    #[tokio::test]
    async fn reaper_runs_passes_through_the_handler() {
        let store = store_with_worker().await;
        let workflow_id = Uuid::now_v7();
        running_workflow(&store, workflow_id).await;
        claimed_single_attempt_task(&store, Some(workflow_id)).await;
        tokio::time::sleep(Duration::from_millis(5)).await;

        let handler = Arc::new(Recording::default());
        let reaper = StaleTaskReaper::new(
            store.clone(),
            ReaperConfig {
                stale_threshold: Duration::ZERO,
                interval: Duration::from_millis(10),
                ..ReaperConfig::default()
            },
            handler.clone(),
        );
        let running = tokio::spawn(async move { reaper.run().await });
        tokio::time::timeout(Duration::from_secs(5), async {
            while handler.failed.lock().is_empty() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the reaper settles the dead task");
        running.abort();
        assert_eq!(*handler.errors.lock(), 0);
    }
}
