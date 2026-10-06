//! The store a PostgreSQL [`DurableBackend`](crate::DurableBackend) runs its
//! runner and workers on, and how a shared queue routes a task to the
//! process that can run it.
//!
//! Execution behavior:
//! - **A task runs where its session is attached.** A framework turn step
//!   needs the session's `InProcessRuntime`, which lives only in the process
//!   that attached the session, so a worker of another process sharing the
//!   queue cannot run it. Each PostgreSQL backend therefore enqueues every
//!   task to a task queue of its own (`ActivityOptions::queue`, the task
//!   row's `queue` column) and claims from that queue only. Tasks keep their
//!   plain activity type (`reason`), and the server's workers, which claim
//!   from the default queue, never see them.
//! - **The queue is one per backend instance**, named when the backend is
//!   built: a process's tasks are exactly the ones its own sessions started.
//!   A queue shared by processes would hand one process a task for a session
//!   only another one attached; sharing becomes useful only once any process
//!   can build a session's runtime from configuration, which the framework
//!   does not do.
//! - **Workflow ends wake tickets locally.** Every workflow a routed backend
//!   starts ends in its own process (driver completion, task failure,
//!   cancellation, recovery), so status writes through this store, which the
//!   backend's runner and workers share, wake the turn's ticket at once, the
//!   way the memory store's own end signal does, instead of a 50 ms status
//!   poll.
//! - The PostgreSQL claim records `ActivityStarted` itself, so the routed
//!   store records nothing more when a step starts.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use everruns_durable::{
    ClaimedTask, HeartbeatResponse, RunStart, StoreError, TaskFailureOutcome, TaskQueue,
    WorkerInfo, WorkflowError, WorkflowEventStore, WorkflowSignal, WorkflowStatus,
};
use tokio::sync::watch;
use uuid::Uuid;

use crate::turn_store::{
    TurnHandOff, TurnNext, TurnStore, WorkflowEndSignal, WorkflowSnapshot, enqueue_claimed_task_in,
    enqueue_task_in, hand_off_and_record, start_turn_in,
};

/// One backend's share of a shared store: the task queue its tasks go to,
/// and the wakeups for the workflows they belong to.
#[derive(Clone)]
pub(crate) struct Routing {
    queue: Arc<str>,
    ends: Arc<LocalEnds>,
}

impl Routing {
    /// A task queue no other backend instance uses.
    pub(crate) fn unique() -> Self {
        Self {
            queue: Arc::from(format!("fw-{}", Uuid::now_v7().simple())),
            ends: Arc::default(),
        }
    }

    pub(crate) fn queue(&self) -> &str {
        &self.queue
    }
}

/// Wakeups for the workflow ends this process writes; see the module notes.
///
/// One sender per workflow someone waits on. Ending the workflow drops it,
/// which wakes every receiver; a woken ticket re-reads the status, so a
/// wakeup for a status write that ended nothing costs one read.
#[derive(Default)]
struct LocalEnds(Mutex<HashMap<Uuid, watch::Sender<()>>>);

impl LocalEnds {
    fn subscribe(&self, workflow_id: Uuid) -> WorkflowEndSignal {
        let mut receiver = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(workflow_id)
            .or_insert_with(|| watch::channel(()).0)
            .subscribe();
        Box::pin(async move {
            // The sender never sends; its drop is the wakeup.
            let _ = receiver.changed().await;
        })
    }

    fn notify(&self, workflow_id: Uuid) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&workflow_id);
    }
}

/// A shared store routed to one backend: its runner and workers enqueue to
/// and claim from the backend's own task queue, and the workflow ends they
/// write wake that backend's tickets.
pub(crate) struct RoutedStore<S> {
    store: Arc<S>,
    routing: Routing,
}

impl<S> RoutedStore<S> {
    pub(crate) fn new(store: Arc<S>, routing: Routing) -> Self {
        Self { store, routing }
    }

    fn ended(&self, workflow_id: Option<Uuid>) {
        if let Some(workflow_id) = workflow_id {
            self.routing.ends.notify(workflow_id);
        }
    }
}

// Calls name `TurnStore::` or `TaskQueue::` explicitly: `S` has both, with
// methods of the same names.
#[async_trait]
impl<S: WorkflowEventStore> TurnStore for RoutedStore<S> {
    async fn register_worker(&self, worker: WorkerInfo) -> Result<(), StoreError> {
        TurnStore::register_worker(&*self.store, worker).await
    }

    async fn worker_heartbeat(
        &self,
        worker_id: &str,
        current_load: usize,
        accepting_tasks: bool,
    ) -> Result<(), StoreError> {
        TurnStore::worker_heartbeat(&*self.store, worker_id, current_load, accepting_tasks).await
    }

    async fn deregister_worker(&self, worker_id: &str) -> Result<usize, StoreError> {
        TurnStore::deregister_worker(&*self.store, worker_id).await
    }

    async fn claim_task(
        &self,
        worker_id: &str,
        activity_types: &[String],
        max_tasks: usize,
    ) -> Result<Vec<ClaimedTask>, StoreError> {
        TaskQueue::claim_queue_tasks(
            &*self.store,
            worker_id,
            Some(self.routing.queue()),
            activity_types,
            max_tasks,
        )
        .await
    }

    async fn heartbeat_task(
        &self,
        task_id: Uuid,
        worker_id: &str,
        details: Option<serde_json::Value>,
    ) -> Result<HeartbeatResponse, StoreError> {
        TurnStore::heartbeat_task(&*self.store, task_id, worker_id, details).await
    }

    async fn record_activity_started(&self, _task: &ClaimedTask, _worker_id: &str) {
        // The shared (PostgreSQL) claim already recorded it.
    }

    async fn complete_task_and_record(
        &self,
        task: &ClaimedTask,
        worker_id: &str,
        output: serde_json::Value,
    ) -> Result<(), StoreError> {
        TurnStore::complete_task_and_record(&*self.store, task, worker_id, output).await
    }

    async fn complete_task_and_hand_off(
        &self,
        task: &ClaimedTask,
        worker_id: &str,
        output: serde_json::Value,
        hand_off: TurnHandOff,
    ) -> Result<Option<ClaimedTask>, StoreError> {
        let workflow_id = hand_off.workflow_id;
        let ends = matches!(hand_off.next, TurnNext::Complete { .. });
        let result = hand_off_and_record(
            &*self.store,
            Some(self.routing.queue()),
            task.id,
            &task.activity_id,
            worker_id,
            output,
            hand_off,
        )
        .await;
        if ends {
            self.ended(Some(workflow_id));
        }
        result
    }

    async fn count_pending_signals(
        &self,
        workflow_id: Uuid,
        signal_type: &str,
    ) -> Result<usize, StoreError> {
        TurnStore::count_pending_signals(&*self.store, workflow_id, signal_type).await
    }

    async fn fail_task_and_record(
        &self,
        task: &ClaimedTask,
        error: &str,
        retryable: bool,
    ) -> Result<TaskFailureOutcome, StoreError> {
        let outcome = TurnStore::fail_task_and_record(&*self.store, task, error, retryable).await;
        if !matches!(outcome, Ok(TaskFailureOutcome::WillRetry { .. })) {
            self.ended(task.workflow_id);
        }
        outcome
    }

    async fn enqueue_task_and_record(
        &self,
        workflow_id: Uuid,
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
    ) -> Result<Uuid, StoreError> {
        enqueue_task_in(
            &*self.store,
            Some(self.routing.queue()),
            workflow_id,
            activity_id,
            activity_type,
            input,
        )
        .await
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
            &*self.store,
            Some(self.routing.queue()),
            workflow_id,
            activity_id,
            activity_type,
            input,
            worker_id,
        )
        .await
    }

    async fn cancel_pending_tasks(&self, workflow_id: Uuid) -> Result<u64, StoreError> {
        TurnStore::cancel_pending_tasks(&*self.store, workflow_id).await
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
            &*self.store,
            Some(self.routing.queue()),
            workflow_id,
            workflow_type,
            input,
            activity_id,
            activity_type,
        )
        .await
    }

    async fn get_workflow(&self, workflow_id: Uuid) -> Result<WorkflowSnapshot, StoreError> {
        TurnStore::get_workflow(&*self.store, workflow_id).await
    }

    async fn update_workflow_status(
        &self,
        workflow_id: Uuid,
        status: WorkflowStatus,
        output: Option<serde_json::Value>,
        error: Option<WorkflowError>,
    ) -> Result<(), StoreError> {
        let result =
            TurnStore::update_workflow_status(&*self.store, workflow_id, status, output, error)
                .await;
        if status.is_terminal() {
            self.ended(Some(workflow_id));
        }
        result
    }

    async fn complete_workflow(
        &self,
        workflow_id: Uuid,
        event_output: serde_json::Value,
        stored_output: Option<serde_json::Value>,
        error: Option<WorkflowError>,
    ) -> Result<(), StoreError> {
        let result = TurnStore::complete_workflow(
            &*self.store,
            workflow_id,
            event_output,
            stored_output,
            error,
        )
        .await;
        self.ended(Some(workflow_id));
        result
    }

    async fn count_active_workflows(&self) -> Result<usize, StoreError> {
        TurnStore::count_active_workflows(&*self.store).await
    }

    async fn latest_completion_output(
        &self,
        workflow_id: Uuid,
    ) -> Result<Option<serde_json::Value>, StoreError> {
        TurnStore::latest_completion_output(&*self.store, workflow_id).await
    }

    fn workflow_end_signal(&self, workflow_id: Uuid) -> Option<WorkflowEndSignal> {
        Some(self.routing.ends.subscribe(workflow_id))
    }

    async fn send_signal(
        &self,
        workflow_id: Uuid,
        signal: WorkflowSignal,
    ) -> Result<(), StoreError> {
        TurnStore::send_signal(&*self.store, workflow_id, signal).await
    }

    async fn consume_pending_signals(
        &self,
        workflow_id: Uuid,
    ) -> Result<Vec<WorkflowSignal>, StoreError> {
        TurnStore::consume_pending_signals(&*self.store, workflow_id).await
    }

    async fn consume_pending_signals_by_type(
        &self,
        workflow_id: Uuid,
        signal_type: &str,
    ) -> Result<Vec<WorkflowSignal>, StoreError> {
        TurnStore::consume_pending_signals_by_type(&*self.store, workflow_id, signal_type).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use everruns_durable::{
        EventLog, InMemoryWorkflowEventStore, TaskDefinition, TaskQueue, WorkerRegistry,
    };

    #[test]
    fn every_backend_gets_a_queue_of_its_own() {
        assert_ne!(Routing::unique().queue(), Routing::unique().queue());
    }

    #[tokio::test]
    async fn a_routed_store_and_the_default_queue_never_claim_each_others_tasks() {
        let shared = Arc::new(InMemoryWorkflowEventStore::new());
        let routed = RoutedStore::new(shared.clone(), Routing::unique());
        let types = ["reason".to_string()];
        for worker in ["routed", "default"] {
            WorkerRegistry::register_worker(
                &*shared,
                everruns_durable::WorkerInfo::new(worker, types.to_vec()),
            )
            .await
            .unwrap();
        }
        let workflow_id = Uuid::now_v7();
        EventLog::create_workflow(&*shared, workflow_id, "turn_workflow", json!({}), None)
            .await
            .unwrap();
        // A task of the default queue, as the server's workers enqueue them.
        TaskQueue::enqueue_task(
            &*shared,
            TaskDefinition {
                workflow_id: Some(workflow_id),
                activity_id: "reason-default".into(),
                activity_type: "reason".into(),
                input: json!({}),
                options: Default::default(),
            },
        )
        .await
        .unwrap();
        routed
            .enqueue_task_and_record(
                workflow_id,
                "reason-routed".into(),
                "reason".into(),
                json!({}),
            )
            .await
            .unwrap();

        let mine = routed.claim_task("routed", &types, 10).await.unwrap();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].activity_id, "reason-routed");
        // The step keeps its plain type; only the queue routes it.
        assert_eq!(mine[0].activity_type, "reason");
        let theirs = TaskQueue::claim_task(&*shared, "default", &types, 10)
            .await
            .unwrap();
        assert_eq!(theirs.len(), 1);
        assert_eq!(theirs[0].activity_id, "reason-default");
    }

    #[tokio::test]
    async fn a_local_end_wakes_every_waiter_of_that_workflow_only() {
        let ends = LocalEnds::default();
        let workflow_id = Uuid::now_v7();
        let first = ends.subscribe(workflow_id);
        let second = ends.subscribe(workflow_id);
        let other = ends.subscribe(Uuid::now_v7());
        ends.notify(workflow_id);
        first.await;
        second.await;
        let pending = tokio::time::timeout(std::time::Duration::ZERO, other).await;
        assert!(pending.is_err(), "another workflow's waiter stays asleep");
    }
}
