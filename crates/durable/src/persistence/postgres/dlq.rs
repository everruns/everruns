//! Implementation of [`crate::DeadLetters`].

use super::*;

#[async_trait]
impl DeadLetters for PostgresWorkflowEventStore {
    #[instrument(skip(self, error_history))]
    async fn move_to_dlq(
        &self,
        task_id: Uuid,
        error_history: Vec<String>,
    ) -> Result<(), StoreError> {
        let error_json = serde_json::to_value(&error_history)
            .map(sanitize_json_null_bytes)
            .map_err(|e| StoreError::Serialization(e.to_string()))?;

        // Get task details and move to DLQ
        sqlx::query(
            r#"
            INSERT INTO durable_dead_letter_queue (
                original_task_id, workflow_id, activity_id, activity_type,
                input, attempts, last_error, error_history
            )
            SELECT id, workflow_id, activity_id, activity_type,
                   input, attempt, COALESCE(last_error, 'unknown'), $2
            FROM durable_task_queue
            WHERE id = $1
            "#,
        )
        .bind(task_id)
        .bind(&error_json)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to move task to DLQ");
            StoreError::Database(e.to_string())
        })?;

        debug!(%task_id, "moved task to DLQ");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn requeue_from_dlq(&self, dlq_id: Uuid) -> Result<Uuid, StoreError> {
        let task_id = Uuid::now_v7();
        let default_options = serde_json::to_value(ActivityOptions::default())
            .map_err(|e| StoreError::Serialization(e.to_string()))?;

        // Create new task from DLQ entry
        let result = sqlx::query(
            r#"
            WITH dlq_entry AS (
                SELECT workflow_id, activity_id, activity_type, input
                FROM durable_dead_letter_queue
                WHERE id = $1
            )
            INSERT INTO durable_task_queue (
                id, workflow_id, activity_id, activity_type, input, options,
                max_attempts, priority,
                schedule_to_start_timeout_ms, start_to_close_timeout_ms
            )
            SELECT $2, workflow_id, activity_id, activity_type, input,
                   $3, 3, 0, 60000, 300000
            FROM dlq_entry
            RETURNING id
            "#,
        )
        .bind(dlq_id)
        .bind(task_id)
        .bind(&default_options)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to requeue from DLQ");
            StoreError::Database(e.to_string())
        })?;

        if result.is_none() {
            return Err(StoreError::TaskNotFound(dlq_id));
        }

        // Update DLQ entry
        sqlx::query(
            r#"
            UPDATE durable_dead_letter_queue
            SET requeued_at = NOW(),
                requeue_count = requeue_count + 1
            WHERE id = $1
            "#,
        )
        .bind(dlq_id)
        .execute(&self.pool)
        .await
        .map_err(|e| StoreError::Database(e.to_string()))?;

        debug!(%dlq_id, %task_id, "requeued task from DLQ");
        Ok(task_id)
    }

    #[instrument(skip(self))]
    async fn list_dlq(
        &self,
        filter: DlqFilter,
        pagination: Pagination,
    ) -> Result<Vec<DlqEntry>, StoreError> {
        let rows = sqlx::query(
            r#"
            SELECT id, original_task_id, workflow_id, activity_id, activity_type,
                   input, attempts, last_error, error_history, dead_at
            FROM durable_dead_letter_queue
            WHERE ($1::uuid IS NULL OR workflow_id = $1)
              AND ($2::text IS NULL OR activity_type = $2)
            ORDER BY dead_at DESC
            OFFSET $3
            LIMIT $4
            "#,
        )
        .bind(filter.workflow_id)
        .bind(&filter.activity_type)
        .bind(pagination.offset as i64)
        .bind(pagination.limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to list DLQ");
            StoreError::Database(e.to_string())
        })?;

        let entries = rows
            .into_iter()
            .map(|row| {
                let error_history_json: serde_json::Value = row.get("error_history");
                let error_history: Vec<String> =
                    serde_json::from_value(error_history_json).unwrap_or_default();

                DlqEntry {
                    id: row.get("id"),
                    original_task_id: row.get("original_task_id"),
                    workflow_id: row.get::<Option<Uuid>, _>("workflow_id"),
                    activity_id: row.get("activity_id"),
                    activity_type: row.get("activity_type"),
                    input: row.get("input"),
                    attempts: row.get::<i32, _>("attempts") as u32,
                    last_error: row.get("last_error"),
                    error_history,
                    dead_at: row.get("dead_at"),
                }
            })
            .collect();

        Ok(entries)
    }
}
