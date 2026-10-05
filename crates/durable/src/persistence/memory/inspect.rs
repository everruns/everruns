//! Inspection and test helpers on the in-memory store.

#[cfg(test)]
use std::sync::atomic::Ordering;

use super::*;

impl InMemoryWorkflowEventStore {
    #[cfg(test)]
    pub fn load_events_call_count(&self) -> usize {
        self.load_events_calls.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub fn count_events_call_count(&self) -> usize {
        self.count_events_calls.load(Ordering::Relaxed)
    }

    /// Get the number of workflows
    pub fn workflow_count(&self) -> usize {
        self.workflows.read().len()
    }

    /// Get the number of pending tasks
    pub fn pending_task_count(&self) -> usize {
        self.tasks.read().pending_total()
    }

    /// Make a claimed task look abandoned, so the next
    /// [`reclaim_stale_tasks`](super::TaskQueue::reclaim_stale_tasks) returns
    /// it whatever the threshold. Stands in for a worker that stopped
    /// heartbeating.
    pub fn expire_claim(&self, task_id: Uuid) {
        self.tasks.write().update(task_id, |task| {
            task.heartbeat_at = Some(chrono::DateTime::<Utc>::MIN_UTC);
        });
    }

    /// Get the number of DLQ entries
    pub fn dlq_count(&self) -> usize {
        self.dlq.read().len()
    }

    /// Clear all data (for testing)
    pub fn clear(&self) {
        self.workflows.write().clear();
        self.tasks.write().clear();
        self.workers.write().clear();
        self.dlq.write().clear();
        // Waiters wake and find their workflow gone.
        self.end_watchers.clear();
    }
}
