// Task Worker Implementation
//
// Decision: Single worker implementation generic over WorkerAdapters
// Decision: Works with both gRPC (external) and Direct (in-process) adapters
//
// TaskWorker runs two loops over one set of execution slots:
// - the session actor loop (`actor_runtime`): leases runnable sessions and
//   runs each one's whole turn. Turns run only here (actor-based design,
//   step 6c); no turn step is a queued task any more.
// - the durable task queue loop: claims the activities that are not turns
//   (leased-resource cleanup, the session task reaper, scheduled channel and
//   agent-trigger invocations) and runs them through `TaskDriver`.
//
// Decision: the worker owns registration, heartbeats, operator drain and
// concurrency for both loops: they share `in_flight` (the advertised load),
// the model-wait slot release, and the shutdown grace.

use crate::durable::WorkerInfo;
use anyhow::Result;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::{Notify, watch};
use tokio::task::JoinSet;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::actor_runtime::{
    ActorRuntime, ActorRuntimeConfig, ActorSlots, DetachedCalls, DetachedCallsConfig,
    WorkerActorHosts,
};
use crate::engine::model_wait::ModelWaitSlots;
use crate::model_wait_slots::{DEFAULT_MAX_CONCURRENT_MODEL_WAITS, WorkerModelWaitSlots};
use crate::task_driver::TaskDriver;
use crate::task_store::TaskStore;
use crate::task_wakeup::spawn_wakeup_listener;
use crate::worker_adapters::WorkerAdapters;

// =============================================================================
// Configuration
// =============================================================================

/// Default execution concurrency for workers.
///
/// Historically 1000, which let a single worker advertise too much capacity for
/// small Postgres instances. 1000-way concurrency is now opt-in via
/// `MAX_CONCURRENT_TASKS`; the default mirrors the server's default DB pool.
pub const DEFAULT_MAX_CONCURRENT_TASKS: usize = 50;

/// Upper bound on a single claim request, independent of execution concurrency.
pub const DEFAULT_CLAIM_BATCH_SIZE: usize = 50;

/// Base fallback poll interval when push notifications are unavailable.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Maximum fallback poll interval while the queue is empty.
pub const DEFAULT_POLL_BACKOFF_MAX: Duration = Duration::from_secs(5);

/// How many tasks to claim in one request, given currently-free execution slots.
pub(crate) fn claim_limit(available_slots: usize, claim_batch_size: usize) -> usize {
    available_slots.min(claim_batch_size)
}

/// Next fallback poll interval.
pub(crate) fn next_poll_backoff(
    current: Duration,
    base: Duration,
    max: Duration,
    found_work: bool,
) -> Duration {
    if found_work {
        base.min(max)
    } else {
        current.checked_mul(2).unwrap_or(max).min(max)
    }
}

/// Configuration for the task worker
#[derive(Debug, Clone)]
pub struct TaskWorkerConfig {
    /// Worker ID (unique identifier for this worker instance)
    pub worker_id: String,
    /// Activity types this worker handles
    pub activity_types: Vec<String>,
    /// Maximum concurrent tasks (execution concurrency / advertised capacity)
    pub max_concurrent_tasks: usize,
    /// Maximum tasks claimed in a single request, regardless of available slots.
    pub claim_batch_size: usize,
    /// How many reason steps may hand their slot back while they wait on the
    /// model (`MAX_CONCURRENT_MODEL_WAITS`; 0 turns it off). See
    /// `model_wait_slots`.
    pub max_concurrent_model_waits: usize,
    /// Base poll interval when no tasks available
    pub poll_interval: Duration,
    /// Cap for the exponential poll backoff while the queue is empty.
    pub poll_backoff_max: Duration,
    /// Heartbeat interval for worker registration
    pub heartbeat_interval: Duration,
    /// Worker group name (optional, for routing)
    pub worker_group: Option<String>,
    /// gRPC address for standalone worker control-plane communication.
    pub grpc_address: String,
    /// Timeout for initial connection to control-plane gRPC.
    pub connect_timeout: Duration,
    /// How long a shutting-down worker lets in-flight tasks finish before it
    /// aborts them (`WORKER_SHUTDOWN_GRACE_SECS`). Keep it under the
    /// orchestrator's kill timeout; aborted tasks go back to the queue when
    /// the worker deregisters.
    pub shutdown_grace: Duration,
    /// How long a server-side tool call runs with its turn before it detaches
    /// and the turn parks (`WORKER_TOOL_DETACH_AFTER_SECS`; zero keeps every
    /// call with its turn). See `actor_runtime::detached_calls`.
    pub tool_detach_after: Duration,
    /// Most detached calls this worker holds (`MAX_DETACHED_TOOL_CALLS`).
    pub max_detached_tool_calls: usize,
}

/// Default for [`TaskWorkerConfig::shutdown_grace`]: inside Kubernetes'
/// default 30 s termination grace period.
pub const DEFAULT_SHUTDOWN_GRACE: Duration = Duration::from_secs(25);

impl Default for TaskWorkerConfig {
    fn default() -> Self {
        Self {
            worker_id: format!("worker-{}", Uuid::now_v7()),
            // Turns are not queued activities: the actor loop runs them.
            activity_types: crate::activities::all(),
            max_concurrent_tasks: DEFAULT_MAX_CONCURRENT_TASKS,
            claim_batch_size: DEFAULT_CLAIM_BATCH_SIZE,
            max_concurrent_model_waits: DEFAULT_MAX_CONCURRENT_MODEL_WAITS,
            poll_interval: DEFAULT_POLL_INTERVAL,
            poll_backoff_max: DEFAULT_POLL_BACKOFF_MAX,
            heartbeat_interval: Duration::from_secs(10),
            worker_group: None,
            grpc_address: "127.0.0.1:9001".to_string(),
            connect_timeout: Duration::from_secs(30),
            shutdown_grace: DEFAULT_SHUTDOWN_GRACE,
            tool_detach_after: crate::actor_runtime::detached_calls::DEFAULT_TOOL_DETACH_AFTER,
            max_detached_tool_calls:
                crate::actor_runtime::detached_calls::DEFAULT_MAX_DETACHED_TOOL_CALLS,
        }
    }
}

impl TaskWorkerConfig {
    /// Create dev mode configuration (faster pickup, lower concurrency).
    /// Keeps a short backoff cap so an idle dev queue still picks up new work
    /// quickly.
    pub fn dev_mode() -> Self {
        let max_concurrent_tasks = 10;
        Self {
            worker_id: format!("dev-worker-{}", Uuid::now_v7()),
            worker_group: Some("dev".to_string()),
            max_concurrent_tasks,
            // Keep the claim batch within execution concurrency.
            claim_batch_size: DEFAULT_CLAIM_BATCH_SIZE.min(max_concurrent_tasks),
            poll_interval: Duration::from_millis(10),
            poll_backoff_max: Duration::from_millis(250),
            ..Default::default()
        }
    }

    /// Create a high-concurrency configuration. This is the explicit opt-in for
    /// 1000-way execution; the claim batch stays bounded by `claim_batch_size`.
    pub fn production() -> Self {
        Self {
            max_concurrent_tasks: 1000,
            ..Default::default()
        }
    }

    /// Create configuration from environment variables.
    ///
    /// Execution concurrency (`MAX_CONCURRENT_TASKS`), claim batch size
    /// (`CLAIM_BATCH_SIZE`), and the poll interval/backoff cap
    /// (`WORKER_POLL_INTERVAL_MS` / `WORKER_POLL_BACKOFF_MAX_MS`) are independent.
    /// The claim batch is clamped to the execution concurrency.
    pub fn from_env() -> Self {
        use crate::core::config::{env_duration_ms, env_duration_secs, env_or, env_string_any};

        let worker_id =
            std::env::var("WORKER_ID").unwrap_or_else(|_| format!("worker-{}", Uuid::now_v7()));
        let defaults = Self::default();
        let max_concurrent_tasks = env_or("MAX_CONCURRENT_TASKS", defaults.max_concurrent_tasks);
        let claim_batch_size =
            env_or("CLAIM_BATCH_SIZE", defaults.claim_batch_size).min(max_concurrent_tasks);

        Self {
            worker_id,
            max_concurrent_tasks,
            claim_batch_size,
            max_concurrent_model_waits: env_or(
                "MAX_CONCURRENT_MODEL_WAITS",
                defaults.max_concurrent_model_waits,
            ),
            poll_interval: env_duration_ms("WORKER_POLL_INTERVAL_MS", defaults.poll_interval),
            poll_backoff_max: env_duration_ms(
                "WORKER_POLL_BACKOFF_MAX_MS",
                defaults.poll_backoff_max,
            ),
            worker_group: std::env::var("WORKER_GROUP").ok(),
            grpc_address: env_string_any(
                &["SERVER_GRPC_ADDRESS", "WORKER_GRPC_ADDRESS"],
                &defaults.grpc_address,
            ),
            connect_timeout: env_duration_secs(
                "WORKER_GRPC_CONNECT_TIMEOUT",
                defaults.connect_timeout,
            ),
            shutdown_grace: env_duration_secs(
                "WORKER_SHUTDOWN_GRACE_SECS",
                defaults.shutdown_grace,
            ),
            tool_detach_after: env_duration_secs(
                "WORKER_TOOL_DETACH_AFTER_SECS",
                defaults.tool_detach_after,
            ),
            max_detached_tool_calls: env_or(
                "MAX_DETACHED_TOOL_CALLS",
                defaults.max_detached_tool_calls,
            ),
            ..defaults
        }
    }
}

// =============================================================================
// Unified Worker
// =============================================================================

/// Unified worker that executes tasks from the durable task queue
///
/// This worker is generic over:
/// - `S`: TaskStore implementation (direct store or gRPC store)
/// - `A`: WorkerAdapters implementation (Direct or gRPC)
pub struct TaskWorker<S, A>
where
    S: TaskStore,
    A: WorkerAdapters,
{
    config: TaskWorkerConfig,
    store: Arc<S>,
    /// Runs each claimed task (see `task_driver`).
    driver: TaskDriver<S, A>,
    dreaming_host: crate::runtime_host::WorkerRuntimeHost<A>,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
    in_flight: Arc<AtomicUsize>,
    /// Lets reason steps hand their slot back while waiting on the model.
    model_waits: Arc<dyn ModelWaitSlots>,
    /// Cuts the poll backoff short when new work may be claimable.
    wake: Arc<Notify>,
    /// The registry reported this worker draining (operator drain): claim
    /// nothing until a heartbeat says it is resumed.
    draining: Arc<AtomicBool>,
    /// Leases session actors and runs their turns.
    actors: ActorRuntime<WorkerActorHosts<A>>,
}

impl<S, A> TaskWorker<S, A>
where
    S: TaskStore,
    A: WorkerAdapters,
{
    /// Create a new unified worker
    pub fn new(config: TaskWorkerConfig, store: Arc<S>, adapters: A) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        info!(
            worker_id = %config.worker_id,
            max_concurrent_tasks = config.max_concurrent_tasks,
            max_concurrent_model_waits = config.max_concurrent_model_waits,
            claim_batch_size = config.claim_batch_size,
            poll_interval_ms = config.poll_interval.as_millis(),
            poll_backoff_max_ms = config.poll_backoff_max.as_millis(),
            heartbeat_interval_ms = config.heartbeat_interval.as_millis(),
            "Initialized unified worker"
        );

        let mut actors = ActorRuntime::new(
            WorkerActorHosts::new(adapters.clone()),
            adapters.actor_lease_store(),
            ActorRuntimeConfig::new(config.worker_id.clone()),
        );
        if !config.tool_detach_after.is_zero() && config.max_detached_tool_calls > 0 {
            let emitters = adapters.clone();
            actors = actors.with_detached_calls(Arc::new(DetachedCalls::new(
                adapters.actor_lease_store(),
                config.worker_id.clone(),
                DetachedCallsConfig {
                    detach_after: config.tool_detach_after,
                    max_calls: config.max_detached_tool_calls,
                    ttl: crate::actor_runtime::detached_calls::DETACHED_CALL_TTL,
                },
                Arc::new(move |fence| {
                    Arc::new(
                        crate::worker_adapters::SessionAdapter::new(emitters.clone())
                            .with_actor_fence(Some(fence)),
                    ) as Arc<dyn crate::core::event_emitter::EventEmitter>
                }),
            )));
        }
        let driver = TaskDriver::new(
            store.clone(),
            adapters.clone(),
            config.worker_id.clone(),
            config.heartbeat_interval,
        );

        let in_flight = Arc::new(AtomicUsize::new(0));
        let wake = Arc::new(Notify::new());
        let model_waits = Arc::new(WorkerModelWaitSlots::new(
            in_flight.clone(),
            config.max_concurrent_model_waits,
            wake.clone(),
        ));
        Self {
            config,
            store,
            driver,
            dreaming_host: crate::runtime_host::WorkerRuntimeHost::new(adapters),
            shutdown_tx,
            shutdown_rx,
            in_flight,
            model_waits,
            wake,
            draining: Arc::new(AtomicBool::new(false)),
            actors,
        }
    }

    /// Wake the session actor loop from `wakeups`, for a worker whose store
    /// has no push channel but shares a process with the control plane
    /// (DEV_MODE). Remote workers hear the same wake-ups through
    /// `subscribe_task_wakeups`.
    pub fn with_actor_wakeups(mut self, wakeups: &crate::actor_runtime::ActorWakeups) -> Self {
        self.actors = self.actors.with_wakeups(wakeups);
        self
    }

    /// Run the worker (blocking until shutdown)
    pub async fn run(&mut self) -> Result<()> {
        let maintenance_host = self.dreaming_host.clone();
        let maintenance = tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));
            loop {
                interval.tick().await;
                if let Err(error) = crate::host::dreaming::run_due(maintenance_host.clone()).await {
                    tracing::warn!(%error,"dreaming sweep failed");
                }
            }
        });
        struct StopMaintenance(tokio::task::JoinHandle<()>);
        impl Drop for StopMaintenance {
            fn drop(&mut self) {
                self.0.abort();
            }
        }
        let _maintenance = StopMaintenance(maintenance);

        info!(
            worker_id = %self.config.worker_id,
            "Starting unified worker"
        );

        // Register worker
        let worker_info = WorkerInfo {
            id: self.config.worker_id.clone(),
            worker_group: self.config.worker_group.clone(),
            activity_types: self.config.activity_types.clone(),
            max_concurrency: self.config.max_concurrent_tasks as u32,
            current_load: 0,
            status: "active".to_string(),
            accepting_tasks: true,
            backpressure_reason: None,
            started_at: chrono::Utc::now(),
            last_heartbeat_at: chrono::Utc::now(),
            hostname: None,
            version: None,
            metadata: None,
            tasks_completed: 0,
            tasks_failed: 0,
            avg_task_duration_ms: None,
        };
        if let Err(e) = self.store.register_worker(worker_info).await {
            warn!(error = %e, "Failed to register worker (will continue anyway)");
        } else {
            info!(worker_id = %self.config.worker_id, "Worker registered");
        }

        // Heartbeat task. It outlives the poll loop: a shutting-down worker
        // keeps heartbeating (not accepting) while in-flight tasks finish, so
        // the registry does not reap it mid-drain. The reply says whether an
        // operator drained the worker.
        let (heartbeat_stop_tx, mut heartbeat_stop_rx) = watch::channel(false);
        let shutting_down = Arc::new(AtomicBool::new(false));
        let heartbeat_handle = tokio::spawn(heartbeat_loop(
            self.store.clone(),
            self.config.worker_id.clone(),
            self.config.heartbeat_interval,
            self.in_flight.clone(),
            self.draining.clone(),
            shutting_down.clone(),
            self.wake.clone(),
            async move {
                let _ = heartbeat_stop_rx.changed().await;
            },
        ));

        let wakeup_handle = spawn_wakeup_listener(
            self.store.clone(),
            self.config.worker_id.clone(),
            self.config.activity_types.clone(),
            self.wake.clone(),
            self.shutdown_rx.clone(),
        );

        let actor_handles = self.spawn_actor_loop();

        // Adaptive poll backoff: doubles from `poll_interval` up to
        // `poll_backoff_max` while the queue is empty, so an idle worker stops
        // issuing tight-loop `claim_task` requests. Reset to base on any work.
        let mut poll_backoff = self.config.poll_interval.min(self.config.poll_backoff_max);
        let mut task_handles = JoinSet::new();

        // Main poll loop
        loop {
            if *self.shutdown_rx.borrow() {
                info!("Shutdown signal received, stopping worker");
                break;
            }

            Self::drain_finished_tasks(&mut task_handles);

            match self.poll_and_execute(&mut task_handles).await {
                Ok(executed) => {
                    let mut woken = false;
                    if executed == 0 {
                        tokio::select! {
                            _ = tokio::time::sleep(poll_backoff) => {}
                            _ = self.wake.notified() => woken = true,
                            _ = self.shutdown_rx.changed() => {
                                info!("Shutdown during poll wait");
                                break;
                            }
                        }
                    }
                    poll_backoff = next_poll_backoff(
                        poll_backoff,
                        self.config.poll_interval,
                        self.config.poll_backoff_max,
                        executed > 0 || woken,
                    );
                }
                Err(e) => {
                    error!("Error polling tasks: {}", e);
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }

        let _ = self.shutdown_tx.send(true);
        shutting_down.store(true, Ordering::SeqCst);
        self.finish_in_flight(&mut task_handles).await;
        for handle in actor_handles {
            let _ = handle.await;
        }
        let _ = heartbeat_stop_tx.send(true);
        let _ = heartbeat_handle.await;
        let _ = wakeup_handle.await;

        // Deregister on shutdown; the registry returns tasks still claimed by
        // this worker (aborted after the grace period) to the queue.
        if let Err(e) = self.store.deregister_worker(&self.config.worker_id).await {
            warn!(error = %e, "Failed to deregister worker");
        }

        info!("Unified worker stopped");
        Ok(())
    }

    /// Start the session actor loop and its wake-ups: `session_actor` wakes
    /// the loop, `session_actor_cancel` makes every running lease renew now
    /// (which is how its holder learns of the cancel). Each listener
    /// subscribes under its own id: the control plane keeps one subscription
    /// per id.
    fn spawn_actor_loop(&self) -> Vec<tokio::task::JoinHandle<()>> {
        let slots = ActorSlots {
            in_flight: self.in_flight.clone(),
            max_concurrent: self.config.max_concurrent_tasks,
            claim_batch: self.config.claim_batch_size,
            model_waits: self.model_waits.clone(),
            draining: self.draining.clone(),
            poll_interval: self.config.poll_interval,
            // An expired lease sends no wake-up; see `ACTOR_POLL_BACKOFF_MAX`.
            poll_backoff_max: self
                .config
                .poll_backoff_max
                .min(crate::actor_runtime::ACTOR_POLL_BACKOFF_MAX),
            shutdown_grace: self.config.shutdown_grace,
        };
        let actors = self.actors.clone();
        let shutdown = self.shutdown_rx.clone();
        let run = tokio::spawn(async move { actors.run(slots, shutdown).await });
        let wakeups = spawn_wakeup_listener(
            self.store.clone(),
            format!("{}:actors", self.config.worker_id),
            vec![crate::actor_runtime::SESSION_ACTOR_NOTIFICATION.to_string()],
            self.actors.wake(),
            self.shutdown_rx.clone(),
        );
        let cancels = Arc::new(Notify::new());
        let cancel_wakeups = spawn_wakeup_listener(
            self.store.clone(),
            format!("{}:actor-cancels", self.config.worker_id),
            vec![crate::actor_runtime::SESSION_ACTOR_CANCEL_NOTIFICATION.to_string()],
            cancels.clone(),
            self.shutdown_rx.clone(),
        );
        let poke = self.actors.renew_poke();
        let mut shutdown = self.shutdown_rx.clone();
        let forward = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancels.notified() => poke.notify_waiters(),
                    _ = shutdown.changed() => return,
                }
            }
        });
        vec![run, wakeups, cancel_wakeups, forward]
    }

    /// Signal the worker to shutdown
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    /// Get shutdown handle for external shutdown signaling
    pub fn shutdown_handle(&self) -> ShutdownHandle {
        ShutdownHandle {
            tx: self.shutdown_tx.clone(),
        }
    }

    /// Graceful stop: mark this worker draining so the queue routes new tasks
    /// elsewhere, give in-flight tasks `shutdown_grace` to
    /// finish, then abort what is left.
    async fn finish_in_flight(&self, task_handles: &mut JoinSet<()>) {
        if let Err(e) = self.store.drain_worker(&self.config.worker_id).await {
            warn!(error = %e, "Failed to mark worker draining for shutdown");
        }
        let in_flight = task_handles.len();
        if in_flight > 0 {
            info!(
                in_flight,
                grace_secs = self.config.shutdown_grace.as_secs_f64(),
                "Waiting for in-flight tasks before stopping"
            );
        }
        let finished = tokio::time::timeout(self.config.shutdown_grace, async {
            while let Some(result) = task_handles.join_next().await {
                if let Err(error) = result
                    && !error.is_cancelled()
                {
                    warn!(error = %error, "Worker task join failed during shutdown");
                }
            }
        })
        .await;
        if finished.is_err() {
            warn!(
                aborted = task_handles.len(),
                "Shutdown grace period elapsed, aborting in-flight tasks"
            );
            task_handles.abort_all();
            while task_handles.join_next().await.is_some() {}
        }
    }

    fn drain_finished_tasks(task_handles: &mut JoinSet<()>) {
        while let Some(result) = task_handles.try_join_next() {
            if let Err(error) = result
                && !error.is_cancelled()
            {
                warn!(error = %error, "Worker task join failed");
            }
        }
    }

    /// Poll for tasks and execute them
    async fn poll_and_execute(&self, task_handles: &mut JoinSet<()>) -> Result<usize> {
        if self.draining.load(Ordering::SeqCst) {
            return Ok(0);
        }
        let current_in_flight = self.in_flight.load(Ordering::SeqCst);
        let available_slots = self
            .config
            .max_concurrent_tasks
            .saturating_sub(current_in_flight);

        if available_slots == 0 {
            debug!(
                current_in_flight = current_in_flight,
                max = self.config.max_concurrent_tasks,
                "No available slots, skipping claim"
            );
            return Ok(0);
        }

        // Bound the claim by claim_batch_size so a single claim_task stays cheap
        // even when execution concurrency is high.
        let to_claim = claim_limit(available_slots, self.config.claim_batch_size);
        let tasks = self
            .store
            .claim_task(
                &self.config.worker_id,
                &self.config.activity_types,
                to_claim,
            )
            .await
            .map_err(|e| anyhow::anyhow!("Failed to claim tasks: {}", e))?;

        if tasks.is_empty() {
            return Ok(0);
        }

        debug!(
            worker_id = %self.config.worker_id,
            task_count = tasks.len(),
            "Claimed tasks"
        );

        let task_count = tasks.len();

        // Execute tasks concurrently
        for task in tasks {
            let driver = self.driver.clone();
            let in_flight_guard = InFlightTaskGuard::increment(self.in_flight.clone());
            let wake = self.wake.clone();

            task_handles.spawn(async move {
                let _in_flight_guard = in_flight_guard;
                let result = driver.execute_task(&task).await;
                // A freed slot may take queued work now instead of after the
                // poll backoff.
                wake.notify_one();

                if let Err(e) = result {
                    error!(
                        task_id = %task.id,
                        workflow_id = ?task.workflow_id,
                        activity_type = %task.activity_type,
                        attempt = task.attempt,
                        max_attempts = task.max_attempts,
                        error = %crate::task_driver::failure_message(&task, &e),
                        "Task execution failed"
                    );
                }
            });
        }

        Ok(task_count)
    }
}

/// Heartbeats until `stop` resolves, recording whether the registry reports
/// the worker draining and waking the poll loop when a drain is lifted.
#[allow(clippy::too_many_arguments)]
async fn heartbeat_loop<S: TaskStore>(
    store: Arc<S>,
    worker_id: String,
    interval: Duration,
    in_flight: Arc<AtomicUsize>,
    draining: Arc<AtomicBool>,
    shutting_down: Arc<AtomicBool>,
    wake: Arc<Notify>,
    stop: impl std::future::Future<Output = ()>,
) {
    tokio::pin!(stop);
    loop {
        tokio::select! {
            _ = tokio::time::sleep(interval) => {}
            _ = &mut stop => break,
        }
        let current_load = in_flight.load(Ordering::SeqCst);
        let accepting = !shutting_down.load(Ordering::SeqCst);
        match store
            .worker_heartbeat(&worker_id, current_load, accepting)
            .await
        {
            Ok(reply) => {
                let was = draining.swap(reply.draining, Ordering::SeqCst);
                if reply.draining && !was {
                    info!(worker_id = %worker_id, "Worker drained, no longer claiming tasks");
                } else if was && !reply.draining {
                    info!(worker_id = %worker_id, "Worker resumed, claiming tasks again");
                    wake.notify_one();
                }
            }
            Err(e) => warn!(error = %e, "Failed to send heartbeat"),
        }
    }
}

/// Handle for triggering worker shutdown
#[derive(Clone)]
pub struct ShutdownHandle {
    tx: watch::Sender<bool>,
}

impl ShutdownHandle {
    /// Trigger shutdown of the worker
    pub fn shutdown(&self) {
        let _ = self.tx.send(true);
    }
}

struct InFlightTaskGuard {
    in_flight: Arc<AtomicUsize>,
}

impl InFlightTaskGuard {
    fn increment(in_flight: Arc<AtomicUsize>) -> Self {
        in_flight.fetch_add(1, Ordering::SeqCst);
        Self { in_flight }
    }
}

impl Drop for InFlightTaskGuard {
    fn drop(&mut self) {
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use crate::unified_worker_test_adapters::NoopAdapters;

    use super::*;
    use crate::durable::{ClaimedTask, HeartbeatResponse, StoreError, TaskFailureOutcome};

    #[test]
    fn test_claim_limit_bounds_to_batch() {
        assert_eq!(
            claim_limit(1000, DEFAULT_CLAIM_BATCH_SIZE),
            DEFAULT_CLAIM_BATCH_SIZE
        );
        assert_eq!(claim_limit(1000, 50), 50);
        assert_eq!(claim_limit(10, 50), 10);
        assert_eq!(claim_limit(0, 50), 0);
    }

    #[test]
    fn test_next_poll_backoff() {
        let base = Duration::from_millis(100);
        let max = Duration::from_secs(5);
        let mut backoff = base;

        backoff = next_poll_backoff(backoff, base, max, false);
        assert_eq!(backoff, Duration::from_millis(200));
        backoff = next_poll_backoff(backoff, base, max, false);
        assert_eq!(backoff, Duration::from_millis(400));

        for _ in 0..10 {
            backoff = next_poll_backoff(backoff, base, max, false);
        }
        assert_eq!(backoff, max);

        assert_eq!(next_poll_backoff(backoff, base, max, true), base);
        assert_eq!(next_poll_backoff(base, max * 2, max, true), max);
        assert_eq!(next_poll_backoff(Duration::MAX, base, max, false), max);
    }

    /// Hands out two tasks on the first claim; each fails after `fail_delay`.
    #[derive(Clone, Default)]
    struct ConcurrentStore {
        claimed: Arc<AtomicBool>,
        current: Arc<AtomicUsize>,
        max: Arc<AtomicUsize>,
        fail_delay: Duration,
        /// Tasks that ran to the end (not aborted).
        finished: Arc<AtomicUsize>,
        /// Registry calls in order, to check drain-before-deregister.
        calls: Arc<std::sync::Mutex<Vec<&'static str>>>,
    }

    #[async_trait::async_trait]
    impl TaskStore for ConcurrentStore {
        async fn register_worker(&self, _worker: WorkerInfo) -> Result<(), StoreError> {
            Ok(())
        }

        async fn worker_heartbeat(
            &self,
            _worker_id: &str,
            _current_load: usize,
            _accepting_tasks: bool,
        ) -> Result<crate::durable::WorkerHeartbeat, StoreError> {
            Ok(Default::default())
        }

        async fn drain_worker(&self, _worker_id: &str) -> Result<(), StoreError> {
            self.calls.lock().expect("calls lock").push("drain");
            Ok(())
        }

        async fn deregister_worker(&self, _worker_id: &str) -> Result<usize, StoreError> {
            self.calls.lock().expect("calls lock").push("deregister");
            Ok(0)
        }

        async fn claim_task(
            &self,
            _worker_id: &str,
            _activity_types: &[String],
            _max_tasks: usize,
        ) -> Result<Vec<ClaimedTask>, StoreError> {
            if self.claimed.swap(true, Ordering::SeqCst) {
                return Ok(vec![]);
            }

            Ok((0..2)
                .map(|_| ClaimedTask {
                    id: Uuid::now_v7(),
                    workflow_id: None,
                    activity_id: format!("unknown_{}", Uuid::now_v7()),
                    activity_type: "unknown".to_string(),
                    input: serde_json::json!({}),
                    attempt: 1,
                    max_attempts: 1,
                    ..Default::default()
                })
                .collect())
        }

        async fn heartbeat_task(
            &self,
            _task_id: Uuid,
            _worker_id: &str,
        ) -> Result<HeartbeatResponse, StoreError> {
            Ok(HeartbeatResponse {
                accepted: true,
                should_cancel: false,
            })
        }

        async fn record_activity_started(&self, _task: &ClaimedTask, _worker_id: &str) {}

        async fn complete_task(
            &self,
            _task: &ClaimedTask,
            _worker_id: &str,
            _output: serde_json::Value,
        ) -> Result<(), StoreError> {
            Ok(())
        }

        async fn fail_task(
            &self,
            _task: &ClaimedTask,
            _error: &str,
            _retryable: bool,
        ) -> Result<TaskFailureOutcome, StoreError> {
            let current = self.current.fetch_add(1, Ordering::SeqCst) + 1;
            self.max.fetch_max(current, Ordering::SeqCst);
            tokio::time::sleep(self.fail_delay).await;
            self.current.fetch_sub(1, Ordering::SeqCst);
            self.finished.fetch_add(1, Ordering::SeqCst);
            Ok(TaskFailureOutcome::MovedToDlq)
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn poll_and_execute_allows_concurrent_store_calls() {
        let store = Arc::new(ConcurrentStore {
            fail_delay: Duration::from_millis(50),
            ..Default::default()
        });
        let config = TaskWorkerConfig {
            max_concurrent_tasks: 2,
            claim_batch_size: 2,
            heartbeat_interval: Duration::from_secs(60),
            ..Default::default()
        };

        let worker = TaskWorker::new(config, store.clone(), NoopAdapters);
        let mut task_handles = JoinSet::new();

        let claimed = worker
            .poll_and_execute(&mut task_handles)
            .await
            .expect("poll succeeds");

        assert_eq!(claimed, 2);
        while task_handles.join_next().await.is_some() {}

        assert!(
            store.max.load(Ordering::SeqCst) > 1,
            "store calls were serialized"
        );
    }

    fn shutdown_config(grace: Duration) -> TaskWorkerConfig {
        TaskWorkerConfig {
            max_concurrent_tasks: 2,
            claim_batch_size: 2,
            heartbeat_interval: Duration::from_secs(60),
            shutdown_grace: grace,
            ..Default::default()
        }
    }

    async fn wait_until(what: &str, check: impl Fn() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !check() {
            assert!(std::time::Instant::now() < deadline, "timed out: {what}");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_lets_in_flight_tasks_finish_then_deregisters() {
        let store = Arc::new(ConcurrentStore {
            fail_delay: Duration::from_millis(300),
            ..Default::default()
        });
        let mut worker = TaskWorker::new(
            shutdown_config(Duration::from_secs(10)),
            store.clone(),
            NoopAdapters,
        );
        let shutdown = worker.shutdown_handle();
        let run = tokio::spawn(async move { worker.run().await });

        wait_until("both tasks running", || {
            store.current.load(Ordering::SeqCst) == 2
        })
        .await;
        shutdown.shutdown();
        run.await.expect("join").expect("worker run");

        assert_eq!(
            store.finished.load(Ordering::SeqCst),
            2,
            "in-flight tasks finish before the worker stops"
        );
        assert_eq!(
            *store.calls.lock().expect("calls lock"),
            vec!["drain", "deregister"]
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_aborts_tasks_still_running_after_the_grace_period() {
        let store = Arc::new(ConcurrentStore {
            fail_delay: Duration::from_secs(60),
            ..Default::default()
        });
        let mut worker = TaskWorker::new(
            shutdown_config(Duration::from_millis(100)),
            store.clone(),
            NoopAdapters,
        );
        let shutdown = worker.shutdown_handle();
        let run = tokio::spawn(async move { worker.run().await });

        wait_until("both tasks running", || {
            store.current.load(Ordering::SeqCst) == 2
        })
        .await;
        shutdown.shutdown();
        tokio::time::timeout(Duration::from_secs(5), run)
            .await
            .expect("worker stops after the grace period")
            .expect("join")
            .expect("worker run");

        assert_eq!(store.finished.load(Ordering::SeqCst), 0);
        assert_eq!(
            *store.calls.lock().expect("calls lock"),
            vec!["drain", "deregister"]
        );
    }

    #[tokio::test]
    async fn drained_worker_claims_nothing() {
        let store = Arc::new(ConcurrentStore::default());
        let worker = TaskWorker::new(shutdown_config(Duration::ZERO), store.clone(), NoopAdapters);
        worker.draining.store(true, Ordering::SeqCst);
        let mut task_handles = JoinSet::new();

        let claimed = worker
            .poll_and_execute(&mut task_handles)
            .await
            .expect("poll succeeds");

        assert_eq!(claimed, 0);
        assert!(!store.claimed.load(Ordering::SeqCst), "no claim was sent");
    }
}
