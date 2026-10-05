//! Implementation of [`crate::DurableAdmin`].

use super::*;

#[async_trait]
impl DurableAdmin for InMemoryWorkflowEventStore {
    async fn get_system_health(&self) -> Result<SystemHealth, StoreError> {
        let workers = self.workers.read();
        let heartbeat_threshold =
            Utc::now() - chrono::Duration::seconds(WORKER_HEARTBEAT_TIMEOUT_SECS);

        let total_workers = workers.len();
        let active_workers = workers
            .values()
            .filter(|w| w.status == "active" && w.last_heartbeat_at > heartbeat_threshold)
            .count();
        let workers_accepting = workers
            .values()
            .filter(|w| {
                w.status == "active"
                    && w.accepting_tasks
                    && w.last_heartbeat_at > heartbeat_threshold
            })
            .count();
        let total_capacity: usize = workers
            .values()
            .filter(|w| w.status == "active" && w.last_heartbeat_at > heartbeat_threshold)
            .map(|w| w.max_concurrency as usize)
            .sum();
        let current_load: usize = workers
            .values()
            .filter(|w| w.status == "active" && w.last_heartbeat_at > heartbeat_threshold)
            .map(|w| w.current_load as usize)
            .sum();
        drop(workers);

        let tasks = self.tasks.read();
        let pending_tasks = tasks
            .values()
            .filter(|t| t.status == TaskStatus::Pending)
            .count();
        let claimed_tasks = tasks
            .values()
            .filter(|t| t.status == TaskStatus::Claimed)
            .count();
        let completed_tasks = tasks
            .values()
            .filter(|t| t.status == TaskStatus::Completed)
            .count();
        let failed_tasks = tasks
            .values()
            .filter(|t| matches!(t.status, TaskStatus::Failed | TaskStatus::Dead))
            .count();
        let started_tasks = tasks.values().filter(|t| t.claimed_at.is_some()).count();
        drop(tasks);

        let workflows = self.workflows.read();
        let running_workflows = workflows
            .values()
            .filter(|w| w.status == WorkflowStatus::Running)
            .count();
        let pending_workflows = workflows
            .values()
            .filter(|w| w.status == WorkflowStatus::Pending)
            .count();
        let completed_workflows = workflows
            .values()
            .filter(|w| w.status == WorkflowStatus::Completed)
            .count();
        let failed_workflows = workflows
            .values()
            .filter(|w| matches!(w.status, WorkflowStatus::Failed | WorkflowStatus::Cancelled))
            .count();
        let started_workflows = workflows
            .values()
            .filter(|w| w.started_at.is_some())
            .count();
        drop(workflows);

        let dlq_size = self.dlq.read().len();

        Ok(SystemHealth {
            total_workers,
            active_workers,
            workers_accepting,
            total_capacity,
            current_load,
            pending_tasks,
            claimed_tasks,
            completed_tasks,
            failed_tasks,
            started_tasks,
            running_workflows,
            pending_workflows,
            completed_workflows,
            failed_workflows,
            started_workflows,
            dlq_size,
        })
    }

    async fn get_workflow_extended(
        &self,
        workflow_id: Uuid,
    ) -> Result<Option<WorkflowInfoExtended>, StoreError> {
        // EVE-455: direct id lookup that does not depend on the
        // `list_workflows` page size.
        let workflows = self.workflows.read();
        Ok(workflows.get(&workflow_id).map(|w| WorkflowInfoExtended {
            id: workflow_id,
            workflow_type: w.workflow_type.clone(),
            status: w.status,
            input: w.input.clone(),
            result: w.result.clone(),
            error: w.error.clone(),
            created_at: w.created_at,
            started_at: w.started_at,
            completed_at: w.completed_at,
            continued_as_new_id: w.continued_as_new_id,
        }))
    }

    async fn list_workflows(
        &self,
        filter: WorkflowFilter,
        pagination: Pagination,
    ) -> Result<Vec<WorkflowInfoExtended>, StoreError> {
        let workflows = self.workflows.read();
        let mut result: Vec<_> = workflows
            .iter()
            .filter(|(_, w)| {
                // Apply status filter
                if let Some(ref status) = filter.status
                    && &w.status != status
                {
                    return false;
                }
                // Apply workflow_type filter
                if let Some(ref wf_type) = filter.workflow_type
                    && &w.workflow_type != wf_type
                {
                    return false;
                }
                true
            })
            .map(|(id, w)| WorkflowInfoExtended {
                id: *id,
                workflow_type: w.workflow_type.clone(),
                status: w.status,
                input: w.input.clone(),
                result: w.result.clone(),
                error: w.error.clone(),
                created_at: w.created_at,
                started_at: w.started_at,
                completed_at: w.completed_at,
                continued_as_new_id: w.continued_as_new_id,
            })
            .collect();
        result.sort_by_key(|workflow| std::cmp::Reverse(workflow.created_at));

        let start = pagination.offset as usize;
        let end = (pagination.offset + pagination.limit) as usize;
        Ok(result.into_iter().skip(start).take(end - start).collect())
    }

    async fn count_active_workflows(&self) -> Result<i64, StoreError> {
        let workflows = self.workflows.read();
        Ok(workflows
            .values()
            .filter(|w| matches!(w.status, WorkflowStatus::Pending | WorkflowStatus::Running))
            .count() as i64)
    }
}
