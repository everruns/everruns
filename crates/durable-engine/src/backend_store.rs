//! The store a [`DurableBackend`](crate::DurableBackend) runs its workers and
//! its runner on, and how a shared PostgreSQL queue routes a task to the
//! process that can run it.
//!
//! Execution behavior:
//! - **A task runs where its session is attached.** A framework turn step
//!   needs the session's `InProcessRuntime`, which lives only in the process
//!   that attached the session, so a worker of another process sharing the
//!   queue cannot run it. Each PostgreSQL backend therefore claims only the
//!   tasks it enqueued: it tags every task's activity type with its routing
//!   key (`reason@<key>`) and claims only tagged types. The claim API already
//!   filters by activity type, so routing needs no schema change, and the
//!   driver still sees the plain type (`reason`), because this store strips
//!   the tag from every claimed task.
//! - **The routing key is one per backend instance**, generated when the
//!   backend is built: a process's tasks are exactly the ones its own
//!   sessions started. A key shared by processes would hand one process a
//!   task for a session only another one attached; sharing becomes useful
//!   only once any process can build a session's runtime from configuration,
//!   which the framework does not do.
//! - **Workflow ends wake tickets locally.** Every workflow a routed backend
//!   starts ends in its own process (driver completion, task failure,
//!   cancellation, recovery), so status writes through this store and through
//!   [`RoutedDurableStore`] wake the turn's ticket at once, the way the memory
//!   store's own end signal does, instead of a 50 ms status poll.
//! - The PostgreSQL claim records `ActivityStarted` itself, so the routed
//!   store records nothing more when a step starts.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use anyhow::Result as AnyResult;
use async_trait::async_trait;
use everruns_durable::{
    ClaimedTask, HeartbeatResponse, RunStart, StoreError, TaskFailureOutcome, WorkerInfo,
    WorkflowError, WorkflowEvent, WorkflowSignal, WorkflowStatus,
};
use tokio::sync::watch;
use uuid::Uuid;

use crate::durable_runner::{DurableStoreBackend, WorkflowEndSignal};
use crate::task_store::{TaskStore, TaskWakeups};

/// Separates an activity type from the routing key it is tagged with.
const ROUTE_SEPARATOR: char = '@';

/// One backend's share of a shared queue: the key its tasks are tagged with,
/// and the wakeups for the workflows they belong to.
#[derive(Clone)]
pub(crate) struct Routing {
    key: Arc<str>,
    ends: Arc<LocalEnds>,
}

impl Routing {
    /// A routing key no other backend instance uses.
    pub(crate) fn unique() -> Self {
        Self {
            key: Arc::from(format!("fw-{}", Uuid::now_v7().simple())),
            ends: Arc::default(),
        }
    }

    pub(crate) fn key(&self) -> &str {
        &self.key
    }

    /// `activity_type` tagged with this routing key.
    fn route(&self, activity_type: &str) -> String {
        format!("{activity_type}{ROUTE_SEPARATOR}{}", self.key)
    }
}

/// `activity_type` without its routing tag.
fn unroute(activity_type: &str) -> &str {
    activity_type
        .split_once(ROUTE_SEPARATOR)
        .map_or(activity_type, |(plain, _)| plain)
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

/// The task store a backend's workers and drivers use: the backend's store,
/// routed when the queue is shared.
pub(crate) struct BackendTaskStore {
    inner: Arc<dyn TaskStore>,
    routing: Option<Routing>,
}

impl BackendTaskStore {
    /// `inner` as is: the backend owns the whole queue.
    pub(crate) fn owned(inner: Arc<dyn TaskStore>) -> Self {
        Self {
            inner,
            routing: None,
        }
    }

    /// `inner`, shared with other backends, of which this one claims only
    /// the tasks `routing` tags.
    pub(crate) fn routed(inner: Arc<dyn TaskStore>, routing: Routing) -> Self {
        Self {
            inner,
            routing: Some(routing),
        }
    }

    fn ended(&self, workflow_id: Option<Uuid>) {
        if let (Some(routing), Some(workflow_id)) = (&self.routing, workflow_id) {
            routing.ends.notify(workflow_id);
        }
    }
}

#[async_trait]
impl TaskStore for BackendTaskStore {
    async fn register_worker(&self, worker: WorkerInfo) -> Result<(), StoreError> {
        self.inner.register_worker(worker).await
    }

    async fn worker_heartbeat(
        &self,
        worker_id: &str,
        current_load: usize,
        accepting_tasks: bool,
    ) -> Result<(), StoreError> {
        self.inner
            .worker_heartbeat(worker_id, current_load, accepting_tasks)
            .await
    }

    async fn deregister_worker(&self, worker_id: &str) -> Result<usize, StoreError> {
        self.inner.deregister_worker(worker_id).await
    }

    async fn claim_task(
        &self,
        worker_id: &str,
        activity_types: &[String],
        max_tasks: usize,
    ) -> Result<Vec<ClaimedTask>, StoreError> {
        let Some(routing) = &self.routing else {
            return self
                .inner
                .claim_task(worker_id, activity_types, max_tasks)
                .await;
        };
        let routed: Vec<String> = activity_types
            .iter()
            .map(|activity_type| routing.route(activity_type))
            .collect();
        let mut tasks = self.inner.claim_task(worker_id, &routed, max_tasks).await?;
        for task in &mut tasks {
            task.activity_type = unroute(&task.activity_type).to_string();
        }
        Ok(tasks)
    }

    async fn heartbeat_task(
        &self,
        task_id: Uuid,
        worker_id: &str,
        details: Option<serde_json::Value>,
    ) -> Result<HeartbeatResponse, StoreError> {
        self.inner.heartbeat_task(task_id, worker_id, details).await
    }

    async fn get_workflow_status(&self, workflow_id: Uuid) -> Result<WorkflowStatus, StoreError> {
        self.inner.get_workflow_status(workflow_id).await
    }

    async fn record_activity_started(&self, task: &ClaimedTask, worker_id: &str) {
        // The shared (PostgreSQL) claim already recorded it.
        if self.routing.is_none() {
            self.inner.record_activity_started(task, worker_id).await;
        }
    }

    async fn complete_task_and_record(
        &self,
        task: &ClaimedTask,
        worker_id: &str,
        output: serde_json::Value,
    ) -> Result<(), StoreError> {
        self.inner
            .complete_task_and_record(task, worker_id, output)
            .await
    }

    async fn complete_task_and_drain(
        &self,
        task: &ClaimedTask,
        worker_id: &str,
        output: serde_json::Value,
        drain: Option<&str>,
    ) -> Result<Option<usize>, StoreError> {
        self.inner
            .complete_task_and_drain(task, worker_id, output, drain)
            .await
    }

    async fn fail_task_and_record(
        &self,
        task: &ClaimedTask,
        error: &str,
        retryable: bool,
    ) -> Result<TaskFailureOutcome, StoreError> {
        let outcome = self
            .inner
            .fail_task_and_record(task, error, retryable)
            .await;
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
        let activity_type = match &self.routing {
            Some(routing) => routing.route(&activity_type),
            None => activity_type,
        };
        self.inner
            .enqueue_task_and_record(workflow_id, activity_id, activity_type, input)
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
        let Some(routing) = &self.routing else {
            return self
                .inner
                .enqueue_claimed_task_and_record(
                    workflow_id,
                    activity_id,
                    activity_type,
                    input,
                    worker_id,
                )
                .await;
        };
        let mut claimed = self
            .inner
            .enqueue_claimed_task_and_record(
                workflow_id,
                activity_id,
                routing.route(&activity_type),
                input,
                worker_id,
            )
            .await?;
        if let Some(task) = &mut claimed {
            task.activity_type = unroute(&task.activity_type).to_string();
        }
        Ok(claimed)
    }

    async fn update_workflow_status(
        &self,
        workflow_id: Uuid,
        status: WorkflowStatus,
        output: Option<serde_json::Value>,
        error: Option<WorkflowError>,
    ) -> Result<(), StoreError> {
        let result = self
            .inner
            .update_workflow_status(workflow_id, status, output, error)
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
        let result = self
            .inner
            .complete_workflow(workflow_id, event_output, stored_output, error)
            .await;
        self.ended(Some(workflow_id));
        result
    }

    async fn consume_pending_signals(
        &self,
        workflow_id: Uuid,
    ) -> Result<Vec<WorkflowSignal>, StoreError> {
        self.inner.consume_pending_signals(workflow_id).await
    }

    async fn consume_pending_signals_by_type(
        &self,
        workflow_id: Uuid,
        signal_type: &str,
    ) -> Result<Vec<WorkflowSignal>, StoreError> {
        self.inner
            .consume_pending_signals_by_type(workflow_id, signal_type)
            .await
    }

    async fn subscribe_task_wakeups(
        &self,
        worker_id: &str,
        activity_types: &[String],
    ) -> Result<Option<TaskWakeups>, StoreError> {
        self.inner
            .subscribe_task_wakeups(worker_id, activity_types)
            .await
    }
}

/// The runner's side of a routed backend: tasks it enqueues carry the
/// routing key, and the workflow ends it writes wake tickets.
pub(crate) struct RoutedDurableStore<B> {
    inner: B,
    routing: Routing,
}

impl<B> RoutedDurableStore<B> {
    pub(crate) fn new(inner: B, routing: Routing) -> Self {
        Self { inner, routing }
    }
}

#[async_trait]
impl<B: DurableStoreBackend> DurableStoreBackend for RoutedDurableStore<B> {
    async fn get_workflow_status(
        &self,
        workflow_id: Uuid,
    ) -> AnyResult<(WorkflowStatus, Option<serde_json::Value>, Option<String>)> {
        self.inner.get_workflow_status(workflow_id).await
    }

    async fn create_workflow(
        &self,
        workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
    ) -> AnyResult<Uuid> {
        self.inner
            .create_workflow(workflow_id, workflow_type, input)
            .await
    }

    async fn update_workflow_status(
        &self,
        workflow_id: Uuid,
        status: WorkflowStatus,
        output: Option<serde_json::Value>,
        error: Option<String>,
    ) -> AnyResult<()> {
        let result = self
            .inner
            .update_workflow_status(workflow_id, status, output, error)
            .await;
        if status.is_terminal() {
            self.routing.ends.notify(workflow_id);
        }
        result
    }

    async fn enqueue_task(
        &self,
        workflow_id: Uuid,
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
    ) -> AnyResult<Uuid> {
        let activity_type = self.routing.route(&activity_type);
        self.inner
            .enqueue_task(workflow_id, activity_id, activity_type, input)
            .await
    }

    async fn start_turn(
        &self,
        workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
        activity_id: String,
        activity_type: String,
    ) -> AnyResult<RunStart> {
        let activity_type = self.routing.route(&activity_type);
        self.inner
            .start_turn(
                workflow_id,
                workflow_type,
                input,
                activity_id,
                activity_type,
            )
            .await
    }

    async fn count_active_workflows(&self) -> AnyResult<usize> {
        self.inner.count_active_workflows().await
    }

    async fn cancel_pending_tasks(&self, workflow_id: Uuid) -> AnyResult<u64> {
        self.inner.cancel_pending_tasks(workflow_id).await
    }

    async fn append_events(
        &self,
        workflow_id: Uuid,
        expected_sequence: i32,
        events: Vec<WorkflowEvent>,
    ) -> AnyResult<i32> {
        self.inner
            .append_events(workflow_id, expected_sequence, events)
            .await
    }

    async fn send_signal(&self, workflow_id: Uuid, signal: WorkflowSignal) -> AnyResult<()> {
        self.inner.send_signal(workflow_id, signal).await
    }

    async fn get_and_consume_signals(&self, workflow_id: Uuid) -> AnyResult<Vec<WorkflowSignal>> {
        self.inner.get_and_consume_signals(workflow_id).await
    }

    async fn latest_completion_output(
        &self,
        workflow_id: Uuid,
    ) -> AnyResult<Option<serde_json::Value>> {
        self.inner.latest_completion_output(workflow_id).await
    }

    fn workflow_end_signal(&self, workflow_id: Uuid) -> Option<WorkflowEndSignal> {
        Some(self.routing.ends.subscribe(workflow_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_routed_type_carries_the_key_and_unroutes_to_the_plain_type() {
        let routing = Routing::unique();
        let routed = routing.route("reason");
        assert_eq!(routed, format!("reason@{}", routing.key()));
        assert_eq!(unroute(&routed), "reason");
        assert_eq!(unroute("reason"), "reason");
        assert_ne!(Routing::unique().key(), routing.key());
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
