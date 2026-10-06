//! Workflow trait definition

use serde::{Serialize, de::DeserializeOwned};

use super::{WorkflowAction, WorkflowError, WorkflowSignal};
use crate::activity::ActivityError;

/// A workflow is a deterministic state machine driven by events
///
/// **Experimental** (`workflows` feature): the API may change in any release.
///
/// Workflows are the core abstraction for durable execution. They define:
/// - How to start execution (`on_start`)
/// - How to handle activity completions (`on_activity_completed`, `on_activity_failed`)
/// - How to handle timers (`on_timer_fired`)
/// - How to handle child workflow outcomes (`on_child_workflow_completed`,
///   `on_child_workflow_failed`)
/// - How to handle external signals (`on_signal`)
///
/// # Determinism
///
/// Workflows must be deterministic - given the same sequence of events, they must
/// produce the same sequence of actions. This enables replay-based recovery.
///
/// # Example
///
/// An order that is validated, then charged. A validation failure fails the
/// workflow; replaying the same events always yields the same actions.
///
/// ```
/// use everruns_durable::prelude::*;
/// use serde::{Deserialize, Serialize};
/// use serde_json::{Value, json};
///
/// #[derive(Clone, Serialize, Deserialize)]
/// struct Order {
///     id: String,
/// }
///
/// struct OrderWorkflow {
///     order: Order,
///     charged: Option<Value>,
///     error: Option<WorkflowError>,
/// }
///
/// impl Workflow for OrderWorkflow {
///     const TYPE: &'static str = "order";
///     type Input = Order;
///     type Output = Value;
///
///     fn new(order: Order) -> Self {
///         Self { order, charged: None, error: None }
///     }
///
///     fn on_start(&mut self) -> Vec<WorkflowAction> {
///         vec![WorkflowAction::schedule_activity("validate", "validate_order", json!({ "id": self.order.id }))]
///     }
///
///     fn on_activity_completed(&mut self, activity_id: &str, result: Value) -> Vec<WorkflowAction> {
///         match activity_id {
///             "validate" => vec![WorkflowAction::schedule_activity("charge", "charge_card", result)],
///             _ => {
///                 self.charged = Some(result.clone());
///                 vec![WorkflowAction::complete(result)]
///             }
///         }
///     }
///
///     fn on_activity_failed(&mut self, _: &str, error: &ActivityError) -> Vec<WorkflowAction> {
///         let error = WorkflowError::new(&error.message);
///         self.error = Some(error.clone());
///         vec![WorkflowAction::fail(error)]
///     }
///
///     fn is_completed(&self) -> bool {
///         self.charged.is_some() || self.error.is_some()
///     }
///
///     fn result(&self) -> Option<Value> {
///         self.charged.clone()
///     }
///
///     fn error(&self) -> Option<WorkflowError> {
///         self.error.clone()
///     }
/// }
///
/// let mut order = OrderWorkflow::new(Order { id: "o-1".into() });
/// assert_eq!(
///     order.on_start(),
///     vec![WorkflowAction::schedule_activity("validate", "validate_order", json!({ "id": "o-1" }))]
/// );
/// let next = order.on_activity_completed("validate", json!({ "id": "o-1", "amount": 42 }));
/// assert!(matches!(&next[0], WorkflowAction::ScheduleActivity { activity_id, .. } if activity_id == "charge"));
/// order.on_activity_completed("charge", json!({ "receipt": "r-1" }));
/// assert!(order.is_completed());
/// ```
pub trait Workflow: Send + Sync + 'static {
    /// Unique type identifier for this workflow
    ///
    /// This is used to look up the workflow in the registry during replay.
    const TYPE: &'static str;

    /// Input type for starting the workflow
    type Input: Serialize + DeserializeOwned + Send + Clone;

    /// Output type when workflow completes successfully
    type Output: Serialize + DeserializeOwned + Send;

    /// Create a new workflow instance from input
    ///
    /// This is called both when starting a new workflow and when replaying.
    fn new(input: Self::Input) -> Self;

    /// Called when workflow starts (or replays from beginning)
    ///
    /// Return a list of actions to schedule initial work.
    fn on_start(&mut self) -> Vec<WorkflowAction>;

    /// Called when an activity completes successfully
    ///
    /// The result is the JSON value returned by the activity.
    fn on_activity_completed(
        &mut self,
        activity_id: &str,
        result: serde_json::Value,
    ) -> Vec<WorkflowAction>;

    /// Called when an activity fails (after all retries exhausted)
    fn on_activity_failed(
        &mut self,
        activity_id: &str,
        error: &ActivityError,
    ) -> Vec<WorkflowAction>;

    /// Called when a timer fires
    fn on_timer_fired(&mut self, timer_id: &str) -> Vec<WorkflowAction> {
        let _ = timer_id;
        vec![]
    }

    /// Called when a child workflow started with
    /// [`WorkflowAction::child_workflow`] completes. `child_id` is the id this
    /// workflow gave the child.
    fn on_child_workflow_completed(
        &mut self,
        child_id: &str,
        result: serde_json::Value,
    ) -> Vec<WorkflowAction> {
        let _ = (child_id, result);
        vec![]
    }

    /// Called when a child workflow fails, or cannot be started because its
    /// type is not registered with the executor.
    fn on_child_workflow_failed(
        &mut self,
        child_id: &str,
        error: &WorkflowError,
    ) -> Vec<WorkflowAction> {
        let _ = (child_id, error);
        vec![]
    }

    /// Called when an external signal is received
    fn on_signal(&mut self, signal: &WorkflowSignal) -> Vec<WorkflowAction> {
        let _ = signal;
        vec![]
    }

    /// Check if workflow has reached a terminal state
    fn is_completed(&self) -> bool;

    /// Get the workflow result (if completed successfully)
    fn result(&self) -> Option<Self::Output>;

    /// Get the workflow error (if failed)
    fn error(&self) -> Option<WorkflowError> {
        None
    }

    /// Serialize workflow state for snapshot checkpointing.
    ///
    /// Workflows that implement this (along with `restore_state`) enable
    /// snapshot-based replay: instead of replaying all events from the beginning,
    /// the engine loads the latest snapshot and replays only events after it.
    ///
    /// Default returns `None` (snapshots disabled for this workflow type).
    fn snapshot_state(&self) -> Option<Vec<u8>> {
        None
    }

    /// Restore workflow state from a previously serialized snapshot.
    ///
    /// Returns `None` if restoration fails or snapshots aren't supported.
    /// The `input` parameter is the original workflow input (for fields not
    /// captured in the snapshot).
    fn restore_state(_input: Self::Input, _data: &[u8]) -> Option<Self>
    where
        Self: Sized,
    {
        None
    }
}
