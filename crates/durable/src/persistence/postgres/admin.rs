//! Implementation of [`crate::DurableAdmin`].

use super::*;

#[async_trait]
impl DurableAdmin for PostgresWorkflowEventStore {
    #[instrument(skip(self))]
    async fn count_active_workflows(&self) -> Result<i64, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT COUNT(*) as count
            FROM durable_workflow_instances
            WHERE status IN ('pending', 'running')
            "#,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to count active workflows");
            StoreError::Database(e.to_string())
        })?;

        Ok(row.get("count"))
    }

    #[instrument(skip(self))]
    async fn get_workflow_extended(
        &self,
        workflow_id: Uuid,
    ) -> Result<Option<WorkflowInfoExtended>, StoreError> {
        // EVE-455: direct id lookup. The previous implementation scanned the
        // first 1000 rows of `list_workflows`, which silently dropped older
        // workflows once a deployment grew past that page.
        let row = sqlx::query(
            r#"
            SELECT id, workflow_type, status, input, result, error,
                   created_at, started_at, completed_at, continued_as_new_id
            FROM durable_workflow_instances
            WHERE id = $1
            "#,
        )
        .bind(workflow_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, workflow_id = %workflow_id, "Failed to load workflow");
            StoreError::Database(e.to_string())
        })?;

        let Some(row) = row else { return Ok(None) };
        let status_str: String = row.get("status");
        let error_json: Option<serde_json::Value> = row.get("error");
        Ok(Some(WorkflowInfoExtended {
            id: row.get("id"),
            workflow_type: row.get("workflow_type"),
            status: parse_workflow_status(&status_str)?,
            input: row.get("input"),
            result: row.get("result"),
            error: error_json.and_then(|v| serde_json::from_value(v).ok()),
            created_at: row.get("created_at"),
            started_at: row.get("started_at"),
            completed_at: row.get("completed_at"),
            continued_as_new_id: row.try_get("continued_as_new_id").ok().flatten(),
        }))
    }

    async fn list_workflows(
        &self,
        filter: WorkflowFilter,
        pagination: Pagination,
    ) -> Result<Vec<WorkflowInfoExtended>, StoreError> {
        let status_str = filter.status.map(|s| s.to_string());

        let rows = sqlx::query(
            r#"
            SELECT id, workflow_type, status, input, result, error,
                   created_at, started_at, completed_at, continued_as_new_id
            FROM durable_workflow_instances
            WHERE ($1::text IS NULL OR status = $1)
              AND ($2::text IS NULL OR workflow_type = $2)
            ORDER BY created_at DESC
            OFFSET $3
            LIMIT $4
            "#,
        )
        .bind(&status_str)
        .bind(&filter.workflow_type)
        .bind(pagination.offset as i64)
        .bind(pagination.limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to list workflows");
            StoreError::Database(e.to_string())
        })?;

        let mut workflows = Vec::with_capacity(rows.len());
        for row in rows {
            let status_str: String = row.get("status");
            let error_json: Option<serde_json::Value> = row.get("error");

            workflows.push(WorkflowInfoExtended {
                id: row.get("id"),
                workflow_type: row.get("workflow_type"),
                status: parse_workflow_status(&status_str)?,
                input: row.get("input"),
                result: row.get("result"),
                error: error_json.and_then(|v| serde_json::from_value(v).ok()),
                created_at: row.get("created_at"),
                started_at: row.get("started_at"),
                completed_at: row.get("completed_at"),
                continued_as_new_id: row.try_get("continued_as_new_id").ok().flatten(),
            });
        }

        Ok(workflows)
    }

    #[instrument(skip(self))]
    async fn get_system_health(&self) -> Result<SystemHealth, StoreError> {
        // Single query to get all health stats.
        // $1 = heartbeat threshold: only workers with heartbeat after this are "active".
        let heartbeat_threshold =
            Utc::now() - chrono::Duration::seconds(WORKER_HEARTBEAT_TIMEOUT_SECS);

        // Live gauges (pending/claimed tasks, running/pending workflows, workers,
        // DLQ) are bounded by current work and backed by partial indexes, so they
        // stay as direct queries. The cumulative totals (completed/failed/started
        // for tasks and workflows) are read from `durable_stat_counters`, which is
        // maintained incrementally by triggers (migration 082) and sharded so
        // writers do not queue on one row (migration 182); a total is the sum of
        // its shards. This keeps the health path O(1) instead of scanning the
        // unbounded historical tables on every 10s metrics sample. See EVE-605.
        let row = sqlx::query(
            r#"
            SELECT
                (SELECT COUNT(*) FROM durable_workers) as total_workers,
                (SELECT COUNT(*) FROM durable_workers WHERE status = 'active' AND last_heartbeat_at > $1) as active_workers,
                (SELECT COUNT(*) FROM durable_workers WHERE status = 'active' AND accepting_tasks = true AND last_heartbeat_at > $1) as workers_accepting,
                (SELECT COALESCE(SUM(max_concurrency), 0) FROM durable_workers WHERE status = 'active' AND last_heartbeat_at > $1) as total_capacity,
                (SELECT COALESCE(SUM(current_load), 0) FROM durable_workers WHERE status = 'active' AND last_heartbeat_at > $1) as current_load,
                (SELECT COUNT(*) FROM durable_task_queue WHERE status = 'pending') as pending_tasks,
                (SELECT COUNT(*) FROM durable_task_queue WHERE status = 'claimed') as claimed_tasks,
                COALESCE((SELECT SUM(value) FROM durable_stat_counters WHERE name = 'tasks_completed'), 0)::BIGINT as completed_tasks,
                COALESCE((SELECT SUM(value) FROM durable_stat_counters WHERE name = 'tasks_failed'), 0)::BIGINT as failed_tasks,
                COALESCE((SELECT SUM(value) FROM durable_stat_counters WHERE name = 'tasks_started'), 0)::BIGINT as started_tasks,
                (SELECT COUNT(*) FROM durable_workflow_instances WHERE status = 'running') as running_workflows,
                (SELECT COUNT(*) FROM durable_workflow_instances WHERE status = 'pending') as pending_workflows,
                COALESCE((SELECT SUM(value) FROM durable_stat_counters WHERE name = 'workflows_completed'), 0)::BIGINT as completed_workflows,
                COALESCE((SELECT SUM(value) FROM durable_stat_counters WHERE name = 'workflows_failed'), 0)::BIGINT as failed_workflows,
                COALESCE((SELECT SUM(value) FROM durable_stat_counters WHERE name = 'workflows_started'), 0)::BIGINT as started_workflows,
                (SELECT COUNT(*) FROM durable_dead_letter_queue WHERE requeued_at IS NULL) as dlq_size
            "#,
        )
        .bind(heartbeat_threshold)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to get system health");
            StoreError::Database(e.to_string())
        })?;

        Ok(SystemHealth {
            total_workers: row.get::<i64, _>("total_workers") as usize,
            active_workers: row.get::<i64, _>("active_workers") as usize,
            workers_accepting: row.get::<i64, _>("workers_accepting") as usize,
            total_capacity: row.get::<i64, _>("total_capacity") as usize,
            current_load: row.get::<i64, _>("current_load") as usize,
            pending_tasks: row.get::<i64, _>("pending_tasks") as usize,
            claimed_tasks: row.get::<i64, _>("claimed_tasks") as usize,
            completed_tasks: row.get::<i64, _>("completed_tasks") as usize,
            failed_tasks: row.get::<i64, _>("failed_tasks") as usize,
            started_tasks: row.get::<i64, _>("started_tasks") as usize,
            running_workflows: row.get::<i64, _>("running_workflows") as usize,
            pending_workflows: row.get::<i64, _>("pending_workflows") as usize,
            completed_workflows: row.get::<i64, _>("completed_workflows") as usize,
            failed_workflows: row.get::<i64, _>("failed_workflows") as usize,
            started_workflows: row.get::<i64, _>("started_workflows") as usize,
            dlq_size: row.get::<i64, _>("dlq_size") as usize,
        })
    }

    #[instrument(skip(self))]
    async fn get_workflow_events(
        &self,
        workflow_id: Uuid,
    ) -> Result<Vec<WorkflowEventInfo>, StoreError> {
        let rows = sqlx::query(
            r#"
            SELECT id, workflow_id, sequence_num, event_type, event_data, created_at
            FROM durable_workflow_events
            WHERE workflow_id = $1
            ORDER BY sequence_num
            "#,
        )
        .bind(workflow_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to get workflow events");
            StoreError::Database(e.to_string())
        })?;

        let events = rows
            .into_iter()
            .map(|row| WorkflowEventInfo {
                id: row.get("id"),
                workflow_id: row.get("workflow_id"),
                sequence_num: row.get("sequence_num"),
                event_type: row.get("event_type"),
                event_data: row.get("event_data"),
                created_at: row.get("created_at"),
            })
            .collect();

        Ok(events)
    }

    #[instrument(skip(self))]
    async fn count_workflow_events(&self, workflow_id: Uuid) -> Result<i64, StoreError> {
        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM durable_workflow_events WHERE workflow_id = $1")
                .bind(workflow_id)
                .fetch_one(&self.pool)
                .await
                .map_err(|e| {
                    error!(error = %e, "Failed to count workflow events");
                    StoreError::Database(e.to_string())
                })?;

        Ok(count)
    }
}
