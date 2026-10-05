//! Wakeups for a workflow's end, so an in-process waiter (a turn ticket)
//! need not poll the store.
//!
//! Every path in this store that makes a workflow terminal
//! (`update_workflow_status`, `try_fail_workflow`, `cancel_workflow`,
//! `continue_as_new`) calls `notify_workflow_ended`
//! after it writes the status, so the store's status writes are the one choke
//! point. A waiter in another process cannot see these and must still poll a
//! shared store.

use std::collections::HashMap;

use parking_lot::Mutex;
use tokio::sync::watch;
use uuid::Uuid;

use super::InMemoryWorkflowEventStore;

/// One sender per workflow someone waits on to end. Ending the workflow
/// removes and drops it, which wakes every subscribed receiver. An entry
/// whose subscribers went away stays until the workflow next ends: at most
/// one per workflow, which the store keeps anyway.
#[derive(Default)]
pub(super) struct EndWatchers(Mutex<HashMap<Uuid, watch::Sender<()>>>);

impl EndWatchers {
    /// Wake every waiter, as when the store is cleared.
    pub(super) fn clear(&self) {
        self.0.lock().clear();
    }
}

impl InMemoryWorkflowEventStore {
    /// Subscribe to `workflow_id`'s next transition to a terminal status.
    ///
    /// The subscription starts when this returns, before it is awaited, so a
    /// caller that subscribes and then reads the status cannot miss an end
    /// that lands between the two. Every terminal transition this store
    /// makes wakes it, and so does [`clear`](Self::clear). A waiter re-reads
    /// the status after it wakes: the workflow may already have started its
    /// next run.
    pub fn subscribe_workflow_end(&self, workflow_id: Uuid) -> WorkflowEndSubscription {
        let receiver = self
            .end_watchers
            .0
            .lock()
            .entry(workflow_id)
            .or_insert_with(|| watch::channel(()).0)
            .subscribe();
        WorkflowEndSubscription { receiver }
    }

    /// Wake everyone subscribed to `workflow_id`'s end. Called after the
    /// terminal status is written, so a woken waiter reads it.
    pub(super) fn notify_workflow_ended(&self, workflow_id: Uuid) {
        // Dropping the sender closes the channel, which is the wakeup.
        self.end_watchers.0.lock().remove(&workflow_id);
    }
}

/// A pending wait for a workflow's next terminal transition, from
/// [`InMemoryWorkflowEventStore::subscribe_workflow_end`].
#[derive(Debug)]
pub struct WorkflowEndSubscription {
    receiver: watch::Receiver<()>,
}

impl WorkflowEndSubscription {
    /// Resolve once the workflow has ended since the subscription started.
    pub async fn ended(mut self) {
        // The sender never sends: the only event is its drop, which makes
        // `changed` return an error.
        let _ = self.receiver.changed().await;
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::persistence::store::{EventLog, WorkflowStatus};
    use crate::workflow::WorkflowError;

    /// Whether `subscription` has resolved, without waiting for it.
    async fn has_ended(subscription: WorkflowEndSubscription) -> bool {
        tokio::time::timeout(Duration::ZERO, subscription.ended())
            .await
            .is_ok()
    }

    async fn running_workflow(store: &InMemoryWorkflowEventStore) -> Uuid {
        let workflow_id = Uuid::now_v7();
        store
            .create_workflow(workflow_id, "test_workflow", serde_json::json!({}), None)
            .await
            .unwrap();
        store
            .update_workflow_status(workflow_id, WorkflowStatus::Running, None, None)
            .await
            .unwrap();
        workflow_id
    }

    #[tokio::test]
    async fn workflow_end_subscription_wakes_on_every_terminal_transition() {
        let store = InMemoryWorkflowEventStore::new();

        // A status write that does not end the workflow wakes nobody.
        let workflow_id = running_workflow(&store).await;
        let subscription = store.subscribe_workflow_end(workflow_id);
        store
            .update_workflow_status(workflow_id, WorkflowStatus::Running, None, None)
            .await
            .unwrap();
        assert!(!has_ended(subscription).await);

        for status in [
            WorkflowStatus::Completed,
            WorkflowStatus::Failed,
            WorkflowStatus::Cancelled,
            WorkflowStatus::ContinuedAsNew,
        ] {
            let workflow_id = running_workflow(&store).await;
            let first = store.subscribe_workflow_end(workflow_id);
            let second = store.subscribe_workflow_end(workflow_id);
            store
                .update_workflow_status(workflow_id, status, None, None)
                .await
                .unwrap();
            assert!(has_ended(first).await, "{status:?}");
            assert!(has_ended(second).await, "{status:?}");
        }

        let workflow_id = running_workflow(&store).await;
        let subscription = store.subscribe_workflow_end(workflow_id);
        assert!(
            store
                .try_fail_workflow(workflow_id, WorkflowError::new("dead task"))
                .await
                .unwrap()
        );
        assert!(has_ended(subscription).await, "try_fail_workflow");

        let workflow_id = running_workflow(&store).await;
        let subscription = store.subscribe_workflow_end(workflow_id);
        store.cancel_workflow(workflow_id).await.unwrap();
        assert!(has_ended(subscription).await, "cancel_workflow");

        let workflow_id = running_workflow(&store).await;
        let subscription = store.subscribe_workflow_end(workflow_id);
        store
            .continue_as_new(workflow_id, "test_workflow", serde_json::json!({}), vec![])
            .await
            .unwrap();
        assert!(has_ended(subscription).await, "continue_as_new");

        // A subscription taken after the end waits for the next one.
        let workflow_id = running_workflow(&store).await;
        store
            .update_workflow_status(workflow_id, WorkflowStatus::Completed, None, None)
            .await
            .unwrap();
        let late = store.subscribe_workflow_end(workflow_id);
        assert!(!has_ended(late).await);
    }
}
