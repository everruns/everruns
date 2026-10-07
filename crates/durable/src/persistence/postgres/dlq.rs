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
                input, attempts, last_error, error_history, options
            )
            SELECT id, workflow_id, activity_id, activity_type,
                   input, attempt, COALESCE(last_error, 'unknown'), $2, options
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
        let db_err = |context: &'static str| {
            move |e: sqlx::Error| {
                error!(error = %e, "{context}");
                StoreError::Database(e.to_string())
            }
        };
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(db_err("Failed to begin DLQ requeue"))?;

        let entry = sqlx::query(
            r#"
            SELECT workflow_id, activity_id, activity_type, input, options
            FROM durable_dead_letter_queue
            WHERE id = $1
            FOR UPDATE
            "#,
        )
        .bind(dlq_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_err("Failed to read DLQ entry"))?
        .ok_or(StoreError::TaskNotFound(dlq_id))?;

        // The task goes back with the options it was enqueued with, so it
        // keeps its queue and retry settings. Rows dead before the options
        // column existed requeue with defaults. A requeue runs now: any start
        // delay applied to the first enqueue only.
        let mut options = entry
            .get::<Option<serde_json::Value>, _>("options")
            .and_then(|value| serde_json::from_value::<ActivityOptions>(value).ok())
            .unwrap_or_default();
        options.start_delay = None;
        let options_json =
            serde_json::to_value(&options).map_err(|e| StoreError::Serialization(e.to_string()))?;

        let task_id = Uuid::now_v7();
        sqlx::query(
            r#"
            INSERT INTO durable_task_queue (
                id, workflow_id, activity_id, activity_type, input, options,
                max_attempts, priority,
                schedule_to_start_timeout_ms, start_to_close_timeout_ms, heartbeat_timeout_ms,
                queue
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            "#,
        )
        .bind(task_id)
        .bind(entry.get::<Option<Uuid>, _>("workflow_id"))
        .bind(entry.get::<String, _>("activity_id"))
        .bind(entry.get::<String, _>("activity_type"))
        .bind(entry.get::<serde_json::Value, _>("input"))
        .bind(&options_json)
        .bind(options.retry_policy.max_attempts as i32)
        .bind(options.priority)
        .bind(options.schedule_to_start_timeout.as_millis() as i64)
        .bind(options.start_to_close_timeout.as_millis() as i64)
        .bind(options.heartbeat_timeout.map(|d| d.as_millis() as i64))
        .bind(options.queue.as_deref())
        .execute(&mut *tx)
        .await
        .map_err(db_err("Failed to requeue from DLQ"))?;

        sqlx::query(
            r#"
            UPDATE durable_dead_letter_queue
            SET requeued_at = NOW(),
                requeue_count = requeue_count + 1
            WHERE id = $1
            "#,
        )
        .bind(dlq_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err("Failed to mark DLQ entry requeued"))?;

        tx.commit()
            .await
            .map_err(db_err("Failed to commit DLQ requeue"))?;

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
