// Cluster-once maintenance jobs: blob GC, event retention, and the Memory and
// Knowledge index source syncs.
//
// Decision: these used to be a `tokio::interval` on every replica with no
// leader election, so N replicas ran each sweep N times per interval. Each is
// now a durable schedule (`@every <interval>s`, see
// `everruns_durable::Cadence::Every`) that the durable scheduler fires once
// per cluster, and the activity it enqueues is claimed by exactly one
// replica's job pool (`WorkerPool` over the server's own durable store).
//
// They run in the server, not on workers, because they need what only the
// control plane holds: the database pools, the object store, the GitHub
// connection resolver, the embeddings drivers and the vector store. Workers
// never claim these activity types (they are not in `TaskWorkerConfig`).
//
// Configuration is unchanged: each job reads its own env knobs and interval.
// A job switched off by configuration (or with nothing to do on this backend)
// gets its schedule disabled, so a schedule created while it was on stops
// firing. A job logs its own failure and completes its task, as the interval
// loops did: a failed sweep waits for the next trigger instead of retrying.
// A replica that dies mid-run leaves a stale claim, which the reaper hands to
// another replica.
//
// The same pool also serves one-off tasks the server enqueues itself
// (`ClusterTask`), such as a parked turn's tool-result deadline: a delayed
// standalone task one replica claims when it comes due.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use everruns_durable::{
    Cadence, ScheduleSpec, WorkerPool, WorkerPoolConfig, WorkflowEventStore, disable_schedule,
    ensure_schedule,
};
use tokio::task::JoinHandle;

/// One run of a job.
pub type JobFuture = Pin<Box<dyn Future<Output = ()> + Send>>;
type JobFn = Arc<dyn Fn() -> JobFuture + Send + Sync>;
type TaskFn = Arc<dyn Fn(serde_json::Value) -> JobFuture + Send + Sync>;

/// How many one-off tasks the pool runs at once, beyond one slot per job.
const TASK_CONCURRENCY: usize = 8;

/// A one-off task the server enqueues itself, run with the task's input.
pub struct ClusterTask {
    activity: &'static str,
    run: TaskFn,
}

impl ClusterTask {
    pub fn new<F>(activity: &'static str, run: F) -> Self
    where
        F: Fn(serde_json::Value) -> JobFuture + Send + Sync + 'static,
    {
        Self {
            activity,
            run: Arc::new(run),
        }
    }
}

/// A periodic job that runs once per cluster per period.
pub struct ClusterJob {
    /// Durable schedule name.
    pub schedule: &'static str,
    /// Activity type the schedule enqueues.
    pub activity: &'static str,
    description: &'static str,
    run: Option<(Duration, JobFn)>,
}

impl ClusterJob {
    /// A job that runs `run` every `period`.
    pub fn every<F>(
        schedule: &'static str,
        activity: &'static str,
        description: &'static str,
        period: Duration,
        run: F,
    ) -> Self
    where
        F: Fn() -> JobFuture + Send + Sync + 'static,
    {
        Self {
            schedule,
            activity,
            description,
            run: Some((period, Arc::new(run))),
        }
    }

    /// A job switched off on this deployment.
    pub fn disabled(schedule: &'static str, activity: &'static str) -> Self {
        Self {
            schedule,
            activity,
            description: "",
            run: None,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.run.is_some()
    }

    /// The schedule this job is kept at, `None` when disabled.
    pub fn spec(&self) -> Option<ScheduleSpec> {
        let (period, _) = self.run.as_ref()?;
        Some(
            ScheduleSpec::activity(
                self.schedule,
                self.activity,
                Cadence::Every(*period),
                serde_json::json!({}),
            )
            .with_description(self.description),
        )
    }
}

/// Reconcile the jobs' schedules and start this replica's job pool. Returns
/// the handle that keeps the pool alive, `None` when nothing is enabled.
pub async fn start(
    store: Arc<dyn WorkflowEventStore + Send + Sync>,
    jobs: Vec<ClusterJob>,
    tasks: Vec<ClusterTask>,
) -> Option<JoinHandle<()>> {
    let mut handlers: Vec<(&'static str, TaskFn)> = Vec::new();
    for job in jobs {
        match (job.spec(), job.run) {
            (Some(spec), Some((_, run))) => {
                if let Err(error) = ensure_schedule(store.as_ref(), &spec).await {
                    // Still serve the activity: another replica may have the
                    // schedule in place.
                    tracing::error!(schedule = job.schedule, %error, "Failed to bootstrap cluster job schedule");
                }
                handlers.push((job.activity, Arc::new(move |_| run()) as TaskFn));
            }
            _ => {
                if let Err(error) = disable_schedule(store.as_ref(), job.schedule).await {
                    tracing::warn!(schedule = job.schedule, %error, "Failed to disable cluster job schedule");
                }
            }
        }
    }
    let jobs_enabled = handlers.len();
    handlers.extend(tasks.into_iter().map(|task| (task.activity, task.run)));
    if handlers.is_empty() {
        return None;
    }

    let config = WorkerPoolConfig::new(handlers.iter().map(|(a, _)| a.to_string()).collect())
        .with_worker_id(format!("server-jobs-{}", uuid::Uuid::now_v7()))
        .with_worker_group("server-jobs")
        .with_max_concurrency(jobs_enabled + TASK_CONCURRENCY)
        // The server's stale-task reaper (durable_reaper.rs) owns reclaim.
        .without_stale_reclaim();
    let pool = WorkerPool::new(store, config);
    for (activity, run) in handlers {
        pool.register_handler(activity, move |task| {
            let job = run(task.input);
            async move {
                job.await;
                Ok(serde_json::Value::Null)
            }
        });
    }
    if let Err(error) = pool.start().await {
        tracing::error!(%error, "Failed to start cluster job pool");
        return None;
    }
    tracing::info!("Cluster job pool started");
    // Dropping the pool stops its loops, so the handle owns it.
    Some(tokio::spawn(async move {
        let _pool = pool;
        std::future::pending::<()>().await
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_durable::{
        DurableScheduler, InMemoryWorkflowEventStore, Pagination, ScheduleFilter, ScheduleRow,
        Schedules,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn counting_job(runs: Arc<AtomicUsize>, period: Duration) -> ClusterJob {
        ClusterJob::every(
            "test-cluster-job",
            "test_cluster_job",
            "Counts runs",
            period,
            move || {
                let runs = runs.clone();
                Box::pin(async move {
                    runs.fetch_add(1, Ordering::SeqCst);
                })
            },
        )
    }

    async fn rows(store: &InMemoryWorkflowEventStore) -> Vec<ScheduleRow> {
        store
            .list_schedules(
                ScheduleFilter::default(),
                Pagination {
                    offset: 0,
                    limit: 100,
                },
            )
            .await
            .unwrap()
    }

    async fn wait_for(runs: &AtomicUsize, expected: usize) {
        tokio::time::timeout(Duration::from_secs(10), async {
            while runs.load(Ordering::SeqCst) < expected {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("job should run");
    }

    /// Two replicas, each with its own scheduler and job pool, share one
    /// durable store: a due trigger runs the job once, not once per replica.
    #[tokio::test]
    async fn a_trigger_runs_once_across_two_replicas() {
        let store = Arc::new(InMemoryWorkflowEventStore::new());
        let runs = Arc::new(AtomicUsize::new(0));
        let period = Duration::from_secs(1);
        let replica_a = start(
            store.clone(),
            vec![counting_job(runs.clone(), period)],
            vec![],
        )
        .await
        .expect("pool a");
        let replica_b = start(
            store.clone(),
            vec![counting_job(runs.clone(), period)],
            vec![],
        )
        .await
        .expect("pool b");

        let schedules = rows(&store).await;
        assert_eq!(schedules.len(), 1, "both replicas share one schedule");
        assert_eq!(schedules[0].cron_expression, "@every 1s");

        tokio::time::sleep(Duration::from_millis(1100)).await;
        let scheduler_a = DurableScheduler::with_defaults(store.clone(), "sched-a".into());
        let scheduler_b = DurableScheduler::with_defaults(store.clone(), "sched-b".into());
        let (a, b) = tokio::join!(
            scheduler_a.process_due_schedules(),
            scheduler_b.process_due_schedules()
        );
        a.unwrap();
        b.unwrap();

        wait_for(&runs, 1).await;
        // Give both pools time to claim anything else they could see.
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1);

        replica_a.abort();
        replica_b.abort();
    }

    #[tokio::test]
    async fn disabling_a_job_disables_its_schedule() {
        let store = Arc::new(InMemoryWorkflowEventStore::new());
        let runs = Arc::new(AtomicUsize::new(0));
        start(
            store.clone(),
            vec![counting_job(runs.clone(), Duration::from_secs(60))],
            vec![],
        )
        .await
        .expect("pool")
        .abort();
        assert!(rows(&store).await[0].enabled);

        let none = start(
            store.clone(),
            vec![ClusterJob::disabled("test-cluster-job", "test_cluster_job")],
            vec![],
        )
        .await;
        assert!(none.is_none(), "no enabled job, no pool");
        assert!(!rows(&store).await[0].enabled);
    }

    #[tokio::test]
    async fn changing_the_period_updates_the_schedule() {
        let store = Arc::new(InMemoryWorkflowEventStore::new());
        let runs = Arc::new(AtomicUsize::new(0));
        for secs in [60, 7200] {
            start(
                store.clone(),
                vec![counting_job(runs.clone(), Duration::from_secs(secs))],
                vec![],
            )
            .await
            .expect("pool")
            .abort();
        }
        let schedules = rows(&store).await;
        assert_eq!(schedules.len(), 1);
        assert_eq!(schedules[0].cron_expression, "@every 7200s");
    }

    #[test]
    fn workers_never_claim_cluster_jobs() {
        let types = everruns_worker::TaskWorkerConfig::default().activity_types;
        for activity in [
            crate::blob_gc::BLOB_GC_ACTIVITY,
            crate::event_retention::EVENT_RETENTION_ACTIVITY,
            crate::sandbox_history_retention::SANDBOX_HISTORY_RETENTION_ACTIVITY,
            crate::domains::memory::source_sync::MEMORY_SOURCE_SYNC_ACTIVITY,
            crate::domains::knowledge_indexes::source_sync::KNOWLEDGE_INDEX_SYNC_ACTIVITY,
            crate::tool_result_timeout::TOOL_RESULT_TIMEOUT_SWEEP_ACTIVITY,
            crate::tool_result_timeout::TOOL_RESULT_DEADLINE_ACTIVITY,
        ] {
            assert!(!types.iter().any(|t| t == activity), "{activity}");
        }
    }
}
