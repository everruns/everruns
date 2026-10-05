//! Implementation of [`crate::DeadLetters`].

use super::*;

#[async_trait]
impl DeadLetters for InMemoryWorkflowEventStore {
    async fn move_to_dlq(
        &self,
        task_id: Uuid,
        error_history: Vec<String>,
    ) -> Result<(), StoreError> {
        let tasks = self.tasks.read();
        let task = tasks
            .get(&task_id)
            .ok_or(StoreError::TaskNotFound(task_id))?;

        let entry = DlqEntry {
            id: Uuid::now_v7(),
            original_task_id: task_id,
            workflow_id: task.definition.workflow_id,
            activity_id: task.definition.activity_id.clone(),
            activity_type: task.definition.activity_type.clone(),
            input: task.definition.input.clone(),
            attempts: task.attempt,
            last_error: task.last_error.clone().unwrap_or_default(),
            error_history,
            dead_at: Utc::now(),
        };

        drop(tasks);
        self.dlq.write().insert(entry.id, entry);
        Ok(())
    }

    async fn requeue_from_dlq(&self, dlq_id: Uuid) -> Result<Uuid, StoreError> {
        let mut dlq = self.dlq.write();
        let entry = dlq
            .remove(&dlq_id)
            .ok_or(StoreError::TaskNotFound(dlq_id))?;

        drop(dlq);

        // Create new task from DLQ entry
        let task_id = Uuid::now_v7();
        let mut tasks = self.tasks.write();

        // We need to recreate options - use defaults for simplicity in test
        let options = crate::workflow::ActivityOptions::default();

        tasks.insert(
            task_id,
            TaskState::pending(TaskDefinition {
                workflow_id: entry.workflow_id,
                activity_id: entry.activity_id,
                activity_type: entry.activity_type,
                input: entry.input,
                options,
            }),
        );

        Ok(task_id)
    }

    async fn list_dlq(
        &self,
        filter: DlqFilter,
        pagination: Pagination,
    ) -> Result<Vec<DlqEntry>, StoreError> {
        let dlq = self.dlq.read();
        let mut entries: Vec<_> = dlq
            .values()
            .filter(|e| {
                if let Some(wid) = filter.workflow_id
                    && e.workflow_id != Some(wid)
                {
                    return false;
                }
                if let Some(ref at) = filter.activity_type
                    && &e.activity_type != at
                {
                    return false;
                }
                true
            })
            .cloned()
            .collect();

        entries.sort_by_key(|entry| std::cmp::Reverse(entry.dead_at));

        let start = pagination.offset as usize;
        let end = (pagination.offset + pagination.limit) as usize;

        Ok(entries.into_iter().skip(start).take(end - start).collect())
    }
}
