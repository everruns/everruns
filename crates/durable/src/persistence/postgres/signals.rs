//! SignalStore implementation (see `store.rs` for the trait contract).

use super::*;

#[async_trait]
impl SignalStore for PostgresWorkflowEventStore {
    #[instrument(skip(self, signal))]
    async fn send_signal(
        &self,
        workflow_id: Uuid,
        signal: WorkflowSignal,
    ) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            INSERT INTO durable_signals (workflow_id, signal_type, payload, sent_at)
            VALUES ($1, $2, $3, $4)
            "#,
        )
        .bind(workflow_id)
        .bind(&signal.signal_type)
        .bind(&signal.payload)
        .bind(signal.sent_at)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to send signal");
            StoreError::Database(e.to_string())
        })?;

        debug!(%workflow_id, signal_type = %signal.signal_type, "sent signal");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn get_pending_signals(
        &self,
        workflow_id: Uuid,
    ) -> Result<Vec<WorkflowSignal>, StoreError> {
        let rows = sqlx::query(
            r#"
            SELECT signal_type, payload, sent_at
            FROM durable_signals
            WHERE workflow_id = $1 AND processed_at IS NULL
            ORDER BY sequence_num
            "#,
        )
        .bind(workflow_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to get pending signals");
            StoreError::Database(e.to_string())
        })?;

        let signals = rows
            .into_iter()
            .map(|row| WorkflowSignal {
                signal_type: row.get("signal_type"),
                payload: row.get("payload"),
                sent_at: row.get("sent_at"),
            })
            .collect();

        Ok(signals)
    }

    #[instrument(skip(self))]
    async fn mark_signals_processed(
        &self,
        workflow_id: Uuid,
        count: usize,
    ) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            UPDATE durable_signals
            SET processed_at = NOW()
            WHERE id IN (
                SELECT id FROM durable_signals
                WHERE workflow_id = $1 AND processed_at IS NULL
                ORDER BY sequence_num
                LIMIT $2
            )
            "#,
        )
        .bind(workflow_id)
        .bind(count as i32)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to mark signals processed");
            StoreError::Database(e.to_string())
        })?;

        debug!(%workflow_id, count, "marked signals as processed");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn consume_pending_signals(
        &self,
        workflow_id: Uuid,
    ) -> Result<Vec<WorkflowSignal>, StoreError> {
        // Atomic: UPDATE ... RETURNING in a single statement.
        // Marks all pending signals as processed and returns them.
        let rows = sqlx::query(
            r#"
            UPDATE durable_signals
            SET processed_at = NOW()
            WHERE id IN (
                SELECT id FROM durable_signals
                WHERE workflow_id = $1 AND processed_at IS NULL
                ORDER BY sequence_num
                FOR UPDATE
            )
            RETURNING signal_type, payload, sent_at
            "#,
        )
        .bind(workflow_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to consume pending signals");
            StoreError::Database(e.to_string())
        })?;

        let signals = rows
            .into_iter()
            .map(|row| WorkflowSignal {
                signal_type: row.get("signal_type"),
                payload: row.get("payload"),
                sent_at: row.get("sent_at"),
            })
            .collect::<Vec<_>>();

        debug!(%workflow_id, count = signals.len(), "consumed pending signals atomically");
        Ok(signals)
    }

    #[instrument(skip(self))]
    async fn consume_pending_signals_by_type(
        &self,
        workflow_id: Uuid,
        signal_type: &str,
    ) -> Result<Vec<WorkflowSignal>, StoreError> {
        let rows = sqlx::query(
            r#"
            UPDATE durable_signals
            SET processed_at = NOW()
            WHERE id IN (
                SELECT id FROM durable_signals
                WHERE workflow_id = $1 AND processed_at IS NULL AND signal_type = $2
                ORDER BY sequence_num
                FOR UPDATE
            )
            RETURNING signal_type, payload, sent_at
            "#,
        )
        .bind(workflow_id)
        .bind(signal_type)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to consume pending signals by type");
            StoreError::Database(e.to_string())
        })?;

        let signals = rows
            .into_iter()
            .map(|row| WorkflowSignal {
                signal_type: row.get("signal_type"),
                payload: row.get("payload"),
                sent_at: row.get("sent_at"),
            })
            .collect::<Vec<_>>();

        debug!(%workflow_id, count = signals.len(), %signal_type, "consumed pending signals by type atomically");
        Ok(signals)
    }
}
