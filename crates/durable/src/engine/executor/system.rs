//! System tasks: durable timers and child workflows.
//!
//! Decision: timers and child workflows ride the ordinary task queue instead of
//! a separate timer service or an in-process call chain. A timer is a task that
//! becomes claimable when it is due (`ActivityOptions::start_delay`); starting a
//! child and reporting its outcome to the parent are tasks too. That gives them
//! the queue's guarantees for free: they survive a crash, a failed attempt is
//! retried with backoff, and they show up in the same task listings. It also
//! keeps the executor from recursing into a parent while the parent's own
//! actions are still being applied, which would race its optimistic sequence.
//!
//! The work is at-least-once, so every handler is idempotent: a timer fires
//! once per `StartTimer`, a child's id is derived from its parent and the
//! parent's id for it, and a parent hears about each child once.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{debug, warn};
use uuid::Uuid;

use super::{ExecutorError, WorkflowExecutor};
use crate::persistence::{ClaimedTask, StoreError, TaskDefinition, WorkflowEventStore};
use crate::workflow::{ActivityOptions, ParentWorkflow, WorkflowError, WorkflowEvent};

/// Fires a timer when its task becomes visible.
const TIMER: &str = "durable.timer";
/// Creates a child workflow.
const START_CHILD: &str = "durable.start_child";
/// Reports a finished child workflow to its parent.
const CHILD_RESULT: &str = "durable.child_result";

/// Activity types of the engine's own tasks.
///
/// Register a worker for these and call
/// [`WorkflowExecutor::run_system_tasks`] in a loop, or timers never fire and
/// child workflows never start. See that method for an example.
pub const SYSTEM_ACTIVITY_TYPES: [&str; 3] = [TIMER, START_CHILD, CHILD_RESULT];

/// The UUID of the child a parent knows as `child_id`.
///
/// Derived, not random, so a retried start finds the child it already made.
pub(super) fn child_workflow_id(parent: Uuid, child_id: &str) -> Uuid {
    Uuid::new_v5(&parent, child_id.as_bytes())
}

pub(super) fn timer_task(
    workflow_id: Uuid,
    timer_id: String,
    duration: std::time::Duration,
) -> TaskDefinition {
    TaskDefinition {
        workflow_id: Some(workflow_id),
        activity_id: timer_id,
        activity_type: TIMER.to_string(),
        input: Value::Null,
        options: ActivityOptions::default().with_start_delay(duration),
    }
}

/// Input of a [`START_CHILD`] task.
#[derive(Serialize, Deserialize)]
pub(super) struct StartChild {
    pub child_id: String,
    pub workflow_type: String,
    pub input: Value,
}

impl StartChild {
    pub fn task(&self, parent: Uuid, child: Uuid) -> Result<TaskDefinition, ExecutorError> {
        Ok(TaskDefinition {
            workflow_id: Some(parent),
            activity_id: format!("start_child:{child}"),
            activity_type: START_CHILD.to_string(),
            input: serde_json::to_value(self)?,
            options: ActivityOptions::default(),
        })
    }
}

/// Input of a [`CHILD_RESULT`] task.
#[derive(Serialize, Deserialize)]
struct ChildResult {
    child_workflow_id: Uuid,
    child_id: String,
    outcome: Result<Value, WorkflowError>,
}

impl<S: WorkflowEventStore> WorkflowExecutor<S> {
    /// Run due timers, start scheduled child workflows, and report finished
    /// children to their parents. Returns how many system tasks were handled.
    ///
    /// `worker_id` must be registered for [`SYSTEM_ACTIVITY_TYPES`]. A task
    /// that fails is retried with its retry policy, like any activity.
    ///
    /// ```
    /// use std::time::Duration;
    /// use everruns_durable::prelude::*;
    /// use everruns_durable::SYSTEM_ACTIVITY_TYPES;
    /// use serde_json::{Value, json};
    ///
    /// /// Waits a moment, then completes.
    /// struct Nap {
    ///     woke: bool,
    /// }
    ///
    /// impl Workflow for Nap {
    ///     const TYPE: &'static str = "nap";
    ///     type Input = Value;
    ///     type Output = Value;
    ///
    ///     fn new(_: Value) -> Self {
    ///         Self { woke: false }
    ///     }
    ///     fn on_start(&mut self) -> Vec<WorkflowAction> {
    ///         vec![WorkflowAction::timer("nap", Duration::from_millis(10))]
    ///     }
    ///     fn on_timer_fired(&mut self, _: &str) -> Vec<WorkflowAction> {
    ///         self.woke = true;
    ///         vec![WorkflowAction::complete(json!("rested"))]
    ///     }
    ///     fn on_activity_completed(&mut self, _: &str, _: Value) -> Vec<WorkflowAction> {
    ///         vec![]
    ///     }
    ///     fn on_activity_failed(&mut self, _: &str, _: &ActivityError) -> Vec<WorkflowAction> {
    ///         vec![]
    ///     }
    ///     fn is_completed(&self) -> bool {
    ///         self.woke
    ///     }
    ///     fn result(&self) -> Option<Value> {
    ///         self.woke.then(|| json!("rested"))
    ///     }
    /// }
    ///
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() -> Result<(), ExecutorError> {
    /// let mut executor = WorkflowExecutor::new(InMemoryWorkflowEventStore::new());
    /// executor.register::<Nap>();
    /// let worker = WorkerInfo::new("engine", SYSTEM_ACTIVITY_TYPES);
    /// executor.store().register_worker(worker).await?;
    ///
    /// let id = executor.start_workflow::<Nap>(json!({}), None).await?;
    /// assert_eq!(executor.run_system_tasks("engine", 10).await?, 0, "not due yet");
    ///
    /// tokio::time::sleep(Duration::from_millis(20)).await;
    /// assert_eq!(executor.run_system_tasks("engine", 10).await?, 1);
    /// assert_eq!(executor.store().get_workflow_status(id).await?, WorkflowStatus::Completed);
    /// # Ok(()) }
    /// ```
    pub async fn run_system_tasks(
        &self,
        worker_id: &str,
        max_tasks: usize,
    ) -> Result<usize, ExecutorError> {
        let types = SYSTEM_ACTIVITY_TYPES.map(String::from);
        let tasks = self.store.claim_task(worker_id, &types, max_tasks).await?;
        let handled = tasks.len();
        for task in tasks {
            match self.run_system_task(&task).await {
                Ok(()) => {
                    self.store
                        .complete_task(task.id, worker_id, Value::Null)
                        .await?
                }
                Err(error) => {
                    warn!(task_id = %task.id, activity_type = %task.activity_type, %error, "system task failed");
                    self.store.fail_task(task.id, &error.to_string()).await?;
                }
            }
        }
        Ok(handled)
    }

    async fn run_system_task(&self, task: &ClaimedTask) -> Result<(), ExecutorError> {
        let workflow_id = task.workflow_id.ok_or_else(|| {
            ExecutorError::InvalidAction(format!("system task {} has no workflow", task.id))
        })?;
        match task.activity_type.as_str() {
            TIMER => self.fire_timer(workflow_id, &task.activity_id).await,
            START_CHILD => {
                let start: StartChild = serde_json::from_value(task.input.clone())?;
                self.start_child(workflow_id, start).await
            }
            CHILD_RESULT => {
                let result: ChildResult = serde_json::from_value(task.input.clone())?;
                self.deliver_child_result(workflow_id, result).await
            }
            other => Err(ExecutorError::InvalidAction(format!(
                "unknown system task type {other}"
            ))),
        }
    }

    /// Fire a timer once for each time the workflow started it.
    async fn fire_timer(&self, workflow_id: Uuid, timer_id: &str) -> Result<(), ExecutorError> {
        if self.is_terminal(workflow_id).await? {
            return Ok(());
        }
        let (mut started, mut fired) = (0, 0);
        for (_, event) in self.store.load_events(workflow_id).await? {
            match event {
                WorkflowEvent::TimerStarted { timer_id: id, .. } if id == timer_id => started += 1,
                WorkflowEvent::TimerFired { timer_id: id } if id == timer_id => fired += 1,
                _ => {}
            }
        }
        if fired >= started {
            debug!(%workflow_id, %timer_id, "timer already fired");
            return Ok(());
        }
        self.on_timer_fired(workflow_id, timer_id).await?;
        Ok(())
    }

    async fn start_child(&self, parent: Uuid, start: StartChild) -> Result<(), ExecutorError> {
        let child = child_workflow_id(parent, &start.child_id);
        match self.store.get_workflow_status(child).await {
            Err(StoreError::WorkflowNotFound(_)) => {}
            Ok(_) => {
                debug!(%parent, %child, "child workflow already started");
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        }
        let workflow = match self
            .registry
            .create(&start.workflow_type, start.input.clone())
        {
            Ok(workflow) => workflow,
            // Retrying cannot register the type, so the parent hears now.
            Err(error) => {
                let result = ChildResult {
                    child_workflow_id: child,
                    child_id: start.child_id,
                    outcome: Err(
                        WorkflowError::new(error.to_string()).with_code("child_not_started")
                    ),
                };
                return self.deliver_child_result(parent, result).await;
            }
        };
        let link = ParentWorkflow {
            workflow_id: parent,
            child_id: start.child_id,
        };
        self.start_instance(child, workflow, start.input, None, Some(link))
            .await
    }

    /// Append a child's outcome to its parent once, then advance the parent.
    async fn deliver_child_result(
        &self,
        parent: Uuid,
        result: ChildResult,
    ) -> Result<(), ExecutorError> {
        if self.is_terminal(parent).await? {
            return Ok(());
        }
        let events = self.store.load_events(parent).await?;
        let delivered = events.iter().any(|(_, event)| {
            matches!(event,
                WorkflowEvent::ChildWorkflowCompleted { workflow_id, .. }
                | WorkflowEvent::ChildWorkflowFailed { workflow_id, .. }
                    if *workflow_id == result.child_workflow_id)
        });
        if delivered {
            return Ok(());
        }
        let (workflow_id, child_id) = (result.child_workflow_id, result.child_id);
        let event = match result.outcome {
            Ok(result) => WorkflowEvent::ChildWorkflowCompleted {
                workflow_id,
                child_id,
                result,
            },
            Err(error) => WorkflowEvent::ChildWorkflowFailed {
                workflow_id,
                child_id,
                error,
            },
        };
        self.store
            .append_events(parent, events.len() as i32, vec![event])
            .await?;
        self.process_workflow(parent).await?;
        Ok(())
    }

    /// Queue a finished workflow's outcome for its parent, if it has one.
    /// Returns the number of tasks enqueued.
    pub(super) async fn report_to_parent(
        &self,
        workflow_id: Uuid,
        outcome: Result<Value, WorkflowError>,
    ) -> Result<usize, ExecutorError> {
        let events = self.store.load_events(workflow_id).await?;
        let Some((
            _,
            WorkflowEvent::WorkflowStarted {
                parent: Some(parent),
                ..
            },
        )) = events.into_iter().next()
        else {
            return Ok(0);
        };
        let result = ChildResult {
            child_workflow_id: workflow_id,
            child_id: parent.child_id,
            outcome,
        };
        self.store
            .enqueue_task(TaskDefinition {
                workflow_id: Some(parent.workflow_id),
                activity_id: format!("child_result:{workflow_id}"),
                activity_type: CHILD_RESULT.to_string(),
                input: serde_json::to_value(&result)?,
                options: ActivityOptions::default(),
            })
            .await?;
        Ok(1)
    }

    async fn is_terminal(&self, workflow_id: Uuid) -> Result<bool, ExecutorError> {
        let status = self.store.get_workflow_status(workflow_id).await?;
        Ok(status.is_terminal())
    }
}
