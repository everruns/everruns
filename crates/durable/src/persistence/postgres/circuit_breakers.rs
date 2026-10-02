//! CircuitBreakers implementation (see `store.rs` for the trait contract).

use super::*;

#[async_trait]
impl CircuitBreakers for PostgresWorkflowEventStore {
    #[instrument(skip(self, config))]
    async fn create_circuit_breaker(
        &self,
        key: &str,
        config: &CircuitBreakerConfig,
    ) -> Result<(), StoreError> {
        // Use INSERT ... ON CONFLICT DO NOTHING to handle race conditions
        // when multiple workers try to create the same circuit breaker
        sqlx::query(
            r#"
            INSERT INTO durable_circuit_breaker_state (key, state, failure_count, success_count, updated_at)
            VALUES ($1, 'closed', 0, 0, NOW())
            ON CONFLICT (key) DO NOTHING
            "#,
        )
        .bind(key)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to create circuit breaker");
            StoreError::Database(e.to_string())
        })?;

        debug!(
            key,
            failure_threshold = config.failure_threshold,
            "created circuit breaker"
        );
        Ok(())
    }

    #[instrument(skip(self))]
    async fn get_circuit_breaker(
        &self,
        key: &str,
    ) -> Result<Option<CircuitBreakerState>, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT key, state, failure_count, success_count,
                   last_failure_at, opened_at, half_open_at, updated_at
            FROM durable_circuit_breaker_state
            WHERE key = $1
            "#,
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to get circuit breaker");
            StoreError::Database(e.to_string())
        })?;

        match row {
            Some(row) => {
                let state_str: String = row.get("state");
                let state = parse_circuit_state(&state_str)?;

                Ok(Some(CircuitBreakerState {
                    key: row.get("key"),
                    state,
                    failure_count: row.get::<i32, _>("failure_count") as u32,
                    success_count: row.get::<i32, _>("success_count") as u32,
                    last_failure_at: row.get("last_failure_at"),
                    opened_at: row.get("opened_at"),
                    half_open_at: row.get("half_open_at"),
                    updated_at: row.get("updated_at"),
                }))
            }
            None => Ok(None),
        }
    }

    #[instrument(skip(self))]
    async fn update_circuit_breaker(
        &self,
        key: &str,
        state: CircuitState,
        failure_count: u32,
        success_count: u32,
    ) -> Result<(), StoreError> {
        let state_str = state.to_string();

        // Build the query dynamically based on state transitions
        let (opened_at_update, half_open_at_update, last_failure_update) = match state {
            CircuitState::Open => (
                "NOW()",
                "NULL",
                if failure_count > 0 {
                    "NOW()"
                } else {
                    "last_failure_at"
                },
            ),
            CircuitState::HalfOpen => ("opened_at", "NOW()", "last_failure_at"),
            CircuitState::Closed => ("NULL", "NULL", "NULL"),
        };

        let query = format!(
            r#"
            UPDATE durable_circuit_breaker_state
            SET state = $2,
                failure_count = $3,
                success_count = $4,
                opened_at = {},
                half_open_at = {},
                last_failure_at = {},
                updated_at = NOW()
            WHERE key = $1
            "#,
            opened_at_update, half_open_at_update, last_failure_update
        );

        // SAFETY: opened_at_update, half_open_at_update, last_failure_update are
        // server-chosen literals from a closed set (`"NOW()"`, `"NULL"`,
        // `"last_failure_at"`, `"opened_at"`); no caller input flows into the
        // format string.
        sqlx::query(sqlx::AssertSqlSafe(query))
            .bind(key)
            .bind(&state_str)
            .bind(failure_count as i32)
            .bind(success_count as i32)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to update circuit breaker");
                StoreError::Database(e.to_string())
            })?;

        debug!(key, %state_str, failure_count, success_count, "updated circuit breaker");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn list_circuit_breakers(&self) -> Result<Vec<CircuitBreakerState>, StoreError> {
        let rows = sqlx::query(
            r#"
            SELECT key, state, failure_count, success_count,
                   last_failure_at, opened_at, half_open_at, updated_at
            FROM durable_circuit_breaker_state
            ORDER BY key
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to list circuit breakers");
            StoreError::Database(e.to_string())
        })?;

        let mut breakers = Vec::with_capacity(rows.len());
        for row in rows {
            let state_str: String = row.get("state");
            breakers.push(CircuitBreakerState {
                key: row.get("key"),
                state: parse_circuit_state(&state_str)?,
                failure_count: row.get::<i32, _>("failure_count") as u32,
                success_count: row.get::<i32, _>("success_count") as u32,
                last_failure_at: row.get("last_failure_at"),
                opened_at: row.get("opened_at"),
                half_open_at: row.get("half_open_at"),
                updated_at: row.get("updated_at"),
            });
        }

        Ok(breakers)
    }

    #[instrument(skip(self))]
    async fn force_open_circuit_breaker(&self, key: &str) -> Result<(), StoreError> {
        let result = sqlx::query(
            r#"
            UPDATE durable_circuit_breaker_state
            SET state = 'open',
                failure_count = 0,
                success_count = 0,
                opened_at = NOW(),
                updated_at = NOW()
            WHERE key = $1
            "#,
        )
        .bind(key)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to force open circuit breaker");
            StoreError::Database(e.to_string())
        })?;

        if result.rows_affected() == 0 {
            // Circuit breaker doesn't exist, create it in open state
            sqlx::query(
                r#"
                INSERT INTO durable_circuit_breaker_state
                    (key, state, failure_count, success_count, opened_at, updated_at)
                VALUES ($1, 'open', 0, 0, NOW(), NOW())
                "#,
            )
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to create circuit breaker in open state");
                StoreError::Database(e.to_string())
            })?;
        }

        info!(key, "force opened circuit breaker");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn force_close_circuit_breaker(&self, key: &str) -> Result<(), StoreError> {
        let result = sqlx::query(
            r#"
            UPDATE durable_circuit_breaker_state
            SET state = 'closed',
                failure_count = 0,
                success_count = 0,
                opened_at = NULL,
                half_open_at = NULL,
                updated_at = NOW()
            WHERE key = $1
            "#,
        )
        .bind(key)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to force close circuit breaker");
            StoreError::Database(e.to_string())
        })?;

        if result.rows_affected() == 0 {
            return Err(StoreError::CircuitBreakerNotFound(key.to_string()));
        }

        info!(key, "force closed circuit breaker");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn delete_circuit_breaker(&self, key: &str) -> Result<(), StoreError> {
        let result = sqlx::query(
            r#"
            DELETE FROM durable_circuit_breaker_state
            WHERE key = $1
            "#,
        )
        .bind(key)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to delete circuit breaker");
            StoreError::Database(e.to_string())
        })?;

        if result.rows_affected() == 0 {
            return Err(StoreError::CircuitBreakerNotFound(key.to_string()));
        }

        info!(key, "deleted circuit breaker");
        Ok(())
    }
}
