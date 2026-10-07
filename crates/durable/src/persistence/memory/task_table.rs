//! Task table for the in-memory store.
//!
//! Decision: the in-memory store keeps the indexes PostgreSQL gets from SQL, so
//! a claim costs O(log n) in the pending set instead of a scan over every task
//! ever created. Every status change goes through [`TaskTable::update`], which
//! drops the task from the indexes before the change and re-adds it after, so
//! the indexes cannot drift from the rows.

use std::cmp::Reverse;
use std::collections::{BTreeSet, HashMap, HashSet};

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::persistence::store::{TaskDefinition, TaskStatus};

/// One row of the in-memory task queue.
pub(super) struct TaskState {
    pub definition: TaskDefinition,
    pub status: TaskStatus,
    pub attempt: u32,
    pub claimed_by: Option<String>,
    pub last_error: Option<String>,
    pub error_history: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub claimed_at: Option<DateTime<Utc>>,
    /// Last proof of life from the claiming worker; reclaim compares it with
    /// the stale threshold, like `durable_task_queue.heartbeat_at`.
    pub heartbeat_at: Option<DateTime<Utc>>,
    /// Earliest time a pending task may be claimed. Retries push it out by the
    /// retry policy's backoff, like `durable_task_queue.visible_at`.
    pub visible_at: DateTime<Utc>,
    /// Forward-progress guard (EVE-534): progress token observed at the previous
    /// reclaim, and consecutive no-progress recovery count. `None` token means
    /// the task has not been reclaimed yet.
    pub progress_token: Option<i64>,
    pub no_progress_count: u32,
}

impl TaskState {
    /// A fresh pending task, visible now.
    pub fn pending(definition: TaskDefinition) -> Self {
        let now = Utc::now();
        Self {
            definition,
            status: TaskStatus::Pending,
            attempt: 0,
            claimed_by: None,
            last_error: None,
            error_history: vec![],
            created_at: now,
            claimed_at: None,
            heartbeat_at: None,
            visible_at: now,
            progress_token: None,
            no_progress_count: 0,
        }
    }

    /// A newly enqueued task: claimable once its `start_delay` has passed.
    pub fn scheduled(definition: TaskDefinition) -> Self {
        let mut task = Self::pending(definition);
        if let Some(delay) = task.definition.options.start_delay {
            let delay = chrono::Duration::from_std(delay).unwrap_or(chrono::Duration::MAX);
            task.visible_at =
                (task.visible_at.checked_add_signed(delay)).unwrap_or(DateTime::<Utc>::MAX_UTC);
        }
        task
    }

    /// Return the task to the queue, claimable from `visible_at`.
    pub fn release(&mut self, visible_at: DateTime<Utc>) {
        self.status = TaskStatus::Pending;
        self.claimed_by = None;
        self.claimed_at = None;
        self.heartbeat_at = None;
        self.visible_at = visible_at;
    }
}

/// PostgreSQL claim order: `ORDER BY priority DESC, visible_at`, then the
/// time-ordered id so ties are FIFO.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct PendingKey {
    priority: Reverse<i32>,
    visible_at: DateTime<Utc>,
    id: Uuid,
}

/// `(queue, activity_type)` of a pending task.
type PendingQueue = (Option<String>, String);

fn pending_queue(definition: &TaskDefinition) -> PendingQueue {
    (
        definition.options.queue.clone(),
        definition.activity_type.clone(),
    )
}

#[derive(Default)]
pub(super) struct TaskTable {
    rows: HashMap<Uuid, TaskState>,
    /// Pending tasks per task queue (`None`: the default queue) and
    /// activity type, in claim order.
    pending: HashMap<PendingQueue, BTreeSet<PendingKey>>,
    /// Pending task count per workflow; `None` counts standalone tasks.
    pending_per_workflow: HashMap<Option<Uuid>, u32>,
    claimed: HashSet<Uuid>,
    by_workflow: HashMap<Uuid, Vec<Uuid>>,
}

impl TaskTable {
    pub fn get(&self, id: &Uuid) -> Option<&TaskState> {
        self.rows.get(id)
    }

    pub fn values(&self) -> impl Iterator<Item = &TaskState> {
        self.rows.values()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Uuid, &TaskState)> {
        self.rows.iter()
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn insert(&mut self, id: Uuid, task: TaskState) {
        if let Some(workflow_id) = task.definition.workflow_id {
            self.by_workflow.entry(workflow_id).or_default().push(id);
        }
        self.rows.insert(id, task);
        self.index(id);
    }

    /// Change one task, keeping the indexes in step with its new state.
    pub fn update<R>(&mut self, id: Uuid, change: impl FnOnce(&mut TaskState) -> R) -> Option<R> {
        if !self.rows.contains_key(&id) {
            return None;
        }
        self.unindex(id);
        let result = self.rows.get_mut(&id).map(change);
        self.index(id);
        result
    }

    /// Ids of every task, in any state, that belongs to `workflow_id`.
    pub fn ids_for_workflow(&self, workflow_id: Uuid) -> Vec<Uuid> {
        self.by_workflow
            .get(&workflow_id)
            .cloned()
            .unwrap_or_default()
    }

    /// The last task of `workflow_id` when it completed and no task of the
    /// workflow is pending or claimed: the step a run stranded between steps
    /// resumes from (see `TaskQueue::requeue_stranded_workflows`).
    pub fn stranded_last_step(&self, workflow_id: Uuid) -> Option<(Uuid, &TaskState)> {
        let ids = self.by_workflow.get(&workflow_id)?;
        let live = ids.iter().any(|id| {
            self.rows
                .get(id)
                .is_some_and(|t| matches!(t.status, TaskStatus::Pending | TaskStatus::Claimed))
        });
        // Rows are appended in enqueue order, so the last one is the latest.
        let id = *ids.last()?;
        let last = self.rows.get(&id)?;
        (!live && last.status == TaskStatus::Completed).then_some((id, last))
    }

    /// Enqueue the stranded last step of `workflow_id` again, pending, when
    /// it completed before `completed_before`. Returns the new task's id and
    /// activity type.
    pub fn requeue_stranded(
        &mut self,
        workflow_id: Uuid,
        completed_before: DateTime<Utc>,
    ) -> Option<(Uuid, String)> {
        let (_, last) = self.stranded_last_step(workflow_id)?;
        let completed_at = last
            .heartbeat_at
            .or(last.claimed_at)
            .unwrap_or(last.created_at);
        if completed_at > completed_before {
            return None;
        }
        let definition = last.definition.clone();
        let activity_type = definition.activity_type.clone();
        let task_id = Uuid::now_v7();
        self.insert(task_id, TaskState::pending(definition));
        Some((task_id, activity_type))
    }

    pub fn claimed_ids(&self) -> Vec<Uuid> {
        self.claimed.iter().copied().collect()
    }

    /// Pending tasks of one workflow, or of all standalone tasks for `None`.
    pub fn pending_count(&self, workflow_id: Option<Uuid>) -> u32 {
        self.pending_per_workflow
            .get(&workflow_id)
            .copied()
            .unwrap_or(0)
    }

    pub fn pending_total(&self) -> usize {
        self.pending.values().map(BTreeSet::len).sum()
    }

    /// Up to `max` claimable tasks of the given types in `queue`, in claim
    /// order.
    pub fn claimable(
        &self,
        queue: Option<&str>,
        activity_types: &[String],
        now: DateTime<Utc>,
        max: usize,
    ) -> Vec<Uuid> {
        let mut keys: Vec<PendingKey> = Vec::new();
        for activity_type in activity_types {
            let Some(pending) = self
                .pending
                .get(&(queue.map(str::to_owned), activity_type.clone()))
            else {
                continue;
            };
            // Priority sorts ahead of visibility, so a delayed high-priority
            // retry can precede visible work: skip it rather than stop.
            keys.extend(
                pending
                    .iter()
                    .filter(|key| key.visible_at <= now && self.has_attempts_left(key.id))
                    .take(max),
            );
        }
        keys.sort();
        keys.dedup();
        keys.into_iter().take(max).map(|key| key.id).collect()
    }

    fn has_attempts_left(&self, id: Uuid) -> bool {
        self.rows
            .get(&id)
            .is_some_and(|task| task.attempt < task.definition.options.retry_policy.max_attempts)
    }

    fn index(&mut self, id: Uuid) {
        let Some(task) = self.rows.get(&id) else {
            return;
        };
        match task.status {
            TaskStatus::Pending => {
                self.pending
                    .entry(pending_queue(&task.definition))
                    .or_default()
                    .insert(PendingKey {
                        priority: Reverse(task.definition.options.priority),
                        visible_at: task.visible_at,
                        id,
                    });
                *self
                    .pending_per_workflow
                    .entry(task.definition.workflow_id)
                    .or_default() += 1;
            }
            TaskStatus::Claimed => {
                self.claimed.insert(id);
            }
            _ => {}
        }
    }

    fn unindex(&mut self, id: Uuid) {
        let Some(task) = self.rows.get(&id) else {
            return;
        };
        match task.status {
            TaskStatus::Pending => {
                if let Some(queue) = self.pending.get_mut(&pending_queue(&task.definition)) {
                    queue.remove(&PendingKey {
                        priority: Reverse(task.definition.options.priority),
                        visible_at: task.visible_at,
                        id,
                    });
                }
                if let Some(count) = self
                    .pending_per_workflow
                    .get_mut(&task.definition.workflow_id)
                {
                    *count = count.saturating_sub(1);
                }
            }
            TaskStatus::Claimed => {
                self.claimed.remove(&id);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::ActivityOptions;
    use serde_json::json;

    fn task(activity_type: &str, priority: i32) -> TaskState {
        TaskState::pending(TaskDefinition {
            workflow_id: None,
            activity_id: "a".into(),
            activity_type: activity_type.into(),
            input: json!(null),
            options: ActivityOptions {
                priority,
                ..ActivityOptions::default()
            },
        })
    }

    #[test]
    fn claim_order_is_priority_then_fifo() {
        let mut table = TaskTable::default();
        let ids: Vec<Uuid> = (0..4).map(|_| Uuid::now_v7()).collect();
        table.insert(ids[0], task("t", 0));
        table.insert(ids[1], task("t", 5));
        table.insert(ids[2], task("u", 0));
        table.insert(ids[3], task("t", 5));

        let types = ["t".to_string(), "u".to_string()];
        let order = table.claimable(None, &types, Utc::now(), 10);
        assert_eq!(order[..2], [ids[1], ids[3]]);
        assert_eq!(order.len(), 4);
        assert_eq!(table.claimable(None, &types, Utc::now(), 1), vec![ids[1]]);
    }

    #[test]
    fn a_claim_takes_only_its_own_queue() {
        let mut table = TaskTable::default();
        let default = Uuid::now_v7();
        let queued = Uuid::now_v7();
        table.insert(default, task("t", 0));
        let mut in_queue = task("t", 0);
        in_queue.definition.options.queue = Some("q".into());
        table.insert(queued, in_queue);

        let types = ["t".to_string()];
        assert_eq!(table.claimable(None, &types, Utc::now(), 10), vec![default]);
        assert_eq!(
            table.claimable(Some("q"), &types, Utc::now(), 10),
            vec![queued]
        );
        assert!(
            table
                .claimable(Some("other"), &types, Utc::now(), 10)
                .is_empty()
        );

        table.update(queued, |t| t.status = TaskStatus::Claimed);
        assert!(
            table
                .claimable(Some("q"), &types, Utc::now(), 10)
                .is_empty()
        );
        assert_eq!(table.pending_total(), 1);
    }

    #[test]
    fn update_keeps_indexes_in_step() {
        let mut table = TaskTable::default();
        let id = Uuid::now_v7();
        table.insert(id, task("t", 0));
        assert_eq!(table.pending_count(None), 1);

        table.update(id, |t| t.status = TaskStatus::Claimed);
        assert_eq!(table.pending_count(None), 0);
        assert_eq!(table.claimed_ids(), vec![id]);
        assert!(
            table
                .claimable(None, &["t".into()], Utc::now(), 10)
                .is_empty()
        );

        let later = Utc::now() + chrono::Duration::seconds(60);
        table.update(id, |t| t.release(later));
        assert!(table.claimed_ids().is_empty());
        assert!(
            table
                .claimable(None, &["t".into()], Utc::now(), 10)
                .is_empty()
        );
        assert_eq!(table.claimable(None, &["t".into()], later, 10), vec![id]);
    }
}
