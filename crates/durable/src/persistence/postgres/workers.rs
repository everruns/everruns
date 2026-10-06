//! Implementation of [`crate::WorkerRegistry`].

use super::*;

#[async_trait]
impl WorkerRegistry for PostgresWorkflowEventStore {
    #[instrument(skip(self, worker))]
    async fn register_worker(&self, worker: WorkerInfo) -> Result<(), StoreError> {
        // Default worker_group to "default" if not specified
        let worker_group = worker.worker_group.as_deref().unwrap_or("default");

        sqlx::query(
            r#"
            INSERT INTO durable_workers (
                id, worker_group, activity_types, max_concurrency, current_load,
                status, started_at, last_heartbeat_at, accepting_tasks, hostname, metadata
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ON CONFLICT (id) DO UPDATE SET
                worker_group = EXCLUDED.worker_group,
                activity_types = EXCLUDED.activity_types,
                max_concurrency = EXCLUDED.max_concurrency,
                current_load = EXCLUDED.current_load,
                status = EXCLUDED.status,
                last_heartbeat_at = EXCLUDED.last_heartbeat_at,
                accepting_tasks = EXCLUDED.accepting_tasks
            "#,
        )
        .bind(&worker.id)
        .bind(worker_group)
        .bind(&worker.activity_types)
        .bind(worker.max_concurrency as i32)
        .bind(worker.current_load as i32)
        .bind(&worker.status)
        .bind(worker.started_at)
        .bind(worker.last_heartbeat_at)
        .bind(worker.accepting_tasks)
        .bind::<Option<String>>(None) // hostname
        .bind::<Option<serde_json::Value>>(None) // metadata
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to register worker");
            StoreError::Database(e.to_string())
        })?;

        debug!(worker_id = %worker.id, "registered worker");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn worker_heartbeat(
        &self,
        worker_id: &str,
        current_load: usize,
        accepting_tasks: bool,
    ) -> Result<(), StoreError> {
        // Only update accepting_tasks if worker is NOT draining.
        // When draining, we preserve accepting_tasks = false set by drain_worker.
        sqlx::query(
            r#"
            UPDATE durable_workers
            SET last_heartbeat_at = NOW(),
                current_load = $2,
                accepting_tasks = CASE
                    WHEN status = 'draining' THEN false
                    ELSE $3
                END
            WHERE id = $1
            "#,
        )
        .bind(worker_id)
        .bind(current_load as i32)
        .bind(accepting_tasks)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to update worker heartbeat");
            StoreError::Database(e.to_string())
        })?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn list_workers(&self, filter: WorkerFilter) -> Result<Vec<WorkerInfo>, StoreError> {
        // Only show workers with recent heartbeat (within WORKER_HEARTBEAT_TIMEOUT_SECS)
        // Join with task queue to compute completed/failed counts
        let heartbeat_threshold =
            Utc::now() - chrono::Duration::seconds(WORKER_HEARTBEAT_TIMEOUT_SECS);

        let query = r#"
            SELECT w.id, w.worker_group, w.activity_types, w.max_concurrency, w.current_load,
                   w.status, w.started_at, w.last_heartbeat_at, w.accepting_tasks,
                   w.backpressure_reason, w.hostname, w.version, w.metadata,
                   COALESCE(stats.tasks_completed, 0) AS tasks_completed,
                   COALESCE(stats.tasks_failed, 0) AS tasks_failed,
                   stats.avg_task_duration_ms
            FROM durable_workers w
            LEFT JOIN (
                SELECT claimed_by,
                       COUNT(*) FILTER (WHERE status = 'completed') AS tasks_completed,
                       COUNT(*) FILTER (WHERE status IN ('failed', 'dead')) AS tasks_failed,
                       (AVG(EXTRACT(EPOCH FROM (heartbeat_at - claimed_at)) * 1000)
                           FILTER (WHERE status = 'completed' AND claimed_at IS NOT NULL AND heartbeat_at IS NOT NULL))::FLOAT8
                           AS avg_task_duration_ms
                FROM durable_task_queue
                WHERE claimed_by IS NOT NULL
                GROUP BY claimed_by
            ) stats ON stats.claimed_by = w.id
            WHERE w.last_heartbeat_at > $1
              AND ($2::text IS NULL OR w.status = $2)
              AND ($3::text IS NULL OR w.worker_group = $3)
            "#;

        let rows = sqlx::query(query)
            .bind(heartbeat_threshold)
            .bind(&filter.status)
            .bind(&filter.worker_group)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to list workers");
                StoreError::Database(e.to_string())
            })?;

        let workers = rows
            .into_iter()
            .map(|row| WorkerInfo {
                id: row.get("id"),
                worker_group: row.get("worker_group"),
                activity_types: row.get("activity_types"),
                max_concurrency: row.get::<i32, _>("max_concurrency") as u32,
                current_load: row.get::<i32, _>("current_load") as u32,
                status: row.get("status"),
                accepting_tasks: row.get("accepting_tasks"),
                backpressure_reason: row.get("backpressure_reason"),
                started_at: row.get("started_at"),
                last_heartbeat_at: row.get("last_heartbeat_at"),
                hostname: row.get("hostname"),
                version: row.get("version"),
                metadata: row.get("metadata"),
                tasks_completed: row.get::<i64, _>("tasks_completed") as u64,
                tasks_failed: row.get::<i64, _>("tasks_failed") as u64,
                avg_task_duration_ms: row
                    .get::<Option<f64>, _>("avg_task_duration_ms")
                    .map(|v| v as u64),
            })
            .collect();

        Ok(workers)
    }

    #[instrument(skip(self))]
    async fn deregister_worker(&self, worker_id: &str) -> Result<usize, StoreError> {
        // Reclaim all tasks claimed by this worker (set back to pending)
        // This allows immediate task reassignment instead of waiting for heartbeat timeout
        let reclaimed = sqlx::query(
            r#"
            UPDATE durable_task_queue
            SET status = 'pending',
                claimed_by = NULL,
                claimed_at = NULL
            WHERE status = 'claimed'
              AND claimed_by = $1
            "#,
        )
        .bind(worker_id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to reclaim worker tasks");
            StoreError::Database(e.to_string())
        })?
        .rows_affected() as usize;

        // Mark worker as stopped
        sqlx::query(
            r#"
            UPDATE durable_workers
            SET status = 'stopped'
            WHERE id = $1
            "#,
        )
        .bind(worker_id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to deregister worker");
            StoreError::Database(e.to_string())
        })?;

        if reclaimed > 0 {
            info!(
                worker_id,
                reclaimed, "deregistered worker and reclaimed tasks"
            );
        } else {
            debug!(worker_id, "deregistered worker (no tasks to reclaim)");
        }

        Ok(reclaimed)
    }

    #[instrument(skip(self))]
    async fn get_capacity_snapshot(&self) -> Result<CapacitySnapshot, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT
                COALESCE(SUM(max_concurrency - current_load), 0)::INT AS total_available,
                COUNT(*)::INT AS active_workers
            FROM durable_workers
            WHERE status = 'active'
              AND accepting_tasks = true
              AND last_heartbeat_at > NOW() - INTERVAL '30 seconds'
            "#,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to get capacity snapshot");
            StoreError::Database(e.to_string())
        })?;

        Ok(CapacitySnapshot {
            total_available: row.get::<i32, _>("total_available") as u32,
            active_workers: row.get::<i32, _>("active_workers") as u32,
        })
    }

    #[instrument(skip(self))]
    async fn drain_worker(&self, worker_id: &str) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            UPDATE durable_workers
            SET status = 'draining',
                accepting_tasks = false
            WHERE id = $1
            "#,
        )
        .bind(worker_id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to drain worker");
            StoreError::Database(e.to_string())
        })?;

        debug!(worker_id, "drained worker");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn resume_worker(&self, worker_id: &str) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            UPDATE durable_workers
            SET status = 'active',
                accepting_tasks = true
            WHERE id = $1 AND status = 'draining'
            "#,
        )
        .bind(worker_id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to resume worker");
            StoreError::Database(e.to_string())
        })?;

        debug!(worker_id, "resumed worker");
        Ok(())
    }
}
