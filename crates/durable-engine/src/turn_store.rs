//! The one store contract agent turns run on: the runner starts, steers,
//! cancels and observes a session's turn workflow through it, and the worker
//! loop and [`TurnTaskDriver`](crate::TurnTaskDriver) claim, run and settle
//! its tasks through it.
//!
//! Every `WorkflowEventStore` gets this trait through the blanket impl below.
//! Transport-backed stores implement it from their own crate for their own
//! type (the worker does so for its gRPC client), which coherence accepts
//! because that type does not implement `WorkflowEventStore`.
//!
//! Decision: one trait, not a runner store and a separate task store. The two
//! halves were always implemented by the same three stores (memory,
//! PostgreSQL, the worker's gRPC client), duplicated their workflow-status
//! reads and writes, and a backend had to wrap both to route one queue.

use std::any::Any;
use std::future::Future;
use std::pin::Pin;

use crate::durable::{
    ClaimedTask, DurableAdmin, Enqueued, EventLog, HandOff, HeartbeatResponse,
    InMemoryWorkflowEventStore, NextStep, RunStart, SignalDrain, SignalStore, StoreError,
    TaskDefinition, TaskFailureOutcome, TaskQueue, WorkerHeartbeat, WorkerInfo, WorkerRegistry,
    WorkflowError, WorkflowEvent, WorkflowEventStore, WorkflowSignal, WorkflowStatus, append_event,
    record_activity_completed, record_activity_failed, record_activity_started,
    record_workflow_failed,
};
use crate::durable_turn::activity_options_for;
use async_trait::async_trait;
use uuid::Uuid;

/// Each item signals that new work may be claimable. Closure means resubscribe.
pub type TaskWakeups = tokio::sync::mpsc::Receiver<()>;

/// Resolves when a workflow next reaches a terminal status; see
/// [`TurnStore::workflow_end_signal`].
pub type WorkflowEndSignal = Pin<Box<dyn Future<Output = ()> + Send>>;

/// A workflow's status with what it stored: the turn checkpoint as `output`,
/// and the message of the error it ended with.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowSnapshot {
    pub status: WorkflowStatus,
    pub output: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[async_trait]
pub trait TurnStore: Send + Sync + 'static {
    // ---- Workers -------------------------------------------------------

    async fn register_worker(&self, worker: WorkerInfo) -> Result<(), StoreError>;

    /// Records liveness and load; the reply says whether the worker is
    /// draining, so it can stop claiming.
    async fn worker_heartbeat(
        &self,
        worker_id: &str,
        current_load: usize,
        accepting_tasks: bool,
    ) -> Result<WorkerHeartbeat, StoreError>;

    /// Marks the worker draining: the queue hands it no new tasks and no
    /// chained steps. A worker calls it on itself when it starts shutting down.
    async fn drain_worker(&self, worker_id: &str) -> Result<(), StoreError>;

    async fn deregister_worker(&self, worker_id: &str) -> Result<usize, StoreError>;

    // ---- Tasks ---------------------------------------------------------

    async fn claim_task(
        &self,
        worker_id: &str,
        activity_types: &[String],
        max_tasks: usize,
    ) -> Result<Vec<ClaimedTask>, StoreError>;

    async fn heartbeat_task(
        &self,
        task_id: Uuid,
        worker_id: &str,
        details: Option<serde_json::Value>,
    ) -> Result<HeartbeatResponse, StoreError>;

    async fn record_activity_started(&self, task: &ClaimedTask, worker_id: &str);

    async fn complete_task_and_record(
        &self,
        task: &ClaimedTask,
        worker_id: &str,
        output: serde_json::Value,
    ) -> Result<(), StoreError>;

    /// Complete `task` and hand its workflow to the next step in one atomic
    /// store write: consume the steering signals the next step was planned
    /// with (`hand_off.drain`), then enqueue that step or end the workflow.
    /// Returns the next step when it was enqueued claimed by
    /// `TurnNext::Step::claim_for`. Nothing changes on `TaskNotOwned`.
    ///
    /// Decision: one write, so no process exit or lost reply can leave the
    /// workflow running with no task and a drained wake with nothing to act
    /// on it. The default makes the writes one after another
    /// ([`hand_off_in_steps`]), for a store with no atomic form; a run it
    /// strands is resumed by the stranded-run sweep
    /// ([`crate::durable::requeue_stranded_workflows`]).
    async fn complete_task_and_hand_off(
        &self,
        task: &ClaimedTask,
        worker_id: &str,
        output: serde_json::Value,
        hand_off: TurnHandOff,
    ) -> Result<Option<ClaimedTask>, StoreError> {
        hand_off_in_steps(self, task, worker_id, output, hand_off).await
    }

    /// How many signals of `signal_type` are pending on `workflow_id`,
    /// without consuming them. A step plans its successor with this count,
    /// then its hand-off consumes exactly that many.
    async fn count_pending_signals(
        &self,
        workflow_id: Uuid,
        signal_type: &str,
    ) -> Result<usize, StoreError>;

    async fn fail_task_and_record(
        &self,
        task: &ClaimedTask,
        error: &str,
        retryable: bool,
    ) -> Result<TaskFailureOutcome, StoreError>;

    /// Record `ActivityScheduled` and enqueue the task, with the turn options
    /// its activity id implies ([`activity_options_for`]).
    async fn enqueue_task_and_record(
        &self,
        workflow_id: Uuid,
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
    ) -> Result<Uuid, StoreError>;

    /// Enqueue and record a workflow's next step already claimed by
    /// `worker_id`, which then runs it without a wakeup and a claim (see
    /// [`TaskQueue::enqueue_claimed_task`]). Returns `None` when the task
    /// went to the queue instead; the default always does that.
    async fn enqueue_claimed_task_and_record(
        &self,
        workflow_id: Uuid,
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
        worker_id: &str,
    ) -> Result<Option<ClaimedTask>, StoreError> {
        let _ = worker_id;
        self.enqueue_task_and_record(workflow_id, activity_id, activity_type, input)
            .await?;
        Ok(None)
    }

    /// Drop the workflow's pending (unclaimed) tasks; how many were dropped.
    async fn cancel_pending_tasks(&self, workflow_id: Uuid) -> Result<u64, StoreError>;

    /// Open a push channel that signals new claimable work.
    ///
    /// `Ok(None)` means the store has none and the worker polls only. See
    /// the worker wake-up listener for subscription recovery.
    async fn subscribe_task_wakeups(
        &self,
        _worker_id: &str,
        _activity_types: &[String],
    ) -> Result<Option<TaskWakeups>, StoreError> {
        Ok(None)
    }

    // ---- Workflows -----------------------------------------------------

    /// Start a turn: create the session's workflow or start a new run of it,
    /// and enqueue `activity_type` as its first task, in one atomic step
    /// ([`EventLog::start_run_with_task`]). Returns [`RunStart::Active`],
    /// changing nothing, while a turn is already running.
    async fn start_turn(
        &self,
        workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
        activity_id: String,
        activity_type: String,
    ) -> Result<RunStart, StoreError>;

    async fn get_workflow(&self, workflow_id: Uuid) -> Result<WorkflowSnapshot, StoreError>;

    async fn update_workflow_status(
        &self,
        workflow_id: Uuid,
        status: WorkflowStatus,
        output: Option<serde_json::Value>,
        error: Option<WorkflowError>,
    ) -> Result<(), StoreError>;

    async fn complete_workflow(
        &self,
        workflow_id: Uuid,
        event_output: serde_json::Value,
        stored_output: Option<serde_json::Value>,
        error: Option<WorkflowError>,
    ) -> Result<(), StoreError>;

    /// Workflows still pending or running.
    async fn count_active_workflows(&self) -> Result<usize, StoreError>;

    /// The output the workflow's latest `WorkflowCompleted` event recorded.
    ///
    /// A [`DurableRunner`](crate::DurableRunner) turn ticket reads it to fill
    /// in the turn's response and stop reason. The default returns `None`,
    /// for a store without a cheap event-log read; the ticket then falls back
    /// to the turn checkpoint (see [`crate::turn_backend`]).
    async fn latest_completion_output(
        &self,
        workflow_id: Uuid,
    ) -> Result<Option<serde_json::Value>, StoreError> {
        let _ = workflow_id;
        Ok(None)
    }

    /// A signal that resolves when `workflow_id` next reaches a terminal
    /// status, subscribed when this returns.
    ///
    /// A [`DurableRunner`](crate::DurableRunner) turn ticket takes it before
    /// each status read and waits on it instead of the short poll, so a turn
    /// reports back as soon as its workflow ends. The default returns `None`,
    /// for a store whose workflows other processes may end (PostgreSQL); the
    /// ticket then polls (see [`crate::turn_backend`]). A store that returns
    /// a signal must fire it on every terminal transition this process makes;
    /// the ticket still re-reads the status on a long fallback interval.
    fn workflow_end_signal(&self, workflow_id: Uuid) -> Option<WorkflowEndSignal> {
        let _ = workflow_id;
        None
    }

    // ---- Signals -------------------------------------------------------

    async fn send_signal(
        &self,
        workflow_id: Uuid,
        signal: WorkflowSignal,
    ) -> Result<(), StoreError>;

    async fn consume_pending_signals(
        &self,
        workflow_id: Uuid,
    ) -> Result<Vec<WorkflowSignal>, StoreError>;

    async fn consume_pending_signals_by_type(
        &self,
        workflow_id: Uuid,
        signal_type: &str,
    ) -> Result<Vec<WorkflowSignal>, StoreError>;
}

/// What a completed turn step hands its workflow to; see
/// [`TurnStore::complete_task_and_hand_off`].
#[derive(Debug, Clone)]
pub enum TurnNext {
    /// Enqueue the next step, claimed by `claim_for` when set and able.
    Step {
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
        claim_for: Option<String>,
    },
    /// End the turn: record `event_output` as `WorkflowCompleted` and store
    /// `stored_output` (the turn checkpoint) on the completed workflow.
    Complete {
        event_output: serde_json::Value,
        stored_output: Option<serde_json::Value>,
        error: Option<WorkflowError>,
    },
}

/// A turn step's hand-off of `workflow_id`; see
/// [`TurnStore::complete_task_and_hand_off`].
#[derive(Debug, Clone)]
pub struct TurnHandOff {
    pub workflow_id: Uuid,
    /// The pending signals the plan counted, consumed with the hand-off.
    pub drain: Option<SignalDrain>,
    pub next: TurnNext,
}

/// [`TurnStore::complete_task_and_hand_off`] as separate writes: complete,
/// drain, then enqueue or end. A store that cannot hand off atomically (or a
/// server too old to) uses it; see the trait method on what it risks.
///
/// The drain consumes every pending signal of its type, as these stores'
/// consume calls do, not only the ones the plan counted.
pub async fn hand_off_in_steps<S: TurnStore + ?Sized>(
    store: &S,
    task: &ClaimedTask,
    worker_id: &str,
    output: serde_json::Value,
    hand_off: TurnHandOff,
) -> Result<Option<ClaimedTask>, StoreError> {
    store
        .complete_task_and_record(task, worker_id, output)
        .await?;
    finish_hand_off_in_steps(store, hand_off).await
}

/// The writes of [`hand_off_in_steps`] after the completion: drain, then
/// enqueue or end.
pub async fn finish_hand_off_in_steps<S: TurnStore + ?Sized>(
    store: &S,
    hand_off: TurnHandOff,
) -> Result<Option<ClaimedTask>, StoreError> {
    let TurnHandOff {
        workflow_id,
        drain,
        next,
    } = hand_off;
    if let Some(drain) = drain.filter(|drain| drain.limit > 0) {
        store
            .consume_pending_signals_by_type(workflow_id, &drain.signal_type)
            .await?;
    }
    match next {
        TurnNext::Step {
            activity_id,
            activity_type,
            input,
            claim_for: Some(worker_id),
        } => {
            store
                .enqueue_claimed_task_and_record(
                    workflow_id,
                    activity_id,
                    activity_type,
                    input,
                    &worker_id,
                )
                .await
        }
        TurnNext::Step {
            activity_id,
            activity_type,
            input,
            claim_for: None,
        } => store
            .enqueue_task_and_record(workflow_id, activity_id, activity_type, input)
            .await
            .map(|_| None),
        TurnNext::Complete {
            event_output,
            stored_output,
            error,
        } => store
            .complete_workflow(workflow_id, event_output, stored_output, error)
            .await
            .map(|()| None),
    }
}

/// [`TurnStore::complete_task_and_hand_off`] on a durable store, with the
/// next step in `queue`: one atomic [`TaskQueue::complete_task_and_hand_off`]
/// moves the queue rows; the history events are appended before it, in the
/// order separate writes appended them (`ActivityCompleted`, then
/// `ActivityScheduled` or `WorkflowCompleted`), so a ticket that sees the
/// workflow end finds its completion event. A hand-off rejected because the
/// task was reclaimed leaves those events behind; the reclaiming run's own
/// follow them.
///
/// The server's durable gRPC service runs it for its workers, so it takes
/// the task's id and activity id rather than a claim.
#[allow(clippy::too_many_arguments)]
pub async fn hand_off_and_record<S: WorkflowEventStore>(
    store: &S,
    queue: Option<&str>,
    task_id: Uuid,
    activity_id: &str,
    worker_id: &str,
    output: serde_json::Value,
    hand_off: TurnHandOff,
) -> Result<Option<ClaimedTask>, StoreError> {
    let TurnHandOff {
        workflow_id,
        drain,
        next,
    } = hand_off;
    record_activity_completed(
        store,
        Some(workflow_id),
        activity_id.to_string(),
        output.clone(),
    )
    .await;
    let next = match next {
        TurnNext::Step {
            activity_id,
            activity_type,
            input,
            claim_for,
        } => {
            let task = turn_task(queue, workflow_id, activity_id, activity_type, input);
            if let Err(error) = record_scheduled(store, &task).await {
                tracing::warn!(%workflow_id, %error, "Failed to record ActivityScheduled");
            }
            NextStep::Enqueue {
                task: Box::new(task),
                claim_for,
            }
        }
        TurnNext::Complete {
            event_output,
            stored_output,
            error,
        } => {
            crate::durable::record_workflow_completed(store, workflow_id, event_output).await;
            NextStep::Complete {
                result: stored_output,
                error,
            }
        }
    };
    let handed = TaskQueue::complete_task_and_hand_off(
        store,
        task_id,
        worker_id,
        output,
        HandOff {
            workflow_id,
            drain,
            next,
        },
    )
    .await?;
    Ok(handed.next.and_then(Enqueued::into_claimed))
}

/// A turn task of `workflow_id` for `queue` (`None`: the default queue),
/// with the turn options its activity id implies.
fn turn_task(
    queue: Option<&str>,
    workflow_id: Uuid,
    activity_id: String,
    activity_type: String,
    input: serde_json::Value,
) -> TaskDefinition {
    let mut options = activity_options_for(&activity_id);
    options.queue = queue.map(str::to_owned);
    TaskDefinition {
        workflow_id: Some(workflow_id),
        options,
        activity_id,
        activity_type,
        input,
    }
}

async fn record_scheduled<S: WorkflowEventStore>(
    store: &S,
    task: &TaskDefinition,
) -> Result<(), StoreError> {
    let Some(workflow_id) = task.workflow_id else {
        return Ok(());
    };
    let event = WorkflowEvent::ActivityScheduled {
        activity_id: task.activity_id.clone(),
        activity_type: task.activity_type.clone(),
        input: task.input.clone(),
        options: task.options.clone(),
    };
    append_event(store, workflow_id, event).await.map(|_| ())
}

/// [`TurnStore::count_pending_signals`] on a durable store.
pub(crate) async fn count_pending_signals_in<S: WorkflowEventStore>(
    store: &S,
    workflow_id: Uuid,
    signal_type: &str,
) -> Result<usize, StoreError> {
    Ok(SignalStore::get_pending_signals(store, workflow_id)
        .await?
        .iter()
        .filter(|signal| signal.signal_type == signal_type)
        .count())
}

/// [`TurnStore::enqueue_task_and_record`] into `queue`.
pub(crate) async fn enqueue_task_in<S: WorkflowEventStore>(
    store: &S,
    queue: Option<&str>,
    workflow_id: Uuid,
    activity_id: String,
    activity_type: String,
    input: serde_json::Value,
) -> Result<Uuid, StoreError> {
    let task = turn_task(queue, workflow_id, activity_id, activity_type, input);
    record_scheduled(store, &task).await?;
    TaskQueue::enqueue_task(store, task).await
}

/// [`TurnStore::enqueue_claimed_task_and_record`] into `queue`.
pub(crate) async fn enqueue_claimed_task_in<S: WorkflowEventStore>(
    store: &S,
    queue: Option<&str>,
    workflow_id: Uuid,
    activity_id: String,
    activity_type: String,
    input: serde_json::Value,
    worker_id: &str,
) -> Result<Option<ClaimedTask>, StoreError> {
    let task = turn_task(queue, workflow_id, activity_id, activity_type, input);
    record_scheduled(store, &task).await?;
    TaskQueue::enqueue_claimed_task(store, task, worker_id)
        .await
        .map(Enqueued::into_claimed)
}

/// [`TurnStore::start_turn`] with its first task in `queue`.
pub(crate) async fn start_turn_in<S: WorkflowEventStore>(
    store: &S,
    queue: Option<&str>,
    workflow_id: Uuid,
    workflow_type: &str,
    input: serde_json::Value,
    activity_id: String,
    activity_type: String,
) -> Result<RunStart, StoreError> {
    let task = turn_task(
        queue,
        workflow_id,
        activity_id,
        activity_type,
        input.clone(),
    );
    EventLog::start_run_with_task(store, workflow_id, workflow_type, input, task).await
}

#[async_trait]
impl<S> TurnStore for S
where
    S: WorkflowEventStore,
{
    async fn register_worker(&self, worker: WorkerInfo) -> Result<(), StoreError> {
        WorkerRegistry::register_worker(self, worker).await
    }

    async fn worker_heartbeat(
        &self,
        worker_id: &str,
        current_load: usize,
        accepting_tasks: bool,
    ) -> Result<WorkerHeartbeat, StoreError> {
        WorkerRegistry::worker_heartbeat(self, worker_id, current_load, accepting_tasks).await
    }

    async fn drain_worker(&self, worker_id: &str) -> Result<(), StoreError> {
        WorkerRegistry::drain_worker(self, worker_id).await
    }

    async fn deregister_worker(&self, worker_id: &str) -> Result<usize, StoreError> {
        WorkerRegistry::deregister_worker(self, worker_id).await
    }

    async fn claim_task(
        &self,
        worker_id: &str,
        activity_types: &[String],
        max_tasks: usize,
    ) -> Result<Vec<ClaimedTask>, StoreError> {
        TaskQueue::claim_task(self, worker_id, activity_types, max_tasks).await
    }

    async fn heartbeat_task(
        &self,
        task_id: Uuid,
        worker_id: &str,
        details: Option<serde_json::Value>,
    ) -> Result<HeartbeatResponse, StoreError> {
        TaskQueue::heartbeat_task(self, task_id, worker_id, details).await
    }

    async fn record_activity_started(&self, task: &ClaimedTask, worker_id: &str) {
        record_activity_started(
            self,
            task.workflow_id,
            task.activity_id.clone(),
            task.attempt,
            worker_id.to_string(),
        )
        .await;
    }

    async fn complete_task_and_record(
        &self,
        task: &ClaimedTask,
        worker_id: &str,
        output: serde_json::Value,
    ) -> Result<(), StoreError> {
        TaskQueue::complete_task(self, task.id, worker_id, output.clone()).await?;
        record_activity_completed(self, task.workflow_id, task.activity_id.clone(), output).await;
        Ok(())
    }

    async fn complete_task_and_hand_off(
        &self,
        task: &ClaimedTask,
        worker_id: &str,
        output: serde_json::Value,
        hand_off: TurnHandOff,
    ) -> Result<Option<ClaimedTask>, StoreError> {
        hand_off_and_record(
            self,
            None,
            task.id,
            &task.activity_id,
            worker_id,
            output,
            hand_off,
        )
        .await
    }

    async fn count_pending_signals(
        &self,
        workflow_id: Uuid,
        signal_type: &str,
    ) -> Result<usize, StoreError> {
        count_pending_signals_in(self, workflow_id, signal_type).await
    }

    async fn fail_task_and_record(
        &self,
        task: &ClaimedTask,
        error: &str,
        retryable: bool,
    ) -> Result<TaskFailureOutcome, StoreError> {
        let outcome = match TaskQueue::fail_task_with_retry(self, task.id, error, retryable).await {
            Ok(outcome) => outcome,
            Err(StoreError::TaskNotOwned(_)) => return Ok(TaskFailureOutcome::MovedToDlq),
            Err(error) => return Err(error),
        };
        let will_retry = matches!(outcome, TaskFailureOutcome::WillRetry { .. });
        record_activity_failed(
            self,
            task.workflow_id,
            task.activity_id.clone(),
            error.to_string(),
            will_retry,
        )
        .await;
        if matches!(outcome, TaskFailureOutcome::MovedToDlq)
            && let Some(workflow_id) = task.workflow_id
            && EventLog::try_fail_workflow(self, workflow_id, WorkflowError::new(error)).await?
        {
            record_workflow_failed(self, workflow_id, error.to_string()).await;
            return Ok(TaskFailureOutcome::ExhaustedRetries);
        }
        Ok(outcome)
    }

    async fn enqueue_task_and_record(
        &self,
        workflow_id: Uuid,
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
    ) -> Result<Uuid, StoreError> {
        enqueue_task_in(self, None, workflow_id, activity_id, activity_type, input).await
    }

    async fn enqueue_claimed_task_and_record(
        &self,
        workflow_id: Uuid,
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
        worker_id: &str,
    ) -> Result<Option<ClaimedTask>, StoreError> {
        enqueue_claimed_task_in(
            self,
            None,
            workflow_id,
            activity_id,
            activity_type,
            input,
            worker_id,
        )
        .await
    }

    async fn cancel_pending_tasks(&self, workflow_id: Uuid) -> Result<u64, StoreError> {
        TaskQueue::cancel_pending_tasks_for_workflow(self, workflow_id).await
    }

    async fn start_turn(
        &self,
        workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
        activity_id: String,
        activity_type: String,
    ) -> Result<RunStart, StoreError> {
        start_turn_in(
            self,
            None,
            workflow_id,
            workflow_type,
            input,
            activity_id,
            activity_type,
        )
        .await
    }

    async fn get_workflow(&self, workflow_id: Uuid) -> Result<WorkflowSnapshot, StoreError> {
        let info = EventLog::get_workflow_info(self, workflow_id).await?;
        Ok(WorkflowSnapshot {
            status: info.status,
            output: info.result,
            error: info.error.map(|error| error.message),
        })
    }

    async fn update_workflow_status(
        &self,
        workflow_id: Uuid,
        status: WorkflowStatus,
        output: Option<serde_json::Value>,
        error: Option<WorkflowError>,
    ) -> Result<(), StoreError> {
        EventLog::update_workflow_status(self, workflow_id, status, output, error).await
    }

    async fn complete_workflow(
        &self,
        workflow_id: Uuid,
        event_output: serde_json::Value,
        stored_output: Option<serde_json::Value>,
        error: Option<WorkflowError>,
    ) -> Result<(), StoreError> {
        crate::durable::record_workflow_completed(self, workflow_id, event_output).await;
        EventLog::update_workflow_status(
            self,
            workflow_id,
            WorkflowStatus::Completed,
            stored_output,
            error,
        )
        .await
    }

    async fn count_active_workflows(&self) -> Result<usize, StoreError> {
        DurableAdmin::count_active_workflows(self)
            .await
            .map(|count| usize::try_from(count).unwrap_or_default())
    }

    async fn latest_completion_output(
        &self,
        workflow_id: Uuid,
    ) -> Result<Option<serde_json::Value>, StoreError> {
        crate::turn_backend::latest_completion_output(self, workflow_id).await
    }

    /// Decision: only the memory store ends its workflows in this process
    /// alone, so only it can push ends. The generic `everruns-durable` store
    /// traits carry no such hook; a downcast keeps it out of them.
    fn workflow_end_signal(&self, workflow_id: Uuid) -> Option<WorkflowEndSignal> {
        let memory = (self as &dyn Any).downcast_ref::<InMemoryWorkflowEventStore>()?;
        Some(Box::pin(memory.subscribe_workflow_end(workflow_id).ended()))
    }

    async fn send_signal(
        &self,
        workflow_id: Uuid,
        signal: WorkflowSignal,
    ) -> Result<(), StoreError> {
        SignalStore::send_signal(self, workflow_id, signal).await
    }

    async fn consume_pending_signals(
        &self,
        workflow_id: Uuid,
    ) -> Result<Vec<WorkflowSignal>, StoreError> {
        SignalStore::consume_pending_signals(self, workflow_id).await
    }

    async fn consume_pending_signals_by_type(
        &self,
        workflow_id: Uuid,
        signal_type: &str,
    ) -> Result<Vec<WorkflowSignal>, StoreError> {
        SignalStore::consume_pending_signals_by_type(self, workflow_id, signal_type).await
    }
}
