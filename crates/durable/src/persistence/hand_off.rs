//! Step hand-off and stranded-workflow recovery records for [`TaskQueue`].
//!
//! Decision: a workflow that runs as a chain of queued tasks moves from one
//! step to the next with one store write,
//! [`TaskQueue::complete_task_and_hand_off`]: the step's task completes, the
//! signals its successor was planned from are consumed, and the successor is
//! enqueued (or the workflow completes), all or nothing. Done as separate
//! writes, a process exit or a lost reply between them left the workflow
//! running with no task, and a drained signal with nothing to act on it.
//!
//! Only the queue rows move together (task, signals, workflow status). The
//! workflow's event history stays outside the transaction: it is appended by
//! the caller, around the hand-off, as before.
//!
//! [`TaskQueue::requeue_stranded_workflows`] is the safety net for a workflow
//! that still ends up running with no task (a store or client that hands off
//! in separate writes, or a run stranded before this existed).
//!
//! [`TaskQueue`]: crate::TaskQueue
//! [`TaskQueue::complete_task_and_hand_off`]: crate::TaskQueue::complete_task_and_hand_off
//! [`TaskQueue::requeue_stranded_workflows`]: crate::TaskQueue::requeue_stranded_workflows

use uuid::Uuid;

use super::store::{Enqueued, TaskDefinition};
use crate::workflow::WorkflowError;

/// Pending signals a hand-off consumes: the oldest `limit` of `signal_type`.
///
/// The limit is how many the caller saw when it planned the next step, so a
/// signal that arrives in between stays pending for a later step instead of
/// being consumed without anything counting it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalDrain {
    pub signal_type: String,
    pub limit: usize,
}

/// What follows a completed step.
#[derive(Debug, Clone)]
pub enum NextStep {
    /// Enqueue the next step's task, claimed by `claim_for` when that worker
    /// is registered and not draining (as
    /// [`TaskQueue::enqueue_claimed_task`](crate::TaskQueue::enqueue_claimed_task)),
    /// pending otherwise. The per-workflow pending cap does not apply: the
    /// task replaces the one that completed.
    Enqueue {
        task: Box<TaskDefinition>,
        claim_for: Option<String>,
    },
    /// Mark the workflow `Completed` with this stored result and error.
    Complete {
        result: Option<serde_json::Value>,
        error: Option<WorkflowError>,
    },
}

/// A step hand-off of `workflow_id`; see the module notes.
#[derive(Debug, Clone)]
pub struct HandOff {
    pub workflow_id: Uuid,
    pub drain: Option<SignalDrain>,
    pub next: NextStep,
}

/// What a hand-off committed.
#[derive(Debug, Clone)]
pub struct HandedOff {
    /// Signals consumed for [`HandOff::drain`].
    pub drained: usize,
    /// The next step's task, when one was enqueued.
    pub next: Option<Enqueued>,
}

/// A workflow [`TaskQueue::requeue_stranded_workflows`](crate::TaskQueue::requeue_stranded_workflows)
/// put back in motion: its last step was enqueued again as `task_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequeuedWorkflow {
    pub workflow_id: Uuid,
    pub task_id: Uuid,
    pub activity_type: String,
}
