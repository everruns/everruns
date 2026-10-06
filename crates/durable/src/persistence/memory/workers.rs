//! Implementation of [`crate::WorkerRegistry`].

use super::*;

#[async_trait]
impl WorkerRegistry for InMemoryWorkflowEventStore {
    // Worker management methods
    async fn register_worker(&self, worker: WorkerInfo) -> Result<(), StoreError> {
        let mut workers = self.workers.write();
        workers.insert(worker.id.clone(), worker);
        Ok(())
    }

    async fn worker_heartbeat(
        &self,
        worker_id: &str,
        current_load: usize,
        accepting_tasks: bool,
    ) -> Result<WorkerHeartbeat, StoreError> {
        let mut workers = self.workers.write();
        let Some(worker) = workers.get_mut(worker_id) else {
            return Ok(WorkerHeartbeat::default());
        };
        let draining = worker.status == "draining";
        worker.current_load = current_load as u32;
        // A drained worker stays closed, as in PostgreSQL.
        worker.accepting_tasks = accepting_tasks && !draining;
        worker.last_heartbeat_at = Utc::now();
        Ok(WorkerHeartbeat { draining })
    }

    async fn deregister_worker(&self, worker_id: &str) -> Result<usize, StoreError> {
        let mut workers = self.workers.write();
        workers.remove(worker_id);
        // In DEV_MODE, we don't reclaim tasks since there's only one worker
        Ok(0)
    }

    async fn get_capacity_snapshot(&self) -> Result<CapacitySnapshot, StoreError> {
        let workers = self.workers.read();
        let heartbeat_threshold =
            Utc::now() - chrono::Duration::seconds(WORKER_HEARTBEAT_TIMEOUT_SECS);

        let mut total_available: u32 = 0;
        let mut active_workers: u32 = 0;
        for w in workers.values() {
            if w.status == "active"
                && w.accepting_tasks
                && w.last_heartbeat_at > heartbeat_threshold
            {
                total_available += w.max_concurrency.saturating_sub(w.current_load);
                active_workers += 1;
            }
        }
        Ok(CapacitySnapshot {
            total_available,
            active_workers,
        })
    }

    async fn list_workers(&self, filter: WorkerFilter) -> Result<Vec<WorkerInfo>, StoreError> {
        let workers = self.workers.read();
        let mut result: Vec<_> = workers
            .values()
            .filter(|w| {
                // Apply status filter
                if let Some(ref status) = filter.status
                    && &w.status != status
                {
                    return false;
                }
                // Apply worker_group filter
                if let Some(ref group) = filter.worker_group
                    && w.worker_group.as_ref() != Some(group)
                {
                    return false;
                }
                true
            })
            .cloned()
            .collect();
        result.sort_by_key(|worker| std::cmp::Reverse(worker.started_at));
        Ok(result)
    }

    async fn drain_worker(&self, worker_id: &str) -> Result<(), StoreError> {
        // Unknown ids are a no-op, as the PostgreSQL UPDATE matches no row.
        if let Some(worker) = self.workers.write().get_mut(worker_id) {
            worker.status = "draining".to_string();
            worker.accepting_tasks = false;
        }
        Ok(())
    }

    async fn resume_worker(&self, worker_id: &str) -> Result<(), StoreError> {
        // Only a draining worker resumes, matching the PostgreSQL status guard.
        if let Some(worker) = self.workers.write().get_mut(worker_id)
            && worker.status == "draining"
        {
            worker.status = "active".to_string();
            worker.accepting_tasks = true;
        }
        Ok(())
    }
}
