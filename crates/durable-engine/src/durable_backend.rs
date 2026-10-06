//! [`DurableBackend`]: a framework application's turns as queued,
//! checkpointed steps, driven by workers inside the application's process.
//!
//! Execution behavior:
//! - The backend owns its durable store and a pool of in-process workers.
//!   Each worker claims a turn task and hands it to a [`TurnTaskDriver`] whose
//!   host is the [`InProcessRuntime`] the session attached, so a durable turn
//!   runs the same activities on the same runtime an in-process turn does.
//!   No second adapter exists.
//! - The workflow id is the session id, as on the platform. A session's
//!   runtime is attached for as long as its [`DurableSessionBackend`] lives;
//!   a task whose session is no longer attached fails without retry, which
//!   fails its workflow.
//! - [`TurnInput::Message`] is persisted the way the in-process runtime
//!   persists it ([`InProcessRuntime::persist_accepted_input`]) before the
//!   workflow starts, and the turn keeps the requested turn id.
//!   [`TurnInput::StoredMessage`] starts the same workflow from a message
//!   the caller already recorded, writing nothing first.
//! - Steering keeps the in-process contract exactly. The session's
//!   [`TurnSteering`] stays open; the driver's steering hooks drain it at the
//!   same boundaries `InProcessRuntime::run_steerable_turn` does (before each
//!   reason, and with one atomic drain-or-close when a reason would finish),
//!   so a push is either delivered to this turn or rejected for the next one.
//!   Steered messages cross the `user_prompt_submit` boundary at the reason
//!   that delivers them, as in process. Nothing here sends `USER_MESSAGE`
//!   wake signals; those stay the platform's steering.
//! - Cancellation matches in process: `cancel` marks the workflow cancelled,
//!   then drops the session's in-flight step and waits until it has, so once
//!   it returns the turn takes no further step.
//! - Workers wait on an in-process notification when a turn starts, polling
//!   every [`WORKER_POLL_INTERVAL`] as the fallback. A worker that finished a
//!   step claims again at once, so it usually runs the step it just enqueued.
//! - Workers stop when the last [`DurableBackend`] handle drops, or on
//!   [`DurableBackend::shutdown`]. A step in flight is dropped, as dropping an
//!   in-process turn's ticket drops it.
//! - Continuations reuse the in-process runtime's own resume logic and the
//!   durable engine's own resume paths, so a continued turn takes the steps
//!   it takes in process:
//!   - A turn that parks on client-side tool calls is recorded on the runtime
//!     ([`InProcessRuntime::park_turn`]) when the driver plans the pause, so
//!     `parked_tool_calls` reports it as in process. [`TurnInput::ToolResults`]
//!     records the results under it
//!     ([`InProcessRuntime::deliver_parked_tool_results`]) and continues the
//!     workflow from its stored checkpoint through the runner's
//!     tool-resolution resume, the path the server uses.
//!   - [`TurnInput::ResumeInterrupted`] reads the cut-off turn from the
//!     session log ([`InProcessRuntime::interrupted_turn_plan`]) and starts a
//!     workflow whose first task is that act, checkpointed with the turn's
//!     state, so the driver runs it and plans on from there.
//!   - [`TurnInput::RecordedToolResults`] continues the parked turn the same
//!     way once the caller has recorded the results under it, keying the
//!     resume with the request's resolution id.
//!   - The ticket of a continued turn counts only the steps this run takes,
//!     as an in-process result does.
//! - On PostgreSQL ([`DurableBackend::postgres`]) the queue is shared with
//!   other processes, so each backend enqueues to and claims from a task
//!   queue of its own (see `backend_store`).
//! - Recovery is per session, from the session log, on every store. A turn a
//!   process exit cut off continues through `ResumeInterrupted`, as in
//!   process; nothing replays the dead process's queue. The memory store
//!   dies with its process. On PostgreSQL, the dead process's workflow for
//!   the session is still running in the shared store, its step claimed by a
//!   worker that is gone, and would block the session's next turn; so the
//!   first turn a freshly attached session starts first ends any workflow
//!   another backend left running for it ([`LEFT_BEHIND`]), failing its
//!   claimed tasks and cancelling it. Only then does the turn start, the
//!   same way it would in process.
//! - The request's [`TurnScope`](everruns_core::host::TurnScope) is ignored:
//!   the attached runtime resolves the session. Its request id is carried in
//!   the turn's checkpoint.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::{MessageId, SessionId, TurnId};
use everruns_core::ResolvedExecutionSnapshot;
use everruns_core::engine::{ReasonInput, ReasonResult, TurnPlan, reason_schedules_act};
use everruns_core::events::ToolCompletedData;
use everruns_core::host::{
    AcceptedTurnInput, InProcessRuntime, TurnBackend, TurnInput, TurnRequest, TurnSteering,
    TurnTicket, in_process_internal_org_id,
};
use everruns_durable::{
    ClaimedTask, InMemoryWorkflowEventStore, Pagination, PostgresWorkflowEventStore, TaskFilter,
    TaskQueue, TaskStatus, WorkerInfo,
};
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::backend_store::{RoutedStore, Routing};
use crate::durable_runner::{DurableRunner, DurableTaskNotifier, DurableTurnInput};
use crate::task_heartbeat::CancelSignals;
use crate::turn_backend::TurnBaseline;
use crate::turn_driver::{TurnTaskDriver, TurnTaskHost, act_task_input};
use crate::turn_store::TurnStore;

/// How long an idle worker waits for a start notification before it polls
/// the queue again.
pub const WORKER_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// How long an idle worker on a PostgreSQL backend waits before it polls the
/// queue again. Every task routed to this backend is enqueued in this
/// process, which wakes a worker, and a worker that finished a step claims
/// again at once, so the poll only catches a retry's backoff; it is long so
/// idle workers do not query the database twenty times a second.
pub const POSTGRES_WORKER_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Why a session's workflow is ended when it was left running by a backend
/// that no longer runs it; see the module notes.
pub const LEFT_BEHIND: &str = "Left running by a durable backend that no longer runs it";

/// How often a worker heartbeats the task it runs, as the platform worker.
const TASK_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);

/// The activities a turn is made of.
const TURN_ACTIVITIES: [&str; 3] = ["process_input", "reason", "act"];

/// Runs turns as queued, checkpointed steps on a durable store (in memory, or
/// PostgreSQL shared with other processes), with a pool of workers in this
/// process.
///
/// **Experimental**, with [`TurnBackend`].
///
/// Attach each session's runtime with [`attach`](Self::attach); the returned
/// [`DurableSessionBackend`] is that session's [`TurnBackend`]. Clones share
/// the store and the workers. The workers start with the first attached
/// session and stop when the last handle drops.
///
/// # Example
///
/// ```
/// use everruns_contracts::typed_id::TurnId;
/// use everruns_durable_engine::DurableBackend;
/// use everruns_durable_engine::core::InputMessage;
/// use everruns_durable_engine::host::{
///     AcceptedTurnInput, InProcessRuntime, TurnBackend, TurnInput, TurnRequest,
/// };
/// use everruns_llmsim::{LlmSimConfig, LlmSimRuntimeExt};
///
/// # #[tokio::main(flavor = "current_thread")]
/// # async fn main() -> everruns_contracts::error::Result<()> {
/// let runtime = InProcessRuntime::builder()
///     .llm_sim_as_default(LlmSimConfig::fixed("durably done"))
///     .single_session(|session| session)
///     .build()
///     .await?;
/// let session_id = runtime.default_session_id().expect("single_session seeds one");
///
/// let backend = DurableBackend::memory(2);
/// let session = backend.attach(session_id, runtime);
/// let ticket = session
///     .start_turn(TurnRequest::new(
///         session_id,
///         TurnId::new(),
///         TurnInput::Message(Box::new(AcceptedTurnInput::new(InputMessage::user("hi")))),
///     ))
///     .await?;
/// assert_eq!(ticket.await?.response, "durably done");
///
/// backend.shutdown().await;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct DurableBackend {
    shared: Arc<Shared>,
    /// Stops the workers when the last handle drops.
    _owner: Arc<Owner>,
}

struct Owner {
    shutdown: CancellationToken,
}

impl Drop for Owner {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

struct Shared {
    store: Arc<dyn TurnStore>,
    /// The PostgreSQL store other backends share, for ending the workflows
    /// they left behind; `None` for a store this backend owns.
    shared_store: Option<PostgresWorkflowEventStore>,
    runner: DurableRunner,
    sessions: Mutex<HashMap<SessionId, Arc<SessionSlot>>>,
    wake: Arc<Notify>,
    shutdown: CancellationToken,
    worker_count: usize,
    poll_interval: Duration,
    workers: Mutex<Option<Vec<JoinHandle<()>>>>,
    running_workers: Arc<AtomicUsize>,
}

/// Wakes an idle worker when a turn starts.
struct WakeWorkers(Arc<Notify>);

#[async_trait]
impl DurableTaskNotifier for WakeWorkers {
    async fn notify_task_available(&self, _activity_type: &str) {
        self.0.notify_one();
    }
}

impl DurableBackend {
    /// A backend over a fresh in-memory durable store, run by `workers`
    /// in-process workers (at least one).
    ///
    /// The store lives as long as the backend: turns survive nothing beyond
    /// the process, but every step is queued and checkpointed exactly as on a
    /// persistent store.
    pub fn memory(workers: usize) -> Self {
        Self::new(
            Arc::new(InMemoryWorkflowEventStore::new()),
            None,
            workers,
            WORKER_POLL_INTERVAL,
        )
    }

    /// A backend over a PostgreSQL durable store, run by `workers`
    /// in-process workers (at least one).
    ///
    /// Every step is queued and checkpointed in the database, so the queue
    /// can be shared: several backends, in one process or many, may use the
    /// same database. Each claims only the steps of the sessions attached to
    /// it, so a step always runs where its session's runtime lives. A session
    /// that comes back after its process exited (attached again, here or in
    /// a new process) first ends whatever workflow the old backend left
    /// running for it; a turn cut off in its tool calls then continues from
    /// the session log with [`TurnInput::ResumeInterrupted`], as in process.
    ///
    /// The store must carry `everruns-durable`'s schema
    /// ([`PostgresWorkflowEventStore::connect`] applies it).
    ///
    /// # Example
    ///
    /// ```no_run
    /// use everruns_durable_engine::DurableBackend;
    /// use everruns_durable_engine::durable::PostgresWorkflowEventStore;
    /// use everruns_durable_engine::host::InProcessRuntime;
    /// # use everruns_contracts::typed_id::SessionId;
    ///
    /// # async fn run(runtime: InProcessRuntime, session_id: SessionId)
    /// # -> Result<(), Box<dyn std::error::Error>> {
    /// let store = PostgresWorkflowEventStore::connect("postgres://localhost/my_app").await?;
    /// let backend = DurableBackend::postgres(store, 4);
    /// let session = backend.attach(session_id, runtime);
    /// // `session` is the session's `TurnBackend`, as on the memory store.
    /// # let _ = session;
    /// # Ok(())
    /// # }
    /// ```
    pub fn postgres(store: PostgresWorkflowEventStore, workers: usize) -> Self {
        let routing = Routing::unique();
        debug!(queue = routing.queue(), "durable PostgreSQL backend");
        Self::new(
            Arc::new(RoutedStore::new(Arc::new(store.clone()), routing)),
            Some(store),
            workers,
            POSTGRES_WORKER_POLL_INTERVAL,
        )
    }

    /// The runner and the workers share `store`, so a routed store's tags
    /// and local workflow-end wakeups cover both.
    fn new(
        store: Arc<dyn TurnStore>,
        shared_store: Option<PostgresWorkflowEventStore>,
        workers: usize,
        poll_interval: Duration,
    ) -> Self {
        let wake = Arc::new(Notify::new());
        let runner = DurableRunner::from_shared(store.clone())
            .with_task_notifier(Arc::new(WakeWorkers(wake.clone())));
        let shutdown = CancellationToken::new();
        Self {
            shared: Arc::new(Shared {
                store,
                shared_store,
                runner,
                sessions: Mutex::default(),
                wake,
                shutdown: shutdown.clone(),
                worker_count: workers.max(1),
                poll_interval,
                workers: Mutex::new(None),
                running_workers: Arc::default(),
            }),
            _owner: Arc::new(Owner { shutdown }),
        }
    }

    /// Run `session_id`'s turns on this backend, each step on `runtime`.
    ///
    /// Starts the workers if they are not running yet, so call it inside a
    /// Tokio runtime. Attaching a session again replaces its runtime; the
    /// earlier handle then no longer reaches the session.
    pub fn attach(
        &self,
        session_id: SessionId,
        runtime: InProcessRuntime,
    ) -> DurableSessionBackend {
        self.shared.start_workers();
        let slot = Arc::new(SessionSlot::new(session_id, runtime));
        lock(&self.shared.sessions).insert(session_id, slot.clone());
        DurableSessionBackend {
            shared: self.shared.clone(),
            slot,
        }
    }

    /// How many workers are running. Zero before the first
    /// [`attach`](Self::attach) and after shutdown.
    pub fn running_workers(&self) -> usize {
        self.shared.running_workers.load(Ordering::SeqCst)
    }

    /// The live worker count, readable after this handle drops.
    #[cfg(test)]
    pub(crate) fn running_worker_counter(&self) -> Arc<AtomicUsize> {
        self.shared.running_workers.clone()
    }

    /// Stop the workers and wait until they have stopped. A step in flight is
    /// dropped. Later turns stay queued; no worker runs them.
    pub async fn shutdown(&self) {
        self.shared.shutdown.cancel();
        let handles = lock(&self.shared.workers).take().unwrap_or_default();
        for handle in handles {
            let _ = handle.await;
        }
    }
}

impl fmt::Debug for DurableBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DurableBackend")
            .field("workers", &self.shared.worker_count)
            .field("running_workers", &self.running_workers())
            .field("sessions", &lock(&self.shared.sessions).len())
            .finish_non_exhaustive()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every critical section is a single insert, remove, take or replace, so a
    // poisoned lock still guards a consistent value.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    fn start_workers(self: &Arc<Self>) {
        let mut workers = lock(&self.workers);
        if workers.is_some() || self.shutdown.is_cancelled() {
            return;
        }
        let handles = (0..self.worker_count)
            .map(|index| {
                let worker_id = format!("in-process-{}-{index}", uuid::Uuid::now_v7());
                // Count the worker before it is spawned, so a caller that
                // just attached already sees it.
                self.running_workers.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(run_worker(
                    Arc::downgrade(self),
                    worker_id,
                    self.poll_interval,
                    self.wake.clone(),
                    self.shutdown.clone(),
                    self.running_workers.clone(),
                ))
            })
            .collect();
        *workers = Some(handles);
    }

    fn slot(&self, session_id: SessionId) -> Option<Arc<SessionSlot>> {
        lock(&self.sessions).get(&session_id).cloned()
    }
}

/// Decrements the running-worker count however the worker ends.
struct WorkerExit(Arc<AtomicUsize>);

impl Drop for WorkerExit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// One worker: claim a turn task, run it on its session's runtime, repeat.
///
/// Holds the backend weakly between tasks, so a worker never keeps the
/// sessions' runtimes alive by itself.
async fn run_worker(
    shared: std::sync::Weak<Shared>,
    worker_id: String,
    poll_interval: Duration,
    wake: Arc<Notify>,
    shutdown: CancellationToken,
    running: Arc<AtomicUsize>,
) {
    let _exit = WorkerExit(running);
    let activity_types = TURN_ACTIVITIES.map(String::from);
    let store = match shared.upgrade() {
        Some(shared) => shared.store.clone(),
        None => return,
    };
    if let Err(error) = store
        .register_worker(WorkerInfo::new(worker_id.clone(), TURN_ACTIVITIES))
        .await
    {
        warn!(%worker_id, %error, "durable backend worker failed to register");
        return;
    }
    while !shutdown.is_cancelled() {
        let claimed = store.claim_task(&worker_id, &activity_types, 1).await;
        match claimed.map(|tasks| tasks.into_iter().next()) {
            Ok(Some(task)) => {
                let Some(shared) = shared.upgrade() else {
                    break;
                };
                run_task(&shared, &worker_id, &shutdown, task).await;
            }
            Ok(None) => {
                tokio::select! {
                    () = shutdown.cancelled() => break,
                    () = wake.notified() => {}
                    () = tokio::time::sleep(poll_interval) => {}
                }
            }
            Err(error) => {
                warn!(%worker_id, %error, "durable backend worker failed to claim a task");
                tokio::select! {
                    () = shutdown.cancelled() => break,
                    () = tokio::time::sleep(poll_interval) => {}
                }
            }
        }
    }
    let _ = store.deregister_worker(&worker_id).await;
}

/// Run one claimed task on its session's runtime, unless the session is gone
/// or its turn was cancelled.
async fn run_task(
    shared: &Shared,
    worker_id: &str,
    shutdown: &CancellationToken,
    task: ClaimedTask,
) {
    let Some(slot) = task
        .workflow_id
        .map(SessionId::from_uuid)
        .and_then(|session_id| shared.slot(session_id))
    else {
        fail_task(
            shared,
            &task,
            "no session runtime is attached to this backend",
        )
        .await;
        return;
    };
    // One step of a session at a time; `cancel` takes this lock to wait out
    // the step it cancels.
    let _step = slot.step.lock().await;
    let cancel = slot.turn().cancel.clone();
    if cancel.is_cancelled() {
        fail_task(shared, &task, "Workflow cancelled").await;
        return;
    }
    let driver = TurnTaskDriver::new(
        shared.store.clone(),
        SessionHosts(slot.clone()),
        worker_id,
        TASK_HEARTBEAT_INTERVAL,
    );
    tokio::select! {
        biased;
        // The step is dropped mid-flight, as an in-process turn is when its
        // ticket is cancelled. Failing the task without retry keeps it from
        // ever running again, in this turn or the session's next one.
        () = cancel.cancelled() => fail_task(shared, &task, "Workflow cancelled").await,
        () = shutdown.cancelled() => {}
        result = driver.execute_task(&task) => {
            if let Err(error) = result {
                debug!(task_id = %task.id, %error, "durable turn step failed");
            }
        }
    }
}

async fn fail_task(shared: &Shared, task: &ClaimedTask, reason: &str) {
    if let Err(error) = shared.store.fail_task_and_record(task, reason, false).await {
        warn!(task_id = %task.id, %error, "failed to fail a durable turn task");
    }
}

/// What the backend keeps for one attached session.
struct SessionSlot {
    session_id: SessionId,
    runtime: InProcessRuntime,
    turn: Mutex<SlotTurn>,
    step: tokio::sync::Mutex<()>,
    /// Whether this attachment already ended any workflow another backend
    /// left running for the session; see the module notes.
    recovered: AtomicBool,
}

/// The session's current turn, as its steps need it.
struct SlotTurn {
    steering: TurnSteering,
    cancel: CancellationToken,
    /// Steering persisted after a reason, for the next reason's
    /// `user_prompt_submit` boundary.
    pending_prompt_messages: Vec<MessageId>,
}

impl SessionSlot {
    fn new(session_id: SessionId, runtime: InProcessRuntime) -> Self {
        let steering = TurnSteering::new();
        steering.close();
        Self {
            session_id,
            runtime,
            turn: Mutex::new(SlotTurn {
                steering,
                cancel: CancellationToken::new(),
                pending_prompt_messages: Vec::new(),
            }),
            step: tokio::sync::Mutex::new(()),
            recovered: AtomicBool::new(false),
        }
    }

    fn turn(&self) -> MutexGuard<'_, SlotTurn> {
        lock(&self.turn)
    }

    fn steering(&self) -> TurnSteering {
        self.turn().steering.clone()
    }
}

/// The session's runtime as the driver's host, plus the in-process steering
/// contract at the driver's boundaries.
#[derive(Clone)]
struct SessionHosts(Arc<SessionSlot>);

#[async_trait]
impl TurnTaskHost for SessionHosts {
    type Host = InProcessRuntime;

    fn host(&self) -> InProcessRuntime {
        self.0.runtime.clone()
    }

    fn reason_host(&self, _input: &ReasonInput, _cancel: CancelSignals) -> InProcessRuntime {
        // Cancellation drops the step instead (see `run_task`).
        self.0.runtime.clone()
    }

    async fn before_reason(&self, _input: &DurableTurnInput) -> anyhow::Result<Vec<MessageId>> {
        let (mut prompt_messages, steering) = {
            let mut turn = self.0.turn();
            (
                std::mem::take(&mut turn.pending_prompt_messages),
                turn.steering.clone(),
            )
        };
        for input in steering.drain() {
            prompt_messages.push(
                self.0
                    .runtime
                    .persist_accepted_input(self.0.session_id, input)
                    .await?,
            );
        }
        Ok(prompt_messages)
    }

    async fn after_reason(
        &self,
        input: &DurableTurnInput,
        reason: &ReasonResult,
        drained_wakes: usize,
    ) -> anyhow::Result<usize> {
        // The same decision `run_steerable_turn` makes after a reason: only a
        // reason that would otherwise finish may close user ingress, and the
        // drain-or-close is one atomic step.
        let steering = self.0.steering();
        let can_continue = reason.success && (input.iteration as usize) < reason.max_iterations;
        let pending = if !reason_schedules_act(input, reason) && can_continue && drained_wakes == 0
        {
            steering.drain_or_close()
        } else {
            Vec::new()
        };
        if !can_continue {
            steering.close();
        }
        let count = pending.len();
        let mut persisted = Vec::with_capacity(count);
        for input in pending {
            persisted.push(
                self.0
                    .runtime
                    .persist_accepted_input(self.0.session_id, input)
                    .await?,
            );
        }
        self.0.turn().pending_prompt_messages.extend(persisted);
        Ok(count)
    }

    async fn turn_planned(
        &self,
        checkpoint: &DurableTurnInput,
        plan: &TurnPlan,
        output: &serde_json::Value,
    ) -> anyhow::Result<()> {
        let steering = self.0.steering();
        match plan {
            TurnPlan::Complete { .. } => steering.close(),
            // A parked turn keeps what was steered into it and waits on the
            // runtime for its client-side results, as in process.
            TurnPlan::WaitForToolResults { resume } => {
                steering.close();
                if let Some(turn_id) = checkpoint.turn_id {
                    self.0
                        .runtime
                        .append_accepted_inputs(self.0.session_id, turn_id, steering.drain())
                        .await?;
                    let client_tool_calls = output
                        .get("client_tool_calls")
                        .cloned()
                        .map(serde_json::from_value)
                        .transpose()?
                        .unwrap_or_default();
                    self.0.runtime.park_turn(
                        self.0.session_id,
                        turn_id,
                        client_tool_calls,
                        resume.clone(),
                    );
                }
            }
            TurnPlan::ScheduleReason(_) | TurnPlan::ScheduleAct(_) => {}
        }
        Ok(())
    }
}

/// One session's [`TurnBackend`] on a [`DurableBackend`], from
/// [`DurableBackend::attach`].
///
/// **Experimental**, with [`TurnBackend`].
///
/// Serves every [`TurnInput`]: new and stored messages, new and recorded
/// tool results, and [`TurnInput::ResumeInterrupted`]. The workflow starts before
/// [`start_turn`](TurnBackend::start_turn) returns, so the turn runs whether
/// or not its ticket is polled; the ticket resolves when the workflow ends.
/// Dropping this handle detaches the session: its in-flight step is dropped
/// and its remaining steps fail.
pub struct DurableSessionBackend {
    shared: Arc<Shared>,
    slot: Arc<SessionSlot>,
}

impl fmt::Debug for DurableSessionBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DurableSessionBackend")
            .field("session_id", &self.slot.session_id)
            .finish_non_exhaustive()
    }
}

impl Drop for DurableSessionBackend {
    fn drop(&mut self) {
        let mut sessions = lock(&self.shared.sessions);
        if sessions
            .get(&self.slot.session_id)
            .is_some_and(|slot| Arc::ptr_eq(slot, &self.slot))
        {
            sessions.remove(&self.slot.session_id);
        }
        drop(sessions);
        let turn = self.slot.turn();
        turn.steering.close();
        turn.cancel.cancel();
    }
}

fn unsupported_input() -> AgentLoopError {
    AgentLoopError::config("the durable backend cannot run this turn input")
}

fn already_running(session_id: SessionId) -> AgentLoopError {
    AgentLoopError::store(format!("session {session_id} already runs a turn"))
}

fn store_error(error: anyhow::Error) -> AgentLoopError {
    AgentLoopError::store(format!("{error:#}"))
}

impl DurableSessionBackend {
    /// On a shared store, end the workflow another backend left running for
    /// this session, once per attachment: fail the steps its gone workers
    /// still hold, then cancel it, so the session's next turn can start. See
    /// the module notes.
    async fn recover_left_behind(&self, session_id: SessionId) -> Result<()> {
        let Some(store) = &self.shared.shared_store else {
            return Ok(());
        };
        if self.slot.recovered.load(Ordering::SeqCst) {
            return Ok(());
        }
        if self.shared.runner.is_running(session_id).await {
            let claimed = store
                .list_tasks(
                    TaskFilter {
                        status: Some(TaskStatus::Claimed),
                        workflow_id: Some(session_id.uuid()),
                        ..TaskFilter::default()
                    },
                    Pagination::default(),
                )
                .await
                .map_err(|error| store_error(error.into()))?;
            for task in claimed {
                // Not retryable: the step goes to the dead-letter queue
                // instead of back to a queue no worker of this key claims
                // from. A step that is no longer claimed was failed already.
                if let Err(error) = store
                    .fail_task_with_retry(task.id, LEFT_BEHIND, false)
                    .await
                {
                    debug!(task_id = %task.id, %error, "left-behind step already ended");
                }
            }
            self.shared
                .runner
                .cancel_workflow(session_id, LEFT_BEHIND)
                .await?;
            warn!(%session_id, "ended a durable turn workflow another backend left running");
        }
        self.slot.recovered.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Make `steering` the session's current turn's, with a fresh cancel.
    fn begin_turn(&self, steering: TurnSteering) {
        *self.slot.turn() = SlotTurn {
            steering,
            cancel: CancellationToken::new(),
            pending_prompt_messages: Vec::new(),
        };
    }

    /// Start a new turn from `input`, persisted exactly as
    /// `InProcessRuntime::run_steerable_turn` persists it.
    async fn start_message(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
        input: AcceptedTurnInput,
        steering: TurnSteering,
        request_id: Option<String>,
    ) -> Result<TurnTicket> {
        let runtime = &self.slot.runtime;
        let snapshot = runtime.resolved_execution_snapshot(session_id).await?;
        self.begin_turn(steering);
        let input_message_id = runtime.persist_accepted_input(session_id, input).await?;
        self.start_from_message(session_id, turn_id, &snapshot, input_message_id, request_id)
            .await
    }

    /// Start a new turn from the message `input_message_id` the caller
    /// already recorded in the session's log.
    async fn start_stored_message(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
        input_message_id: MessageId,
        steering: TurnSteering,
        request_id: Option<String>,
    ) -> Result<TurnTicket> {
        let snapshot = self
            .slot
            .runtime
            .resolved_execution_snapshot(session_id)
            .await?;
        self.begin_turn(steering);
        self.start_from_message(session_id, turn_id, &snapshot, input_message_id, request_id)
            .await
    }

    /// The workflow of a turn whose input message is in the session's log.
    async fn start_from_message(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
        snapshot: &ResolvedExecutionSnapshot,
        input_message_id: MessageId,
        request_id: Option<String>,
    ) -> Result<TurnTicket> {
        // A new turn supersedes one parked on client-side tool calls.
        self.slot.runtime.supersede_parked_turn(session_id);
        let turn_input = DurableTurnInput {
            org_id: in_process_internal_org_id(&snapshot.organization_id),
            session_id,
            harness_id: snapshot.harness_id,
            agent_id: snapshot.agent_id,
            input_message_id,
            turn_id: Some(turn_id),
            previous_response_id: None,
            iteration: 1,
            request_id,
            started_at: Some(Utc::now()),
            cumulative_usage: None,
            tool_call_count: 0,
            llm_call_count: 0,
            time_to_first_token_ms: None,
            final_message_id: None,
            final_answer_preview: None,
        };
        let started = self
            .shared
            .runner
            .start_workflow(session_id.uuid(), &turn_input)
            .await
            .map_err(store_error)?;
        if !started {
            return Err(already_running(session_id));
        }
        Ok(self.shared.runner.ticket(session_id, turn_id))
    }

    /// Continue the turn parked on client-side tool calls: record `results`
    /// under it on the runtime, then resume the workflow from the checkpoint
    /// it parked with, through the runner's tool-resolution resume.
    ///
    /// `results` may be empty when the caller recorded them already
    /// ([`TurnInput::RecordedToolResults`]); `resolution_id` keys the resume.
    async fn resume_tool_results(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
        results: Vec<ToolCompletedData>,
        resolution_id: uuid::Uuid,
        steering: TurnSteering,
    ) -> Result<TurnTicket> {
        let resume = self
            .slot
            .runtime
            .deliver_parked_tool_results(session_id, results)
            .await?;
        self.begin_turn(steering);
        self.shared
            .runner
            .resume_persisted_resolution(session_id, resolution_id)
            .await
            .map_err(store_error)?;
        let baseline = TurnBaseline {
            // The next reason runs at the resume state's iteration.
            iterations: resume.iteration.saturating_sub(1),
            tool_calls: resume.tool_call_count,
        };
        Ok(self
            .shared
            .runner
            .ticket_after(session_id, resume.turn_id.unwrap_or(turn_id), baseline))
    }

    /// Continue the turn a process exit cut off in its act: a workflow whose
    /// first task is the act that runs its unfinished calls again.
    async fn resume_interrupted(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
        steering: TurnSteering,
    ) -> Result<TurnTicket> {
        let (state, plan) = self
            .slot
            .runtime
            .interrupted_turn_plan(session_id)
            .await?
            .ok_or_else(|| {
                AgentLoopError::store(format!(
                    "session {session_id} has no turn interrupted in its tool calls"
                ))
            })?;
        self.begin_turn(steering);
        let rerun = u32::try_from(plan.input.tool_calls.len()).unwrap_or(u32::MAX);
        let input = act_task_input(&plan, &state).map_err(store_error)?;
        let started = self
            .shared
            .runner
            .start_workflow_at(
                session_id.uuid(),
                format!("act_{}", uuid::Uuid::now_v7()),
                "act",
                input,
            )
            .await
            .map_err(store_error)?;
        if !started {
            return Err(already_running(session_id));
        }
        let baseline = TurnBaseline {
            // The act moves the turn to its next iteration's reason.
            iterations: state.iteration,
            // The log already counts the calls the act runs again; in process
            // the act counts them for this run.
            tool_calls: state.tool_call_count.saturating_sub(rerun),
        };
        Ok(self
            .shared
            .runner
            .ticket_after(session_id, state.turn_id.unwrap_or(turn_id), baseline))
    }
}

#[async_trait]
impl TurnBackend for DurableSessionBackend {
    async fn start_turn(&self, request: TurnRequest) -> Result<TurnTicket> {
        let TurnRequest {
            session_id,
            turn_id,
            input,
            steering,
            request_id,
            ..
        } = request;
        if session_id != self.slot.session_id {
            return Err(AgentLoopError::config(format!(
                "this durable backend handle runs session {}, not {session_id}",
                self.slot.session_id
            )));
        }
        self.recover_left_behind(session_id).await?;
        if self.shared.runner.is_running(session_id).await {
            return Err(already_running(session_id));
        }
        match input {
            TurnInput::Message(input) => {
                self.start_message(session_id, turn_id, *input, steering, request_id)
                    .await
            }
            TurnInput::StoredMessage { message_id } => {
                self.start_stored_message(session_id, turn_id, message_id, steering, request_id)
                    .await
            }
            TurnInput::ToolResults(results) => {
                // The facade has no stored resolution; a fresh id keys this
                // resume.
                self.resume_tool_results(
                    session_id,
                    turn_id,
                    results,
                    uuid::Uuid::now_v7(),
                    steering,
                )
                .await
            }
            TurnInput::RecordedToolResults { resolution_id } => {
                self.resume_tool_results(session_id, turn_id, Vec::new(), resolution_id, steering)
                    .await
            }
            TurnInput::ResumeInterrupted => {
                self.resume_interrupted(session_id, turn_id, steering).await
            }
            _ => Err(unsupported_input()),
        }
    }

    async fn cancel(&self, session_id: SessionId) -> Result<bool> {
        if !self.is_running(session_id).await {
            return Ok(false);
        }
        let was_running = self.shared.runner.cancel(session_id).await?;
        let cancel = {
            let turn = self.slot.turn();
            turn.steering.close();
            turn.cancel.clone()
        };
        cancel.cancel();
        // Wait out the step in flight: the worker drops it on the token and
        // fails its task before it releases the session.
        drop(self.slot.step.lock().await);
        Ok(was_running)
    }

    async fn is_running(&self, session_id: SessionId) -> bool {
        session_id == self.slot.session_id && self.shared.runner.is_running(session_id).await
    }

    async fn active_count(&self) -> usize {
        let sessions: Vec<_> = lock(&self.shared.sessions).keys().copied().collect();
        let mut count = 0;
        for session_id in sessions {
            if self.shared.runner.is_running(session_id).await {
                count += 1;
            }
        }
        count
    }
}
