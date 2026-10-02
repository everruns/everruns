//! TaskQueue implementation (see `store.rs` for the trait contract).

use super::*;

#[async_trait]
impl TaskQueue for PostgresWorkflowEventStore {
    #[instrument(skip(self, task))]
    async fn enqueue_task(&self, task: TaskDefinition) -> Result<Uuid, StoreError> {
        // Check pending task limits: per-workflow for workflow tasks, global for standalone
        if let Some(wf_id) = task.workflow_id {
            let limit = self.max_pending_tasks_per_workflow;
            let pending_count: i64 = sqlx::query_scalar(
                r#"
                SELECT CASE WHEN $3 AND EXISTS (
                    SELECT 1 FROM durable_task_queue WHERE workflow_id = $1 AND activity_id = $2
                ) THEN 0 ELSE COUNT(*) END FROM durable_task_queue
                WHERE workflow_id = $1 AND status = 'pending'
                "#,
            )
            .bind(wf_id)
            .bind(&task.activity_id)
            .bind(task.options.dedupe_by_activity_id)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to count pending tasks");
                StoreError::Database(e.to_string())
            })?;
            if pending_count >= limit as i64 {
                return Err(StoreError::TaskQueueLimitExceeded {
                    workflow_id: wf_id,
                    current: pending_count as u32,
                    limit,
                });
            }
        } else {
            // Standalone task: check global standalone limit
            let limit = crate::persistence::store::DEFAULT_MAX_PENDING_STANDALONE_TASKS;
            let pending_count: i64 = sqlx::query_scalar(
                r#"
                SELECT COUNT(*) FROM durable_task_queue
                WHERE workflow_id IS NULL AND status = 'pending'
                "#,
            )
            .fetch_one(&self.pool)
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to count standalone pending tasks");
                StoreError::Database(e.to_string())
            })?;

            if pending_count >= limit as i64 {
                return Err(StoreError::StandaloneTaskQueueLimitExceeded {
                    current: pending_count as u32,
                    limit,
                });
            }
        }
        let task_id = Uuid::now_v7();
        let task_input = sanitize_json_null_bytes(task.input.clone());
        let options_json = serde_json::to_value(&task.options)
            .map(sanitize_json_null_bytes)
            .map_err(|e| StoreError::Serialization(e.to_string()))?;

        // A dedupe task returns the existing `(workflow_id, activity_id)` row
        // instead of inserting a second one. `ON CONFLICT DO NOTHING` names no
        // arbiter so any unique index over those ids (the deployment decides
        // which, see `ActivityOptions::dedupe_by_activity_id`) settles a race;
        // the loser then reads the winner's committed row.
        let dedupe = task.options.dedupe_by_activity_id && task.workflow_id.is_some();
        let inserted: Option<Uuid> = sqlx::query_scalar(if dedupe {
            r#"
            INSERT INTO durable_task_queue (
                id, workflow_id, activity_id, activity_type, input, options,
                max_attempts, priority, visible_at,
                schedule_to_start_timeout_ms, start_to_close_timeout_ms, heartbeat_timeout_ms
            )
            SELECT $1, $2, $3, $4, $5, $6, $7, $8, NOW() + $12::bigint * INTERVAL '1 millisecond', $9, $10, $11
            WHERE NOT EXISTS (
                SELECT 1 FROM durable_task_queue WHERE workflow_id = $2 AND activity_id = $3
            )
            ON CONFLICT DO NOTHING
            RETURNING id
            "#
        } else {
            r#"
            INSERT INTO durable_task_queue (
                id, workflow_id, activity_id, activity_type, input, options,
                max_attempts, priority, visible_at,
                schedule_to_start_timeout_ms, start_to_close_timeout_ms, heartbeat_timeout_ms
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW() + $12::bigint * INTERVAL '1 millisecond', $9, $10, $11)
            RETURNING id
            "#
        })
        .bind(task_id)
        .bind(task.workflow_id)
        .bind(&task.activity_id)
        .bind(&task.activity_type)
        .bind(&task_input)
        .bind(&options_json)
        .bind(task.options.retry_policy.max_attempts as i32)
        .bind(task.options.priority)
        .bind(task.options.schedule_to_start_timeout.as_millis() as i64)
        .bind(task.options.start_to_close_timeout.as_millis() as i64)
        .bind(task.options.heartbeat_timeout.map(|d| d.as_millis() as i64))
        .bind(task.options.start_delay.map_or(0, |d| d.as_millis() as i64))
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to enqueue task");
            StoreError::Database(e.to_string())
        })?;

        let task_id = match inserted {
            Some(task_id) => task_id,
            None => sqlx::query_scalar(
                r#"
                SELECT id FROM durable_task_queue
                WHERE workflow_id = $1 AND activity_id = $2
                ORDER BY id
                LIMIT 1
                "#,
            )
            .bind(task.workflow_id)
            .bind(&task.activity_id)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to read deduplicated task");
                StoreError::Database(e.to_string())
            })?,
        };

        #[cfg(feature = "failpoints")]
        fail_point!("postgres_enqueue_task_after_insert", |_| {
            Err(StoreError::Database(
                "injected: after enqueue insert".into(),
            ))
        });

        debug!(%task_id, activity_type = %task.activity_type, "enqueued task");
        Ok(task_id)
    }

    #[instrument(skip(self, activity_types))]
    async fn claim_task(
        &self,
        worker_id: &str,
        activity_types: &[String],
        max_tasks: usize,
    ) -> Result<Vec<ClaimedTask>, StoreError> {
        if activity_types.is_empty() {
            return Ok(vec![]);
        }

        // Use SKIP LOCKED for efficient concurrent claiming.
        // This transaction:
        // 1. Verifies worker is not draining (early exit if draining)
        // 2. Finds pending tasks matching activity types
        // 3. Orders by priority (desc) then visibility time
        // 4. Limits to max_tasks
        // 5. Uses SKIP LOCKED to avoid contention
        // 6. Updates status and claiming info
        // 7. Appends ActivityStarted events for FIRST-attempt workflow tasks
        //    before returning the claim, avoiding a separate pre-execution write
        //    in the worker. Reclaims (attempt > 1) skip this write (EVE-639).
        //
        // The ActivityStarted sequence read is deliberately a separate statement
        // after locking the workflow rows. Under READ COMMITTED, a single-statement
        // CTE keeps its original snapshot even after waiting on row locks, which
        // can duplicate sequence numbers during concurrent claims for one workflow.
        // EVE-639 makes this set-based: one ANY($) lock, one grouped MAX(seq)
        // read, and one multi-row INSERT, rather than three statements per
        // workflow.
        // NOTE: The `attempt < max_attempts` check is critical for preventing infinite
        // retries when workers panic. Without fail_task being called, the task becomes
        // stale, gets reclaimed, but must not be claimed if attempts are exhausted.
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

        let rows = sqlx::query(
            r#"
            WITH worker_check AS (
                SELECT 1 FROM durable_workers
                WHERE id = $3 AND status != 'draining'
            ),
            claimable AS (
                SELECT tq.id
                FROM durable_task_queue tq, worker_check
                WHERE tq.status = 'pending'
                  AND tq.activity_type = ANY($1)
                  AND tq.visible_at <= NOW()
                  AND tq.attempt < tq.max_attempts
                ORDER BY tq.priority DESC, tq.visible_at
                LIMIT $2
                FOR UPDATE SKIP LOCKED
            ),
            updated AS (
                UPDATE durable_task_queue t
                SET status = 'claimed',
                    claimed_by = $3,
                    claimed_at = NOW(),
                    heartbeat_at = NOW(),
                    attempt = attempt + 1
                FROM claimable c
                WHERE t.id = c.id
                RETURNING t.id, t.workflow_id, t.activity_id, t.activity_type,
                          t.input, t.options, t.attempt, t.max_attempts
            )
            SELECT id, workflow_id, activity_id, activity_type,
                   input, options, attempt, max_attempts
            FROM updated
            "#,
        )
        .bind(activity_types)
        .bind(max_tasks as i32)
        .bind(worker_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| store_failure("durable.tasks.claim", "Failed to claim tasks", e))?;

        let mut claimed = Vec::with_capacity(rows.len());
        // EVE-639: only record ActivityStarted on the FIRST attempt of a task,
        // not on every reclaim. `attempt` is post-increment in the claim UPDATE
        // above, so the first claim yields attempt == 1. Reclaims (attempt > 1)
        // record no ActivityStarted event — this is genuine bookkeeping for the
        // initial dispatch only. (The seal guard in reclaim_stale_tasks still
        // excludes activity_started from its progress token; see EVE-534.)
        let mut started_by_workflow: BTreeMap<Uuid, Vec<(Uuid, String, i32)>> = BTreeMap::new();
        for row in rows {
            let options_json: serde_json::Value = row.get("options");
            let options: ActivityOptions = serde_json::from_value(options_json)
                .map_err(|e| StoreError::Serialization(e.to_string()))?;
            let task_id: Uuid = row.get("id");
            let workflow_id = row.get::<Option<Uuid>, _>("workflow_id");
            let activity_id: String = row.get("activity_id");
            let attempt = row.get::<i32, _>("attempt");

            claimed.push(ClaimedTask {
                id: task_id,
                workflow_id,
                activity_id: activity_id.clone(),
                activity_type: row.get("activity_type"),
                input: row.get("input"),
                options,
                attempt: attempt as u32,
                max_attempts: row.get::<i32, _>("max_attempts") as u32,
            });

            if attempt == 1
                && let Some(workflow_id) = workflow_id
            {
                started_by_workflow.entry(workflow_id).or_default().push((
                    task_id,
                    activity_id,
                    attempt,
                ));
            }
        }

        if !started_by_workflow.is_empty() {
            // Set-based ActivityStarted persistence: lock all involved workflow
            // rows in one statement, fetch each workflow's next sequence in one
            // grouped statement, then write every ActivityStarted event in a
            // single multi-row INSERT — instead of three statements per workflow.
            // BTreeMap keeps workflow_ids ordered, which gives a deterministic
            // FOR UPDATE lock order across concurrent claimers, avoiding
            // deadlocks.
            let workflow_ids: Vec<Uuid> = started_by_workflow.keys().copied().collect();

            sqlx::query(
                r#"
                SELECT id FROM durable_workflow_instances
                WHERE id = ANY($1)
                ORDER BY id
                FOR UPDATE
                "#,
            )
            .bind(&workflow_ids)
            .fetch_all(&mut *tx)
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to lock workflows for activity start events");
                StoreError::Database(e.to_string())
            })?;

            let seq_rows = sqlx::query(
                r#"
                SELECT workflow_id, COALESCE(MAX(sequence_num) + 1, 0)::INTEGER AS next_seq
                FROM durable_workflow_events
                WHERE workflow_id = ANY($1)
                GROUP BY workflow_id
                "#,
            )
            .bind(&workflow_ids)
            .fetch_all(&mut *tx)
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to get workflow event sequences");
                StoreError::Database(e.to_string())
            })?;

            let mut next_seq_by_workflow: BTreeMap<Uuid, i32> = BTreeMap::new();
            for row in seq_rows {
                next_seq_by_workflow.insert(row.get("workflow_id"), row.get::<i32, _>("next_seq"));
            }

            // Build the rows for a single multi-row INSERT, assigning contiguous
            // sequence numbers per workflow. Tasks within a workflow are ordered
            // by task_id for deterministic sequencing.
            struct StartedEventRow {
                workflow_id: Uuid,
                sequence_num: i32,
                event_data: serde_json::Value,
            }
            let mut insert_rows: Vec<StartedEventRow> = Vec::new();
            for (workflow_id, mut events) in started_by_workflow {
                events.sort_by_key(|(task_id, _, _)| *task_id);
                // A workflow that has events (it always does — WorkflowStarted is
                // seq 0) returns a row; default to 0 only as a safety net.
                let base_sequence = next_seq_by_workflow.get(&workflow_id).copied().unwrap_or(0);
                for (offset, (_, activity_id, attempt)) in events.into_iter().enumerate() {
                    let event_data = serde_json::to_value(WorkflowEvent::ActivityStarted {
                        activity_id,
                        attempt: attempt as u32,
                        worker_id: worker_id.to_string(),
                    })
                    .map(sanitize_json_null_bytes)
                    .map_err(|e| StoreError::Serialization(e.to_string()))?;
                    insert_rows.push(StartedEventRow {
                        workflow_id,
                        sequence_num: base_sequence + offset as i32,
                        event_data,
                    });
                }
            }

            if !insert_rows.is_empty() {
                let mut builder = sqlx::QueryBuilder::new(
                    "INSERT INTO durable_workflow_events (workflow_id, sequence_num, event_type, event_data) ",
                );
                builder.push_values(insert_rows.iter(), |mut b, r| {
                    b.push_bind(r.workflow_id)
                        .push_bind(r.sequence_num)
                        .push_bind("activity_started")
                        .push_bind(&r.event_data);
                });
                builder.build().execute(&mut *tx).await.map_err(|e| {
                    error!(error = %e, "Failed to write activity start events");
                    StoreError::Database(e.to_string())
                })?;
            }
        }

        tx.commit().await.map_err(|e| {
            error!(error = %e, "Failed to commit task claim");
            StoreError::Database(e.to_string())
        })?;

        #[cfg(feature = "failpoints")]
        fail_point!("postgres_claim_task_after_query", |_| {
            Err(StoreError::Database("injected: after claim query".into()))
        });

        if !claimed.is_empty() {
            debug!(worker_id, count = claimed.len(), "claimed tasks");
        }

        Ok(claimed)
    }

    #[instrument(skip(self, _details))]
    async fn heartbeat_task(
        &self,
        task_id: Uuid,
        worker_id: &str,
        _details: Option<serde_json::Value>,
    ) -> Result<HeartbeatResponse, StoreError> {
        // Renew the claim and report whether the owning workflow was cancelled
        let result: Option<(Option<String>,)> = sqlx::query_as(
            r#"
            UPDATE durable_task_queue q SET heartbeat_at = NOW()
            WHERE q.id = $1 AND q.claimed_by = $2 AND q.status = 'claimed'
            RETURNING (SELECT w.status FROM durable_workflow_instances w WHERE w.id = q.workflow_id)
            "#,
        )
        .bind(task_id)
        .bind(worker_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to heartbeat task");
            StoreError::Database(e.to_string())
        })?;

        #[cfg(feature = "failpoints")]
        fail_point!("postgres_heartbeat_update", |_| {
            Err(StoreError::Database("injected: heartbeat update".into()))
        });

        match result {
            // Still ours, but the workflow was cancelled: stop the work (EVE-1134).
            Some((workflow_status,)) => Ok(HeartbeatResponse {
                accepted: true,
                should_cancel: workflow_status.as_deref() == Some("cancelled"),
            }),
            None => {
                // Task no longer claimed by this worker (maybe reclaimed or completed)
                Ok(HeartbeatResponse {
                    accepted: false,
                    should_cancel: true,
                })
            }
        }
    }

    #[instrument(skip(self, _result))]
    async fn complete_task(
        &self,
        task_id: Uuid,
        worker_id: &str,
        _result: serde_json::Value,
    ) -> Result<(), StoreError> {
        // Only complete if:
        // 1. Task is still claimed by this worker, OR
        // 2. Task is still claimed (status = 'claimed') by this worker
        // This prevents duplicate scheduling when a task is reclaimed due to heartbeat timeout
        let result = sqlx::query(
            r#"
            UPDATE durable_task_queue
            SET status = 'completed'
            WHERE id = $1 AND claimed_by = $2 AND status = 'claimed'
            RETURNING id
            "#,
        )
        .bind(task_id)
        .bind(worker_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to complete task");
            StoreError::Database(e.to_string())
        })?;

        #[cfg(feature = "failpoints")]
        fail_point!("postgres_complete_task_after_update", |_| {
            Err(StoreError::Database("injected: complete task".into()))
        });

        match result {
            Some(_) => {
                debug!(%task_id, %worker_id, "completed task");
                Ok(())
            }
            None => {
                // Task was reclaimed by another worker or already completed
                debug!(%task_id, %worker_id, "task not owned - was reclaimed or already completed");
                Err(StoreError::TaskNotOwned(task_id))
            }
        }
    }

    #[instrument(skip(self))]
    async fn fail_task_with_retry(
        &self,
        task_id: Uuid,
        error: &str,
        retryable: bool,
    ) -> Result<TaskFailureOutcome, StoreError> {
        // EVE-639: SELECT FOR UPDATE and the follow-up UPDATE must share ONE
        // transaction so the row lock is held across the whole read-modify-write.
        // Previously both ran on `self.pool`, so the FOR UPDATE lock was released
        // as soon as the SELECT statement returned and a concurrent fail_task /
        // reclaim_stale_tasks could interleave and double-increment `attempt` or
        // double-route the task to the DLQ. We also gate on status = 'claimed':
        // if a reclaimer has already requeued or sealed this task, there is
        // nothing for this caller to fail and we return TaskNotOwned rather than
        // clobbering the reclaimer's decision.
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

        let row = sqlx::query(
            r#"
            SELECT attempt, max_attempts, options, status
            FROM durable_task_queue
            WHERE id = $1
            FOR UPDATE
            "#,
        )
        .bind(task_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| StoreError::Database(e.to_string()))?
        .ok_or(StoreError::TaskNotFound(task_id))?;

        let status: String = row.get("status");
        if status != "claimed" {
            // Task was reclaimed, completed, or already dead by a concurrent
            // actor while we waited on the lock. Do not act on it.
            debug!(
                %task_id,
                status,
                "fail_task: task no longer claimed, skipping"
            );
            return Err(StoreError::TaskNotOwned(task_id));
        }

        let attempt: i32 = row.get("attempt");
        let max_attempts: i32 = row.get("max_attempts");
        let options_json: serde_json::Value = row.get("options");
        let options: ActivityOptions = serde_json::from_value(options_json)
            .map_err(|e| StoreError::Serialization(e.to_string()))?;

        let outcome = if retryable && attempt < max_attempts {
            // Calculate retry delay
            let delay = options.retry_policy.delay_for_attempt((attempt + 1) as u32);
            let visible_at = Utc::now() + chrono::Duration::from_std(delay).unwrap_or_default();

            // Requeue for retry
            sqlx::query(
                r#"
                UPDATE durable_task_queue
                SET status = 'pending',
                    claimed_by = NULL,
                    claimed_at = NULL,
                    heartbeat_at = NULL,
                    last_error = $2,
                    visible_at = $3
                WHERE id = $1
                "#,
            )
            .bind(task_id)
            .bind(error)
            .bind(visible_at)
            .execute(&mut *tx)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            debug!(%task_id, next_attempt = attempt + 1, "task will retry");
            TaskFailureOutcome::WillRetry {
                next_attempt: (attempt + 1) as u32,
                delay,
            }
        } else {
            // Move to DLQ
            sqlx::query(
                r#"
                UPDATE durable_task_queue
                SET status = 'dead',
                    last_error = $2
                WHERE id = $1
                "#,
            )
            .bind(task_id)
            .bind(error)
            .execute(&mut *tx)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            debug!(%task_id, "task moved to DLQ");
            TaskFailureOutcome::MovedToDlq
        };

        tx.commit()
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

        Ok(outcome)
    }

    #[instrument(skip(self))]
    async fn cancel_pending_tasks_for_workflow(
        &self,
        workflow_id: Uuid,
    ) -> Result<u64, StoreError> {
        let result = sqlx::query(
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
            error!(error = %e, "Failed to cancel pending tasks for workflow");
            StoreError::Database(e.to_string())
        })?;

        let count = result.rows_affected();
        if count > 0 {
            debug!(%workflow_id, count, "cancelled pending tasks for workflow");
        }
        Ok(count)
    }

    #[instrument(skip(self))]
    async fn get_task(&self, task_id: Uuid) -> Result<TaskInfo, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT id, workflow_id, activity_id, activity_type, status,
                   priority, attempt, max_attempts, claimed_by, last_error,
                   created_at, claimed_at
            FROM durable_task_queue
            WHERE id = $1
            "#,
        )
        .bind(task_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to get task");
            StoreError::Database(e.to_string())
        })?
        .ok_or(StoreError::TaskNotFound(task_id))?;

        let status_str: String = row.get("status");
        let status = match status_str.as_str() {
            "pending" => TaskStatus::Pending,
            "claimed" => TaskStatus::Claimed,
            "completed" => TaskStatus::Completed,
            "failed" => TaskStatus::Failed,
            "dead" => TaskStatus::Dead,
            "cancelled" => TaskStatus::Cancelled,
            _ => TaskStatus::Pending,
        };

        Ok(TaskInfo {
            id: row.get("id"),
            workflow_id: row.get::<Option<Uuid>, _>("workflow_id"),
            activity_id: row.get("activity_id"),
            activity_type: row.get("activity_type"),
            status,
            priority: row.get("priority"),
            attempt: row.get::<i32, _>("attempt") as u32,
            max_attempts: row.get::<i32, _>("max_attempts") as u32,
            claimed_by: row.get("claimed_by"),
            last_error: row.get("last_error"),
            created_at: row.get("created_at"),
            claimed_at: row.get("claimed_at"),
        })
    }

    #[instrument(skip(self))]
    async fn reclaim_stale_tasks(
        &self,
        stale_threshold: Duration,
    ) -> Result<ReclaimResult, StoreError> {
        let threshold =
            Utc::now() - chrono::Duration::from_std(stale_threshold).unwrap_or_default();

        // First, mark exhausted tasks as dead (they've used all attempts via stale reclaims)
        // This handles the case where workers panic without calling fail_task.
        let dead_rows = sqlx::query(
            r#"
            UPDATE durable_task_queue
            SET status = 'dead',
                last_error = COALESCE(last_error, 'Worker became unresponsive after exhausting all retry attempts')
            WHERE status = 'claimed'
              AND heartbeat_at < $1
              AND attempt >= max_attempts
            RETURNING id, workflow_id, activity_id, activity_type, input, last_error
            "#,
        )
        .bind(threshold)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to mark exhausted stale tasks as dead");
            StoreError::Database(e.to_string())
        })?;

        let dead_tasks: Vec<DeadTaskInfo> = dead_rows
            .iter()
            .map(|r| DeadTaskInfo {
                task_id: r.get("id"),
                workflow_id: r.get("workflow_id"),
                activity_id: r.get("activity_id"),
                activity_type: r.get("activity_type"),
                input: r.get("input"),
                last_error: r.get("last_error"),
            })
            .collect();

        if !dead_tasks.is_empty() {
            info!(
                count = dead_tasks.len(),
                "marked exhausted stale tasks as dead"
            );
        }

        // Forward-progress guard (EVE-534).
        //
        // For each stale task that still has retry attempts, derive its current
        // progress token from durably-recorded facts — the highest
        // durable_workflow_events.sequence_num for the task's workflow — and
        // compare it to the token observed at the previous reclaim:
        //
        //   - advanced (or first observation): record the new token, reset the
        //     no-progress counter, and requeue (status='pending').
        //   - not advanced: increment the no-progress counter. If it reaches the
        //     configured threshold, SEAL the task — mark the task 'dead' so it
        //     routes to the DLQ instead of looping forever — otherwise requeue.
        //
        // The token is monotonic per workflow, so a non-progressing retry (an
        // atom that crashes before recording any event) cannot game it. The
        // whole transition is a single UPDATE so concurrent reclaimers stay
        // consistent under the row lock.
        let threshold_i64 = no_progress_seal_threshold_from_env() as i64;
        let rows = sqlx::query(
            r#"
            WITH stale AS (
                SELECT q.id,
                       q.workflow_id,
                       q.progress_token AS prev_token,
                       q.no_progress_count AS prev_no_progress,
                       -- Highest durable event sequence for this workflow encoded
                       -- as a monotonic token (-1 => no progress events yet =>
                       -- token 0).
                       --
                       -- We EXCLUDE 'activity_started' events: claim_task writes
                       -- one on the FIRST attempt of a task (see the EVE-639
                       -- started_by_workflow handling; reclaims no longer emit
                       -- it). Counting that first-attempt marker as progress
                       -- would let a task that crashes immediately after dispatch
                       -- — recording nothing else — appear to advance, defeating
                       -- the seal guard (EVE-534). 'activity_started' is the only
                       -- dispatch/bookkeeping event written to
                       -- durable_workflow_events; every other event_type records
                       -- a real workflow fact (scheduled/completed/failed/timer/
                       -- signal/child), so a denylist of just this one type is
                       -- complete.
                       COALESCE(
                           (SELECT MAX(e.sequence_num)
                              FROM durable_workflow_events e
                             WHERE e.workflow_id = q.workflow_id
                               AND e.event_type <> 'activity_started'),
                           -1
                       )::BIGINT + 1 AS cur_token
                  FROM durable_task_queue q
                 WHERE q.status = 'claimed'
                   AND q.heartbeat_at < $1
                   AND q.attempt < q.max_attempts
                   FOR UPDATE OF q SKIP LOCKED
            ),
            decided AS (
                SELECT id,
                       workflow_id,
                       cur_token,
                       -- Standalone tasks (workflow_id IS NULL) have no workflow
                       -- event stream, so cur_token is always 0 and the seal
                       -- guard would DLQ them after N reclaims purely from
                       -- missing workflow context. Exempt them: always treat as
                       -- advanced and never increment no_progress_count, so they
                       -- only DLQ via the max-attempts path (EVE-534).
                       --
                       -- A missing prior token is treated as the 0 baseline (no
                       -- events recorded yet), so a task that records nothing on
                       -- its first attempt already counts as no-progress. The
                       -- token only "advances" when it strictly grows past the
                       -- baseline, which a non-progressing retry can never do.
                       (workflow_id IS NULL
                        OR cur_token > COALESCE(prev_token, 0)) AS advanced,
                       CASE
                           WHEN workflow_id IS NULL THEN 0
                           WHEN cur_token > COALESCE(prev_token, 0) THEN 0
                           ELSE prev_no_progress + 1
                       END AS new_no_progress
                  FROM stale
            )
            UPDATE durable_task_queue q
               SET status = CASE
                                WHEN d.new_no_progress >= $2 THEN 'dead'
                                ELSE 'pending'
                            END,
                   claimed_by = NULL,
                   claimed_at = NULL,
                   -- Leave standalone tasks' progress_token NULL; only
                   -- workflow-scoped tasks track a monotonic token.
                   progress_token = CASE
                                        WHEN d.workflow_id IS NULL THEN NULL
                                        ELSE d.cur_token
                                    END,
                   no_progress_count = d.new_no_progress,
                   last_error = CASE
                                    WHEN d.new_no_progress >= $2
                                    THEN 'Task sealed by the no-progress guard: no forward progress across '
                                         || d.new_no_progress::TEXT
                                         || ' consecutive recoveries (EVE-534)'
                                    ELSE q.last_error
                                END
              FROM decided d
             WHERE q.id = d.id
            RETURNING q.id,
                      q.workflow_id,
                      q.activity_id,
                      q.activity_type,
                      q.input,
                      q.status AS new_status,
                      q.no_progress_count
            "#,
        )
        .bind(threshold)
        .bind(threshold_i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to reclaim stale tasks");
            StoreError::Database(e.to_string())
        })?;

        let mut reclaimed_ids: Vec<Uuid> = Vec::new();
        let mut sealed_tasks: Vec<SealedTaskInfo> = Vec::new();
        for r in &rows {
            let new_status: String = r.get("new_status");
            if new_status == "dead" {
                sealed_tasks.push(SealedTaskInfo {
                    task_id: r.get("id"),
                    workflow_id: r.get("workflow_id"),
                    activity_id: r.get("activity_id"),
                    activity_type: r.get("activity_type"),
                    input: r.get("input"),
                    reason: "no_progress".to_string(),
                    no_progress_count: r.get::<i32, _>("no_progress_count") as u32,
                });
            } else {
                reclaimed_ids.push(r.get("id"));
            }
        }

        #[cfg(feature = "failpoints")]
        fail_point!("postgres_reclaim_stale_after_update", |_| {
            Err(StoreError::Database(
                "injected: after reclaim update".into(),
            ))
        });

        if !reclaimed_ids.is_empty() || !dead_tasks.is_empty() || !sealed_tasks.is_empty() {
            debug!(
                reclaimed = reclaimed_ids.len(),
                dead = dead_tasks.len(),
                sealed = sealed_tasks.len(),
                "processed stale tasks"
            );
        }
        if !sealed_tasks.is_empty() {
            info!(
                count = sealed_tasks.len(),
                "sealed non-progressing tasks during reclaim (EVE-534)"
            );
        }

        // Also mark workers with stale heartbeats as stopped.
        // This cleans up workers that crashed without calling deregister_worker.
        let worker_heartbeat_threshold =
            Utc::now() - chrono::Duration::seconds(WORKER_HEARTBEAT_TIMEOUT_SECS);
        let stale_workers = sqlx::query(
            r#"
            UPDATE durable_workers
            SET status = 'stopped'
            WHERE status = 'active'
              AND last_heartbeat_at < $1
            RETURNING id
            "#,
        )
        .bind(worker_heartbeat_threshold)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to mark stale workers as stopped");
            StoreError::Database(e.to_string())
        })?;

        if !stale_workers.is_empty() {
            let ids: Vec<String> = stale_workers.iter().map(|r| r.get("id")).collect();
            info!(count = stale_workers.len(), worker_ids = ?ids, "marked stale workers as stopped");
        }

        Ok(ReclaimResult {
            reclaimed_ids,
            dead_tasks,
            sealed_tasks,
        })
    }

    #[instrument(skip(self))]
    async fn list_tasks(
        &self,
        filter: TaskFilter,
        pagination: Pagination,
    ) -> Result<Vec<TaskInfo>, StoreError> {
        let status_str = filter.status.map(|s| match s {
            TaskStatus::Pending => "pending",
            TaskStatus::Claimed => "claimed",
            TaskStatus::Completed => "completed",
            TaskStatus::Failed => "failed",
            TaskStatus::Dead => "dead",
            TaskStatus::Cancelled => "cancelled",
        });

        let rows = sqlx::query(
            r#"
            SELECT id, workflow_id, activity_id, activity_type, status,
                   priority, attempt, max_attempts, claimed_by, last_error,
                   created_at, claimed_at
            FROM durable_task_queue
            WHERE ($1::text IS NULL OR status = $1)
              AND ($2::text IS NULL OR activity_type = $2)
              AND ($3::uuid IS NULL OR workflow_id = $3)
              AND (NOT $6 OR workflow_id IS NULL)
            ORDER BY created_at ASC
            OFFSET $4
            LIMIT $5
            "#,
        )
        .bind(status_str)
        .bind(&filter.activity_type)
        .bind(filter.workflow_id)
        .bind(pagination.offset as i64)
        .bind(pagination.limit as i64)
        .bind(filter.standalone_only)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to list tasks");
            StoreError::Database(e.to_string())
        })?;

        let tasks = rows
            .into_iter()
            .map(|row| {
                let status_str: String = row.get("status");
                let status = match status_str.as_str() {
                    "pending" => TaskStatus::Pending,
                    "claimed" => TaskStatus::Claimed,
                    "completed" => TaskStatus::Completed,
                    "failed" => TaskStatus::Failed,
                    "dead" => TaskStatus::Dead,
                    "cancelled" => TaskStatus::Cancelled,
                    _ => TaskStatus::Pending,
                };

                TaskInfo {
                    id: row.get("id"),
                    workflow_id: row.get::<Option<Uuid>, _>("workflow_id"),
                    activity_id: row.get("activity_id"),
                    activity_type: row.get("activity_type"),
                    status,
                    priority: row.get("priority"),
                    attempt: row.get::<i32, _>("attempt") as u32,
                    max_attempts: row.get::<i32, _>("max_attempts") as u32,
                    claimed_by: row.get("claimed_by"),
                    last_error: row.get("last_error"),
                    created_at: row.get("created_at"),
                    claimed_at: row.get("claimed_at"),
                }
            })
            .collect();

        Ok(tasks)
    }
}
