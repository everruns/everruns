//! Schedules implementation (see `store.rs` for the trait contract).

use super::*;

#[async_trait]
impl Schedules for PostgresWorkflowEventStore {
    #[instrument(skip(self, schedule))]
    async fn create_schedule(&self, schedule: CreateScheduleRow) -> Result<Uuid, StoreError> {
        let id = Uuid::now_v7();

        sqlx::query(
            r#"
            INSERT INTO durable_schedules (
                id, name, description, cron_expression, timezone,
                target_type, target_name, target_input, enabled,
                max_concurrent, catch_up_missed, max_catch_up, retry_policy,
                next_trigger_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
            "#,
        )
        .bind(id)
        .bind(&schedule.name)
        .bind(&schedule.description)
        .bind(&schedule.cron_expression)
        .bind(&schedule.timezone)
        .bind(schedule.target_type.to_string())
        .bind(&schedule.target_name)
        .bind(&schedule.target_input)
        .bind(schedule.enabled)
        .bind(schedule.max_concurrent.map(|v| v as i32))
        .bind(schedule.catch_up_missed)
        .bind(schedule.max_catch_up.map(|v| v as i32))
        .bind(&schedule.retry_policy)
        .bind(schedule.next_trigger_at)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to create schedule");
            StoreError::Database(e.to_string())
        })?;

        debug!(%id, name = %schedule.name, "created schedule");
        Ok(id)
    }

    #[instrument(skip(self))]
    async fn get_schedule(&self, id: Uuid) -> Result<ScheduleRow, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT id, name, description, cron_expression, timezone,
                   target_type, target_name, target_input, enabled,
                   max_concurrent, catch_up_missed, max_catch_up, retry_policy,
                   last_triggered_at, next_trigger_at, claimed_by, claimed_at,
                   created_at, updated_at
            FROM durable_schedules
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to get schedule");
            StoreError::Database(e.to_string())
        })?
        .ok_or(StoreError::ScheduleNotFound(id))?;

        parse_schedule_row(row)
    }

    #[instrument(skip(self))]
    async fn list_schedules(
        &self,
        filter: ScheduleFilter,
        pagination: Pagination,
    ) -> Result<Vec<ScheduleRow>, StoreError> {
        let rows = sqlx::query(
            r#"
            SELECT id, name, description, cron_expression, timezone,
                   target_type, target_name, target_input, enabled,
                   max_concurrent, catch_up_missed, max_catch_up, retry_policy,
                   last_triggered_at, next_trigger_at, claimed_by, claimed_at,
                   created_at, updated_at
            FROM durable_schedules
            WHERE ($1::boolean IS NULL OR enabled = $1)
              AND ($2::text IS NULL OR target_type = $2)
            ORDER BY created_at DESC
            OFFSET $3 LIMIT $4
            "#,
        )
        .bind(filter.enabled)
        .bind(
            filter
                .target_type
                .map(|t: ScheduleTargetType| t.to_string()),
        )
        .bind(pagination.offset as i64)
        .bind(pagination.limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to list schedules");
            StoreError::Database(e.to_string())
        })?;

        rows.into_iter().map(parse_schedule_row).collect()
    }

    #[instrument(skip(self))]
    async fn count_schedules(&self, filter: ScheduleFilter) -> Result<u64, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT COUNT(*) as count
            FROM durable_schedules
            WHERE ($1::boolean IS NULL OR enabled = $1)
              AND ($2::text IS NULL OR target_type = $2)
            "#,
        )
        .bind(filter.enabled)
        .bind(
            filter
                .target_type
                .map(|t: ScheduleTargetType| t.to_string()),
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to count schedules");
            StoreError::Database(e.to_string())
        })?;

        let count: i64 = row.get("count");
        Ok(count as u64)
    }

    #[instrument(skip(self, update))]
    async fn update_schedule(&self, id: Uuid, update: UpdateSchedule) -> Result<(), StoreError> {
        // Build dynamic update query
        let result = sqlx::query(
            r#"
            UPDATE durable_schedules
            SET name = COALESCE($2, name),
                description = CASE WHEN $3 THEN $4 ELSE description END,
                cron_expression = COALESCE($5, cron_expression),
                timezone = COALESCE($6, timezone),
                target_type = COALESCE($7, target_type),
                target_name = COALESCE($8, target_name),
                target_input = COALESCE($9, target_input),
                enabled = COALESCE($10, enabled),
                max_concurrent = CASE WHEN $11 THEN $12 ELSE max_concurrent END,
                catch_up_missed = COALESCE($13, catch_up_missed),
                max_catch_up = CASE WHEN $14 THEN $15 ELSE max_catch_up END,
                retry_policy = CASE WHEN $16 THEN $17 ELSE retry_policy END,
                next_trigger_at = CASE WHEN $18 THEN $19 ELSE next_trigger_at END,
                updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(update.name)
        .bind(update.description.is_changed())
        .bind(update.description.into_value())
        .bind(update.cron_expression)
        .bind(update.timezone)
        .bind(
            update
                .target_type
                .map(|t: ScheduleTargetType| t.to_string()),
        )
        .bind(update.target_name)
        .bind(update.target_input)
        .bind(update.enabled)
        .bind(update.max_concurrent.is_changed())
        .bind(update.max_concurrent.into_value().map(|v| v as i32))
        .bind(update.catch_up_missed)
        .bind(update.max_catch_up.is_changed())
        .bind(update.max_catch_up.into_value().map(|v| v as i32))
        .bind(update.retry_policy.is_changed())
        .bind(update.retry_policy.into_value())
        .bind(update.next_trigger_at.is_changed())
        .bind(update.next_trigger_at.into_value())
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to update schedule");
            StoreError::Database(e.to_string())
        })?;

        if result.rows_affected() == 0 {
            return Err(StoreError::ScheduleNotFound(id));
        }

        debug!(%id, "updated schedule");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn delete_schedule(&self, id: Uuid) -> Result<(), StoreError> {
        let result = sqlx::query("DELETE FROM durable_schedules WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to delete schedule");
                StoreError::Database(e.to_string())
            })?;

        if result.rows_affected() == 0 {
            return Err(StoreError::ScheduleNotFound(id));
        }

        debug!(%id, "deleted schedule");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn claim_due_schedules(
        &self,
        scheduler_id: &str,
        limit: u32,
    ) -> Result<Vec<ScheduleRow>, StoreError> {
        // Claim due schedules ordered by next_trigger_at
        let rows = sqlx::query(
            r#"
            WITH due_schedules AS (
                SELECT id
                FROM durable_schedules
                WHERE enabled = true
                  AND next_trigger_at <= NOW()
                  AND (claimed_by IS NULL OR claimed_at < NOW() - INTERVAL '30 seconds')
                ORDER BY next_trigger_at
                FOR UPDATE SKIP LOCKED
                LIMIT $2
            )
            UPDATE durable_schedules s
            SET claimed_by = $1, claimed_at = NOW()
            FROM due_schedules d
            WHERE s.id = d.id
            RETURNING s.id, s.name, s.description, s.cron_expression, s.timezone,
                      s.target_type, s.target_name, s.target_input, s.enabled,
                      s.max_concurrent, s.catch_up_missed, s.max_catch_up, s.retry_policy,
                      s.last_triggered_at, s.next_trigger_at, s.claimed_by, s.claimed_at,
                      s.created_at, s.updated_at
            "#,
        )
        .bind(scheduler_id)
        .bind(limit as i32)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| store_failure("durable.schedules.claim", "claim due schedules failed", e))?;

        let schedules: Result<Vec<_>, _> = rows.into_iter().map(parse_schedule_row).collect();
        let schedules = schedules?;

        debug!(
            count = schedules.len(),
            scheduler_id, "claimed due schedules"
        );
        Ok(schedules)
    }

    #[instrument(skip(self))]
    async fn update_next_trigger(
        &self,
        id: Uuid,
        next: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), StoreError> {
        let result = sqlx::query(
            r#"
            UPDATE durable_schedules
            SET next_trigger_at = $2,
                last_triggered_at = NOW(),
                claimed_by = NULL,
                claimed_at = NULL,
                updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(next)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to update next trigger");
            StoreError::Database(e.to_string())
        })?;

        if result.rows_affected() == 0 {
            return Err(StoreError::ScheduleNotFound(id));
        }

        debug!(%id, %next, "updated next trigger");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn skip_schedule_trigger(&self, id: Uuid) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            UPDATE durable_schedules
            SET claimed_by = NULL, claimed_at = NULL
            WHERE id = $1
            "#,
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to skip schedule trigger");
            StoreError::Database(e.to_string())
        })?;

        debug!(%id, "skipped schedule trigger");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn release_schedule(&self, id: Uuid) -> Result<(), StoreError> {
        self.skip_schedule_trigger(id).await
    }

    #[instrument(skip(self))]
    async fn create_schedule_execution(
        &self,
        schedule_id: Uuid,
        scheduled_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<Uuid, StoreError> {
        let id = Uuid::now_v7();

        sqlx::query(
            r#"
            INSERT INTO durable_schedule_executions (id, schedule_id, scheduled_at, status)
            VALUES ($1, $2, $3, 'running')
            "#,
        )
        .bind(id)
        .bind(schedule_id)
        .bind(scheduled_at)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to create schedule execution");
            StoreError::Database(e.to_string())
        })?;

        debug!(%id, %schedule_id, "created schedule execution");
        Ok(id)
    }

    #[instrument(skip(self))]
    async fn get_schedule_execution(&self, id: Uuid) -> Result<ScheduleExecutionRow, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT id, schedule_id, scheduled_at, started_at, completed_at,
                   status, workflow_id, task_id, error, duration_ms, created_at
            FROM durable_schedule_executions
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to get schedule execution");
            StoreError::Database(e.to_string())
        })?
        .ok_or(StoreError::ScheduleExecutionNotFound(id))?;

        parse_schedule_execution_row(row)
    }

    #[instrument(skip(self))]
    async fn complete_schedule_execution(
        &self,
        execution_id: Uuid,
        target_id: Uuid,
        is_workflow: bool,
    ) -> Result<(), StoreError> {
        let result = sqlx::query(
            r#"
            UPDATE durable_schedule_executions
            SET status = 'completed',
                completed_at = NOW(),
                duration_ms = EXTRACT(MILLISECONDS FROM (NOW() - started_at))::int,
                workflow_id = CASE WHEN $2 THEN $3 ELSE workflow_id END,
                task_id = CASE WHEN NOT $2 THEN $3 ELSE task_id END
            WHERE id = $1
            "#,
        )
        .bind(execution_id)
        .bind(is_workflow)
        .bind(target_id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to complete schedule execution");
            StoreError::Database(e.to_string())
        })?;

        if result.rows_affected() == 0 {
            return Err(StoreError::ScheduleExecutionNotFound(execution_id));
        }

        debug!(%execution_id, "completed schedule execution");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn fail_schedule_execution(
        &self,
        execution_id: Uuid,
        error: &str,
    ) -> Result<(), StoreError> {
        let result = sqlx::query(
            r#"
            UPDATE durable_schedule_executions
            SET status = 'failed',
                completed_at = NOW(),
                duration_ms = EXTRACT(MILLISECONDS FROM (NOW() - started_at))::int,
                error = $2
            WHERE id = $1
            "#,
        )
        .bind(execution_id)
        .bind(error)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to fail schedule execution");
            StoreError::Database(e.to_string())
        })?;

        if result.rows_affected() == 0 {
            return Err(StoreError::ScheduleExecutionNotFound(execution_id));
        }

        debug!(%execution_id, "failed schedule execution");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn skip_schedule_execution(
        &self,
        execution_id: Uuid,
        reason: &str,
    ) -> Result<(), StoreError> {
        let result = sqlx::query(
            r#"
            UPDATE durable_schedule_executions
            SET status = 'skipped',
                completed_at = NOW(),
                duration_ms = EXTRACT(MILLISECONDS FROM (NOW() - started_at))::int,
                error = $2
            WHERE id = $1
            "#,
        )
        .bind(execution_id)
        .bind(reason)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to skip schedule execution");
            StoreError::Database(e.to_string())
        })?;

        if result.rows_affected() == 0 {
            return Err(StoreError::ScheduleExecutionNotFound(execution_id));
        }

        debug!(%execution_id, "skipped schedule execution");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn list_schedule_executions(
        &self,
        filter: ScheduleExecutionFilter,
        pagination: Pagination,
    ) -> Result<Vec<ScheduleExecutionRow>, StoreError> {
        let rows = sqlx::query(
            r#"
            SELECT id, schedule_id, scheduled_at, started_at, completed_at,
                   status, workflow_id, task_id, error, duration_ms, created_at
            FROM durable_schedule_executions
            WHERE ($1::uuid IS NULL OR schedule_id = $1)
              AND ($2::text IS NULL OR status = $2)
            ORDER BY created_at DESC
            OFFSET $3 LIMIT $4
            "#,
        )
        .bind(filter.schedule_id)
        .bind(
            filter
                .status
                .map(|s: ScheduleExecutionStatus| s.to_string()),
        )
        .bind(pagination.offset as i64)
        .bind(pagination.limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to list schedule executions");
            StoreError::Database(e.to_string())
        })?;

        rows.into_iter().map(parse_schedule_execution_row).collect()
    }

    #[instrument(skip(self))]
    async fn count_running_executions(&self, schedule_id: Uuid) -> Result<u32, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT COUNT(*) as count
            FROM durable_schedule_executions
            WHERE schedule_id = $1 AND status = 'running'
            "#,
        )
        .bind(schedule_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to count running executions");
            StoreError::Database(e.to_string())
        })?;

        let count: i64 = row.get("count");
        Ok(count as u32)
    }

    #[instrument(skip(self))]
    async fn get_schedule_stats(&self, schedule_id: Uuid) -> Result<ScheduleStats, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT
                COUNT(*) as total,
                COUNT(*) FILTER (WHERE status = 'completed') as successful,
                COUNT(*) FILTER (WHERE status = 'failed') as failed,
                COUNT(*) FILTER (WHERE status = 'skipped') as skipped,
                AVG(duration_ms) FILTER (WHERE duration_ms IS NOT NULL) as avg_duration
            FROM durable_schedule_executions
            WHERE schedule_id = $1
            "#,
        )
        .bind(schedule_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to get schedule stats");
            StoreError::Database(e.to_string())
        })?;

        let total: i64 = row.get("total");
        let successful: i64 = row.get("successful");
        let failed: i64 = row.get("failed");
        let skipped: i64 = row.get("skipped");
        let avg_duration: Option<f64> = row.get("avg_duration");

        // Get last execution status
        let last_row = sqlx::query(
            r#"
            SELECT status
            FROM durable_schedule_executions
            WHERE schedule_id = $1
            ORDER BY created_at DESC
            LIMIT 1
            "#,
        )
        .bind(schedule_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to get last execution status");
            StoreError::Database(e.to_string())
        })?;

        let last_execution_status = last_row
            .map(|r| {
                let status: String = r.get("status");
                parse_schedule_execution_status(&status)
            })
            .transpose()?;

        Ok(ScheduleStats {
            total_executions: total as u64,
            successful_executions: successful as u64,
            failed_executions: failed as u64,
            skipped_executions: skipped as u64,
            avg_duration_ms: avg_duration.map(|v| v as u64),
            last_execution_status,
        })
    }

    #[instrument(skip(self, instance))]
    async fn register_scheduler_instance(
        &self,
        instance: SchedulerInstanceInfo,
    ) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            INSERT INTO durable_scheduler_instances (instance_id, started_at, last_heartbeat_at, schedules_processed, hostname, version)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (instance_id) DO UPDATE
            SET started_at = $2, last_heartbeat_at = $3, schedules_processed = $4, hostname = $5, version = $6
            "#,
        )
        .bind(&instance.instance_id)
        .bind(instance.started_at)
        .bind(instance.last_heartbeat_at)
        .bind(instance.schedules_processed as i64)
        .bind(&instance.hostname)
        .bind(&instance.version)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to register scheduler instance");
            StoreError::Database(e.to_string())
        })?;

        debug!(instance_id = %instance.instance_id, "registered scheduler instance");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn heartbeat_scheduler_instance(
        &self,
        instance_id: &str,
        schedules_processed: u64,
    ) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            UPDATE durable_scheduler_instances
            SET last_heartbeat_at = NOW(), schedules_processed = $2
            WHERE instance_id = $1
            "#,
        )
        .bind(instance_id)
        .bind(schedules_processed as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to heartbeat scheduler instance");
            StoreError::Database(e.to_string())
        })?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn list_scheduler_instances(&self) -> Result<Vec<SchedulerInstanceInfo>, StoreError> {
        let rows = sqlx::query(
            r#"
            SELECT instance_id, started_at, last_heartbeat_at, schedules_processed, hostname, version
            FROM durable_scheduler_instances
            ORDER BY started_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to list scheduler instances");
            StoreError::Database(e.to_string())
        })?;

        Ok(rows
            .into_iter()
            .map(|row| SchedulerInstanceInfo {
                instance_id: row.get("instance_id"),
                started_at: row.get("started_at"),
                last_heartbeat_at: row.get("last_heartbeat_at"),
                schedules_processed: row.get::<i64, _>("schedules_processed") as u64,
                hostname: row.get("hostname"),
                version: row.get("version"),
            })
            .collect())
    }

    #[instrument(skip(self))]
    async fn deregister_scheduler_instance(&self, instance_id: &str) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            DELETE FROM durable_scheduler_instances WHERE instance_id = $1
            "#,
        )
        .bind(instance_id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to deregister scheduler instance");
            StoreError::Database(e.to_string())
        })?;

        debug!(instance_id, "deregistered scheduler instance");
        Ok(())
    }
}
