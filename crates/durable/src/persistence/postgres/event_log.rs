//! EventLog implementation (see `store.rs` for the trait contract).

use super::*;

#[async_trait]
impl EventLog for PostgresWorkflowEventStore {
    #[instrument(skip(self, input, trace_context))]
    async fn create_workflow(
        &self,
        workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
        trace_context: Option<&TraceContext>,
    ) -> Result<(), StoreError> {
        let (trace_id, span_id) = trace_context
            .map(|tc| (Some(tc.trace_id.clone()), Some(tc.span_id.clone())))
            .unwrap_or((None, None));

        let input = sanitize_json_null_bytes(input);

        sqlx::query(
            r#"
            INSERT INTO durable_workflow_instances (id, workflow_type, status, input, trace_id, span_id)
            VALUES ($1, $2, 'pending', $3, $4, $5)
            "#,
        )
        .bind(workflow_id)
        .bind(workflow_type)
        .bind(&input)
        .bind(&trace_id)
        .bind(&span_id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to create workflow");
            StoreError::Database(e.to_string())
        })?;

        debug!(%workflow_id, %workflow_type, "created workflow");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn get_workflow_status(&self, workflow_id: Uuid) -> Result<WorkflowStatus, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT status FROM durable_workflow_instances WHERE id = $1
            "#,
        )
        .bind(workflow_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to get workflow status");
            StoreError::Database(e.to_string())
        })?
        .ok_or(StoreError::WorkflowNotFound(workflow_id))?;

        let status: String = row.get("status");
        parse_workflow_status(&status)
    }

    #[instrument(skip(self))]
    async fn get_workflow_info(&self, workflow_id: Uuid) -> Result<WorkflowInfo, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT id, workflow_type, status, input, result, error, continued_as_new_id
            FROM durable_workflow_instances
            WHERE id = $1
            "#,
        )
        .bind(workflow_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to get workflow info");
            StoreError::Database(e.to_string())
        })?
        .ok_or(StoreError::WorkflowNotFound(workflow_id))?;

        let status_str: String = row.get("status");
        let error_json: Option<serde_json::Value> = row.get("error");

        Ok(WorkflowInfo {
            id: row.get("id"),
            workflow_type: row.get("workflow_type"),
            status: parse_workflow_status(&status_str)?,
            input: row.get("input"),
            result: row.get("result"),
            error: error_json.and_then(|v| serde_json::from_value(v).ok()),
            continued_as_new_id: row.try_get("continued_as_new_id").ok().flatten(),
        })
    }

    #[instrument(skip(self, events))]
    async fn append_events(
        &self,
        workflow_id: Uuid,
        expected_sequence: i32,
        events: Vec<WorkflowEvent>,
    ) -> Result<i32, StoreError> {
        if events.is_empty() {
            return Ok(expected_sequence);
        }

        // EVE-639: single multi-row INSERT instead of FOR UPDATE + MAX(...) +
        // one INSERT per event. Sequence numbers are assigned deterministically
        // from `expected_sequence` (the caller's optimistic-concurrency cursor),
        // and the UNIQUE(workflow_id, sequence_num) constraint is the source of
        // truth: if another writer already wrote at any of these sequences, the
        // INSERT fails with a unique violation, which we surface as a
        // ConcurrencyConflict. The FK on workflow_id surfaces WorkflowNotFound.
        let mut new_sequence = expected_sequence;

        let mut builder = sqlx::QueryBuilder::new(
            "INSERT INTO durable_workflow_events (workflow_id, sequence_num, event_type, event_data) ",
        );
        builder.push_values(events.iter(), |mut b, event| {
            let event_type = event_type_name(event);
            // serde_json::to_value is infallible for these event types in
            // practice; on the off chance it fails we bind a null event_data,
            // which the NOT NULL column rejects, turning into a Database error.
            let event_data = serde_json::to_value(event)
                .map(sanitize_json_null_bytes)
                .unwrap_or(serde_json::Value::Null);
            b.push_bind(workflow_id)
                .push_bind(new_sequence)
                .push_bind(event_type)
                .push_bind(event_data);
            new_sequence += 1;
        });

        #[cfg(feature = "failpoints")]
        fail_point!("postgres_append_events_after_insert", |_| {
            Err(StoreError::Database("injected: after insert".into()))
        });

        #[cfg(feature = "failpoints")]
        fail_point!("postgres_append_events_before_commit", |_| {
            Err(StoreError::Database("injected: before commit".into()))
        });

        builder
            .build()
            .execute(&self.pool)
            .await
            .map_err(|e| map_append_events_error(e, workflow_id, expected_sequence))?;

        debug!(%workflow_id, new_sequence, "appended events");
        Ok(new_sequence)
    }

    #[instrument(skip(self))]
    async fn load_events(
        &self,
        workflow_id: Uuid,
    ) -> Result<Vec<(i32, WorkflowEvent)>, StoreError> {
        let rows = sqlx::query(
            r#"
            SELECT sequence_num, event_data
            FROM durable_workflow_events
            WHERE workflow_id = $1
            ORDER BY sequence_num
            "#,
        )
        .bind(workflow_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to load events");
            StoreError::Database(e.to_string())
        })?;

        #[cfg(feature = "failpoints")]
        fail_point!("postgres_load_events_after_query", |_| {
            Err(StoreError::Database(
                "injected: after load events query".into(),
            ))
        });

        let mut events = Vec::with_capacity(rows.len());
        for row in rows {
            let seq: i32 = row.get::<i32, _>("sequence_num");
            let data: serde_json::Value = row.get("event_data");
            let event: WorkflowEvent = serde_json::from_value(data)
                .map_err(|e| StoreError::Serialization(e.to_string()))?;
            events.push((seq, event));
        }

        Ok(events)
    }

    #[instrument(skip(self))]
    async fn count_events(&self, workflow_id: Uuid) -> Result<usize, StoreError> {
        let count = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*)
            FROM durable_workflow_events
            WHERE workflow_id = $1
            "#,
        )
        .bind(workflow_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to count events");
            StoreError::Database(e.to_string())
        })?;

        usize::try_from(count).map_err(|_| {
            StoreError::Database(format!(
                "event count overflow for workflow {workflow_id}: {count}"
            ))
        })
    }

    #[instrument(skip(self))]
    async fn count_events_after(
        &self,
        workflow_id: Uuid,
        after_sequence: i32,
    ) -> Result<usize, StoreError> {
        let count = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*)
            FROM durable_workflow_events
            WHERE workflow_id = $1 AND sequence_num > $2
            "#,
        )
        .bind(workflow_id)
        .bind(after_sequence)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, after_sequence, "Failed to count events after sequence");
            StoreError::Database(e.to_string())
        })?;

        usize::try_from(count).map_err(|_| {
            StoreError::Database(format!(
                "event count overflow for workflow {workflow_id}: {count}"
            ))
        })
    }

    #[instrument(skip(self))]
    async fn load_events_after(
        &self,
        workflow_id: Uuid,
        after_sequence: i32,
    ) -> Result<Vec<(i32, WorkflowEvent)>, StoreError> {
        let rows = sqlx::query(
            r#"
            SELECT sequence_num, event_data
            FROM durable_workflow_events
            WHERE workflow_id = $1 AND sequence_num > $2
            ORDER BY sequence_num
            "#,
        )
        .bind(workflow_id)
        .bind(after_sequence)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, after_sequence, "Failed to load events after sequence");
            StoreError::Database(e.to_string())
        })?;

        let mut events = Vec::with_capacity(rows.len());
        for row in rows {
            let seq: i32 = row.get::<i32, _>("sequence_num");
            let data: serde_json::Value = row.get("event_data");
            let event: WorkflowEvent = serde_json::from_value(data)
                .map_err(|e| StoreError::Serialization(e.to_string()))?;
            events.push((seq, event));
        }

        Ok(events)
    }

    #[instrument(skip(self, snapshot_data))]
    async fn save_snapshot(
        &self,
        workflow_id: Uuid,
        sequence_num: i32,
        snapshot_data: Vec<u8>,
    ) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            INSERT INTO durable_workflow_snapshots (workflow_id, sequence_num, snapshot_data)
            VALUES ($1, $2, $3)
            ON CONFLICT (workflow_id, sequence_num)
            DO UPDATE SET snapshot_data = EXCLUDED.snapshot_data, created_at = NOW()
            "#,
        )
        .bind(workflow_id)
        .bind(sequence_num)
        .bind(&snapshot_data)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to save snapshot");
            StoreError::Database(e.to_string())
        })?;

        debug!(%workflow_id, sequence_num, "saved workflow snapshot");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn load_latest_snapshot(
        &self,
        workflow_id: Uuid,
    ) -> Result<Option<crate::persistence::store::WorkflowSnapshot>, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT sequence_num, snapshot_data, created_at
            FROM durable_workflow_snapshots
            WHERE workflow_id = $1
            ORDER BY sequence_num DESC
            LIMIT 1
            "#,
        )
        .bind(workflow_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to load latest snapshot");
            StoreError::Database(e.to_string())
        })?;

        Ok(row.map(|r| crate::persistence::store::WorkflowSnapshot {
            workflow_id,
            sequence_num: r.get("sequence_num"),
            snapshot_data: r.get("snapshot_data"),
            created_at: r.get("created_at"),
        }))
    }

    #[instrument(skip(self))]
    async fn delete_snapshots(&self, workflow_id: Uuid) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            DELETE FROM durable_workflow_snapshots WHERE workflow_id = $1
            "#,
        )
        .bind(workflow_id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to delete snapshots");
            StoreError::Database(e.to_string())
        })?;

        debug!(%workflow_id, "deleted workflow snapshots");
        Ok(())
    }

    #[instrument(skip(self, result, error))]
    async fn update_workflow_status(
        &self,
        workflow_id: Uuid,
        status: WorkflowStatus,
        result: Option<serde_json::Value>,
        error: Option<WorkflowError>,
    ) -> Result<(), StoreError> {
        let status_str = status.to_string();
        let error_json = error
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| StoreError::Serialization(e.to_string()))?;
        // Pending resets transient fields but retains explicit recovery input.
        let reset_to_pending = matches!(status, WorkflowStatus::Pending);
        let (started_at, completed_at): (Option<DateTime<Utc>>, Option<DateTime<Utc>>) =
            match status {
                WorkflowStatus::Running => (Some(Utc::now()), None),
                WorkflowStatus::Completed
                | WorkflowStatus::Failed
                | WorkflowStatus::Cancelled
                | WorkflowStatus::ContinuedAsNew => (None, Some(Utc::now())),
                WorkflowStatus::Pending => (None, None),
            };
        let result = result.map(sanitize_json_null_bytes);
        let error_json = error_json.map(sanitize_json_null_bytes);
        if reset_to_pending {
            sqlx::query(
                r#"
                UPDATE durable_workflow_instances
                SET status = $2,
                    result = $3,
                    error = NULL,
                    started_at = NULL,
                    completed_at = NULL
                WHERE id = $1
                "#,
            )
            .bind(workflow_id)
            .bind(&status_str)
            .bind(&result)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to update workflow status");
                StoreError::Database(e.to_string())
            })?;
        } else {
            sqlx::query(
                r#"
                UPDATE durable_workflow_instances
                SET status = $2,
                    result = COALESCE($3, result),
                    error = COALESCE($4, error),
                    started_at = COALESCE($5, started_at),
                    completed_at = COALESCE($6, completed_at)
                WHERE id = $1
                "#,
            )
            .bind(workflow_id)
            .bind(&status_str)
            .bind(&result)
            .bind(&error_json)
            .bind(started_at)
            .bind(completed_at)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to update workflow status");
                StoreError::Database(e.to_string())
            })?;
        }

        debug!(%workflow_id, %status_str, "updated workflow status");
        Ok(())
    }

    #[instrument(skip(self, error))]
    async fn try_fail_workflow(
        &self,
        workflow_id: Uuid,
        error: WorkflowError,
    ) -> Result<bool, StoreError> {
        let error_json = sanitize_json_null_bytes(
            serde_json::to_value(error).map_err(|e| StoreError::Serialization(e.to_string()))?,
        );
        let result = sqlx::query(
            r#"
            UPDATE durable_workflow_instances
            SET status = 'failed',
                error = $2,
                completed_at = COALESCE(completed_at, NOW())
            WHERE id = $1 AND status = 'running'
            "#,
        )
        .bind(workflow_id)
        .bind(error_json)
        .execute(&self.pool)
        .await
        .map_err(|e| StoreError::Database(e.to_string()))?;

        Ok(result.rows_affected() == 1)
    }

    #[instrument(skip(self))]
    async fn try_start_new_run(&self, workflow_id: Uuid) -> Result<bool, StoreError> {
        // Atomic CAS: transition from terminal/pending → running.
        // Also cancels stale pending tasks in the same transaction.
        // Uses SELECT FOR UPDATE to prevent concurrent claims across replicas.
        let mut tx = self.pool.begin().await.map_err(|e| {
            error!(error = %e, "Failed to begin transaction");
            StoreError::Database(e.to_string())
        })?;

        // Lock the workflow row and check status
        let row = sqlx::query(
            r#"
            SELECT status FROM durable_workflow_instances
            WHERE id = $1
            FOR UPDATE
            "#,
        )
        .bind(workflow_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to lock workflow for claim");
            StoreError::Database(e.to_string())
        })?;

        let Some(row) = row else {
            tx.rollback().await.ok();
            return Err(StoreError::WorkflowNotFound(workflow_id));
        };

        let status: String = row.get("status");
        if status == "running" {
            // Active run — cannot claim; the caller should signal the run instead
            tx.rollback().await.ok();
            return Ok(false);
        }

        let has_claimed_task = sqlx::query_scalar::<_, bool>(
            r#"
            SELECT EXISTS(
                SELECT 1
                FROM durable_task_queue
                WHERE workflow_id = $1
                  AND status = 'claimed'
            )
            "#,
        )
        .bind(workflow_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to check claimed workflow tasks");
            StoreError::Database(e.to_string())
        })?;

        if has_claimed_task {
            tx.rollback().await.ok();
            return Ok(false);
        }

        // Claim: set to running, clear transient fields
        sqlx::query(
            r#"
            UPDATE durable_workflow_instances
            SET status = 'running',
                result = NULL,
                error = NULL,
                started_at = NOW(),
                completed_at = NULL
            WHERE id = $1
            "#,
        )
        .bind(workflow_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to update workflow to running");
            StoreError::Database(e.to_string())
        })?;

        // Cancel any stale pending tasks from the previous run
        sqlx::query(
            r#"
            UPDATE durable_task_queue
            SET status = 'cancelled'
            WHERE workflow_id = $1 AND status = 'pending'
            "#,
        )
        .bind(workflow_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to cancel stale tasks during claim");
            StoreError::Database(e.to_string())
        })?;

        tx.commit().await.map_err(|e| {
            error!(error = %e, "Failed to commit workflow claim");
            StoreError::Database(e.to_string())
        })?;

        Ok(true)
    }

    #[instrument(skip(self, input, task))]
    async fn start_run_with_task(
        &self,
        workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
        mut task: TaskDefinition,
    ) -> Result<RunStart, StoreError> {
        task.workflow_id = Some(workflow_id);
        let db = |action: &'static str| {
            move |e: sqlx::Error| {
                error!(error = %e, "{action}");
                StoreError::Database(e.to_string())
            }
        };
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(db("Failed to begin run start"))?;

        // Lock the workflow row (or learn it is missing) and see whether a run
        // is active, in one round trip. A start that loses the create race
        // falls through to the second pass and finds the winner's run.
        let mut started = None;
        for _ in 0..2 {
            let row: Option<(String, bool)> = sqlx::query_as(
                r#"
                SELECT w.status,
                       EXISTS(
                           SELECT 1 FROM durable_task_queue t
                           WHERE t.workflow_id = w.id AND t.status = 'claimed'
                       )
                FROM durable_workflow_instances w
                WHERE w.id = $1
                FOR UPDATE OF w
                "#,
            )
            .bind(workflow_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db("Failed to lock workflow for run start"))?;

            match row {
                None => {
                    if insert_started_workflow(
                        &mut tx,
                        workflow_id,
                        workflow_type,
                        input.clone(),
                        &task,
                    )
                    .await?
                    {
                        started = Some(true);
                        break;
                    }
                }
                Some((status, has_claimed_task)) => {
                    if status == "running" || has_claimed_task {
                        tx.rollback().await.ok();
                        return Ok(RunStart::Active);
                    }
                    // New run: reset the workflow and cancel the previous
                    // run's stale pending tasks, as `try_start_new_run` does.
                    sqlx::query(
                        r#"
                        WITH reset AS (
                            UPDATE durable_workflow_instances
                            SET status = 'running',
                                result = NULL,
                                error = NULL,
                                started_at = NOW(),
                                completed_at = NULL
                            WHERE id = $1
                        )
                        UPDATE durable_task_queue
                        SET status = 'cancelled'
                        WHERE workflow_id = $1 AND status = 'pending'
                        "#,
                    )
                    .bind(workflow_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(db("Failed to reset workflow for new run"))?;
                    started = Some(false);
                    break;
                }
            }
        }

        let Some(created) = started else {
            // Only reachable if the workflow vanished between the two passes.
            tx.rollback().await.ok();
            return Err(StoreError::WorkflowNotFound(workflow_id));
        };

        // Stale pending tasks were just cancelled (or the workflow is new),
        // so the per-workflow pending cap cannot be hit here.
        let task_id = insert_workflow_task(&mut tx, workflow_id, &task).await?;
        tx.commit()
            .await
            .map_err(db("Failed to commit run start"))?;
        debug!(%workflow_id, %task_id, created, "started workflow run");
        Ok(RunStart::Started { task_id, created })
    }

    #[instrument(skip(self))]
    async fn cancel_workflow(&self, workflow_id: Uuid) -> Result<(), StoreError> {
        // Update workflow status to cancelled
        let result = sqlx::query(
            r#"
            UPDATE durable_workflow_instances
            SET status = 'cancelled',
                completed_at = NOW()
            WHERE id = $1 AND status IN ('pending', 'running')
            RETURNING id
            "#,
        )
        .bind(workflow_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to cancel workflow");
            StoreError::Database(e.to_string())
        })?;

        if result.is_none() {
            return Err(StoreError::WorkflowNotFound(workflow_id));
        }

        // Cancel any pending tasks for this workflow
        sqlx::query(
            r#"
            UPDATE durable_task_queue
            SET status = 'cancelled'
            WHERE workflow_id = $1 AND status = 'pending'
            "#,
        )
        .bind(workflow_id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to cancel workflow tasks");
            StoreError::Database(e.to_string())
        })?;

        debug!(%workflow_id, "cancelled workflow");
        Ok(())
    }

    #[instrument(skip(self, snapshot_data))]
    async fn continue_as_new(
        &self,
        old_workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
        snapshot_data: Vec<u8>,
    ) -> Result<Uuid, StoreError> {
        let new_workflow_id = Uuid::now_v7();
        let input = sanitize_json_null_bytes(input);
        let start_event = WorkflowEvent::started(input.clone());
        let event_data = serde_json::to_value(&start_event)
            .map(sanitize_json_null_bytes)
            .map_err(|e| StoreError::Serialization(e.to_string()))?;

        let mut tx = self.pool.begin().await.map_err(|e| {
            error!(error = %e, "Failed to begin transaction");
            StoreError::Database(e.to_string())
        })?;

        // 1. Mark old workflow as continued_as_new
        let result = sqlx::query(
            r#"
            UPDATE durable_workflow_instances
            SET status = 'continued_as_new',
                completed_at = NOW(),
                continued_as_new_id = $2
            WHERE id = $1 AND status IN ('pending', 'running')
            RETURNING id
            "#,
        )
        .bind(old_workflow_id)
        .bind(new_workflow_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to mark workflow as continued");
            StoreError::Database(e.to_string())
        })?;

        if result.is_none() {
            return Err(StoreError::WorkflowNotFound(old_workflow_id));
        }

        // 2. Delete old workflow events (archive)
        sqlx::query(r#"DELETE FROM durable_workflow_events WHERE workflow_id = $1"#)
            .bind(old_workflow_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to delete old events");
                StoreError::Database(e.to_string())
            })?;

        // 3. Delete old snapshots
        sqlx::query(r#"DELETE FROM durable_workflow_snapshots WHERE workflow_id = $1"#)
            .bind(old_workflow_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to delete old snapshots");
                StoreError::Database(e.to_string())
            })?;

        // 4. Create new workflow
        sqlx::query(
            r#"
            INSERT INTO durable_workflow_instances (id, workflow_type, status, input, started_at)
            VALUES ($1, $2, 'running', $3, NOW())
            "#,
        )
        .bind(new_workflow_id)
        .bind(workflow_type)
        .bind(&input)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to create new workflow");
            StoreError::Database(e.to_string())
        })?;

        // 5. Write WorkflowStarted event on new workflow
        sqlx::query(
            r#"
            INSERT INTO durable_workflow_events (workflow_id, sequence_num, event_type, event_data)
            VALUES ($1, 0, 'workflow_started', $2)
            "#,
        )
        .bind(new_workflow_id)
        .bind(&event_data)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to write start event");
            StoreError::Database(e.to_string())
        })?;

        // 6. Save snapshot on new workflow at sequence 0
        sqlx::query(
            r#"
            INSERT INTO durable_workflow_snapshots (workflow_id, sequence_num, snapshot_data)
            VALUES ($1, 0, $2)
            "#,
        )
        .bind(new_workflow_id)
        .bind(&snapshot_data)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to save snapshot");
            StoreError::Database(e.to_string())
        })?;

        tx.commit().await.map_err(|e| {
            error!(error = %e, "Failed to commit continue_as_new");
            StoreError::Database(e.to_string())
        })?;

        info!(
            %old_workflow_id,
            %new_workflow_id,
            "continued workflow as new"
        );

        Ok(new_workflow_id)
    }
}
