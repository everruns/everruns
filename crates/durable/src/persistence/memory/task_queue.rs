//! Implementation of [`crate::TaskQueue`].

use super::*;

#[async_trait]
impl TaskQueue for InMemoryWorkflowEventStore {
    async fn enqueue_task(&self, task: TaskDefinition) -> Result<Uuid, StoreError> {
        let mut tasks = self.tasks.write();
        if task.options.dedupe_by_activity_id
            && let Some(workflow_id) = task.workflow_id
            && let Some(task_id) = tasks.ids_for_workflow(workflow_id).into_iter().find(|id| {
                tasks
                    .get(id)
                    .is_some_and(|t| t.definition.activity_id == task.activity_id)
            })
        {
            return Ok(task_id);
        }
        // Check pending task limits
        let pending_count = tasks.pending_count(task.workflow_id);
        if let Some(wf_id) = task.workflow_id {
            let limit = self.max_pending_tasks_per_workflow;
            if pending_count >= limit {
                return Err(StoreError::TaskQueueLimitExceeded {
                    workflow_id: wf_id,
                    current: pending_count,
                    limit,
                });
            }
        } else if pending_count >= DEFAULT_MAX_PENDING_STANDALONE_TASKS {
            return Err(StoreError::StandaloneTaskQueueLimitExceeded {
                current: pending_count,
                limit: DEFAULT_MAX_PENDING_STANDALONE_TASKS,
            });
        }
        let task_id = Uuid::now_v7();
        tasks.insert(task_id, TaskState::scheduled(task));
        Ok(task_id)
    }

    async fn enqueue_claimed_task(
        &self,
        task: TaskDefinition,
        worker_id: &str,
    ) -> Result<Enqueued, StoreError> {
        let may_claim = self
            .workers
            .read()
            .get(worker_id)
            .is_some_and(|w| w.status != "draining");
        if !may_claim || task.options.start_delay.is_some() || task.options.dedupe_by_activity_id {
            return self.enqueue_task(task).await.map(Enqueued::Queued);
        }

        let now = Utc::now();
        let task_id = Uuid::now_v7();
        let mut claimed = ClaimedTask {
            id: task_id,
            workflow_id: task.workflow_id,
            activity_id: task.activity_id.clone(),
            activity_type: task.activity_type.clone(),
            input: task.input.clone(),
            options: task.options.clone(),
            attempt: 1,
            max_attempts: task.options.retry_policy.max_attempts,
            workflow_status: None,
        };
        let mut state = TaskState::pending(task);
        state.status = TaskStatus::Claimed;
        state.claimed_by = Some(worker_id.to_string());
        state.claimed_at = Some(now);
        state.heartbeat_at = Some(now);
        state.attempt = 1;
        self.tasks.write().insert(task_id, state);

        // As a claim does: the workflow status rides along, and the first
        // attempt records ActivityStarted.
        if let Some(wf) = claimed.workflow_id.and_then(|id| {
            self.workflows.write().get_mut(&id).map(|wf| {
                wf.events.push(WorkflowEvent::ActivityStarted {
                    activity_id: claimed.activity_id.clone(),
                    attempt: 1,
                    worker_id: worker_id.to_string(),
                });
                wf.status
            })
        }) {
            claimed.workflow_status = Some(wf);
        }
        Ok(Enqueued::Claimed(Box::new(claimed)))
    }

    async fn claim_queue_tasks(
        &self,
        worker_id: &str,
        queue: Option<&str>,
        activity_types: &[String],
        max_tasks: usize,
    ) -> Result<Vec<ClaimedTask>, StoreError> {
        // Like PostgreSQL, only a registered worker that is not draining claims.
        let may_claim = self
            .workers
            .read()
            .get(worker_id)
            .is_some_and(|w| w.status != "draining");
        if !may_claim {
            return Ok(vec![]);
        }

        let now = Utc::now();
        let mut tasks = self.tasks.write();
        let mut claimed = vec![];
        for task_id in tasks.claimable(queue, activity_types, now, max_tasks) {
            let task = tasks
                .update(task_id, |task| {
                    task.status = TaskStatus::Claimed;
                    task.claimed_by = Some(worker_id.to_string());
                    task.claimed_at = Some(now);
                    task.heartbeat_at = Some(now);
                    task.attempt += 1;
                    ClaimedTask {
                        id: task_id,
                        workflow_id: task.definition.workflow_id,
                        activity_id: task.definition.activity_id.clone(),
                        activity_type: task.definition.activity_type.clone(),
                        input: task.definition.input.clone(),
                        options: task.definition.options.clone(),
                        attempt: task.attempt,
                        max_attempts: task.definition.options.retry_policy.max_attempts,
                        workflow_status: None,
                    }
                })
                .expect("claimable ids come from the table");
            claimed.push(task);
        }
        drop(tasks);

        // PostgreSQL records ActivityStarted on the first attempt only (EVE-639).
        // The workflow status rides along with the claim, as in PostgreSQL.
        let mut workflows = self.workflows.write();
        for task in claimed.iter_mut() {
            let Some(wf) = task.workflow_id.and_then(|id| workflows.get_mut(&id)) else {
                continue;
            };
            task.workflow_status = Some(wf.status);
            if task.attempt == 1 {
                wf.events.push(WorkflowEvent::ActivityStarted {
                    activity_id: task.activity_id.clone(),
                    attempt: task.attempt,
                    worker_id: worker_id.to_string(),
                });
            }
        }

        Ok(claimed)
    }

    async fn heartbeat_task(
        &self,
        task_id: Uuid,
        worker_id: &str,
        _details: Option<serde_json::Value>,
    ) -> Result<HeartbeatResponse, StoreError> {
        let mut tasks = self.tasks.write();
        let owned = tasks.get(&task_id).is_some_and(|t| {
            t.status == TaskStatus::Claimed && t.claimed_by.as_deref() == Some(worker_id)
        });
        if !owned {
            // Reclaimed, finished or unknown: tell the worker to stop.
            return Ok(HeartbeatResponse {
                accepted: false,
                should_cancel: true,
            });
        }
        let workflow_id = tasks
            .update(task_id, |t| {
                t.heartbeat_at = Some(Utc::now());
                t.definition.workflow_id
            })
            .flatten();
        drop(tasks); // one lock at a time
        let status = workflow_id.and_then(|id| self.workflows.read().get(&id).map(|w| w.status));
        Ok(HeartbeatResponse {
            accepted: true,
            should_cancel: status == Some(WorkflowStatus::Cancelled),
        })
    }

    async fn complete_task(
        &self,
        task_id: Uuid,
        worker_id: &str,
        _result: serde_json::Value,
    ) -> Result<(), StoreError> {
        let mut tasks = self.tasks.write();
        let task = tasks
            .get(&task_id)
            .ok_or(StoreError::TaskNotFound(task_id))?;

        // Verify the task is still claimed by this worker
        if task.status != TaskStatus::Claimed || task.claimed_by.as_deref() != Some(worker_id) {
            return Err(StoreError::TaskNotOwned(task_id));
        }

        tasks.update(task_id, |t| t.status = TaskStatus::Completed);
        Ok(())
    }

    async fn fail_task_with_retry(
        &self,
        task_id: Uuid,
        error: &str,
        retryable: bool,
    ) -> Result<TaskFailureOutcome, StoreError> {
        let mut tasks = self.tasks.write();
        let task = tasks
            .get(&task_id)
            .ok_or(StoreError::TaskNotFound(task_id))?;
        // A reclaimer already requeued or sealed it: leave its decision alone.
        if task.status != TaskStatus::Claimed {
            return Err(StoreError::TaskNotOwned(task_id));
        }

        let outcome = tasks.update(task_id, |task| {
            task.error_history.push(error.to_string());
            task.last_error = Some(error.to_string());

            let max_attempts = task.definition.options.retry_policy.max_attempts;
            if retryable && task.attempt < max_attempts {
                let delay = task
                    .definition
                    .options
                    .retry_policy
                    .delay_for_attempt(task.attempt + 1);
                task.release(Utc::now() + chrono::Duration::from_std(delay).unwrap_or_default());
                TaskFailureOutcome::WillRetry {
                    next_attempt: task.attempt + 1,
                    delay,
                }
            } else {
                task.status = TaskStatus::Dead;
                TaskFailureOutcome::MovedToDlq
            }
        });
        Ok(outcome.expect("task exists"))
    }

    async fn cancel_pending_tasks_for_workflow(
        &self,
        workflow_id: Uuid,
    ) -> Result<u64, StoreError> {
        let mut tasks = self.tasks.write();
        let mut count = 0u64;
        for id in tasks.ids_for_workflow(workflow_id) {
            tasks.update(id, |task| {
                if task.status == TaskStatus::Pending {
                    task.status = TaskStatus::Cancelled;
                    count += 1;
                }
            });
        }
        Ok(count)
    }

    async fn get_task(&self, task_id: Uuid) -> Result<TaskInfo, StoreError> {
        let tasks = self.tasks.read();
        let task = tasks
            .get(&task_id)
            .ok_or(StoreError::TaskNotFound(task_id))?;

        Ok(TaskInfo {
            id: task_id,
            workflow_id: task.definition.workflow_id,
            activity_id: task.definition.activity_id.clone(),
            activity_type: task.definition.activity_type.clone(),
            status: task.status,
            priority: task.definition.options.priority,
            attempt: task.attempt,
            max_attempts: task.definition.options.retry_policy.max_attempts,
            claimed_by: task.claimed_by.clone(),
            last_error: task.last_error.clone(),
            created_at: task.created_at,
            claimed_at: task.claimed_at,
            queue: task.definition.options.queue.clone(),
        })
    }

    async fn reclaim_stale_tasks(
        &self,
        stale_threshold: Duration,
    ) -> Result<ReclaimResult, StoreError> {
        // A claimed task whose last heartbeat is older than the threshold is
        // reclaimable, as in PostgreSQL. This also
        // mirrors the Postgres forward-progress guard (EVE-534) so unit tests
        // can exercise the seal decision without a database: derive each task's
        // progress token from the highest recorded event sequence for its
        // workflow, compare to the token observed at the previous reclaim, and
        // seal (mark dead -> DLQ) after N consecutive no-progress recoveries.
        let threshold = no_progress_seal_threshold_from_env();

        // Snapshot the highest *progress* event sequence per workflow first to
        // avoid holding both locks at once.
        //
        // We must use the SAME progress-signal rule as the Postgres reclaim CTE
        // (EVE-534): 'activity_started' events are excluded because claim_task
        // writes one on every (re)claim, so counting them would make the token
        // advance every cycle and defeat the seal guard. The token is the
        // highest sequence position of a NON-'activity_started' event (-1 => no
        // progress events yet => token 0). Both stores write 'activity_started'
        // on a first claim.
        let highest_seq: HashMap<Uuid, i64> = {
            let workflows = self.workflows.read();
            workflows
                .iter()
                .map(|(id, wf)| {
                    let seq = wf
                        .events
                        .iter()
                        .enumerate()
                        .filter(|(_, e)| event_type_name(e) != "activity_started")
                        .map(|(i, _)| i as i64)
                        .next_back()
                        .unwrap_or(-1);
                    (*id, seq)
                })
                .collect()
        };

        let mut reclaimed_ids = Vec::new();
        let mut dead_tasks = Vec::new();
        let mut sealed_tasks = Vec::new();

        let cutoff = Utc::now() - chrono::Duration::from_std(stale_threshold).unwrap_or_default();
        let mut tasks = self.tasks.write();
        let stale: Vec<Uuid> = tasks
            .claimed_ids()
            .into_iter()
            .filter(|id| {
                tasks
                    .get(id)
                    .is_some_and(|t| t.heartbeat_at.is_none_or(|at| at < cutoff))
            })
            .collect();
        for task_id in stale {
            tasks.update(task_id, |task| {
                let task_id = &task_id;
                let max_attempts = task.definition.options.retry_policy.max_attempts;
                if task.attempt >= max_attempts {
                    task.status = TaskStatus::Dead;
                    let error = task.last_error.clone().unwrap_or_else(|| {
                        "Worker became unresponsive after exhausting all retry attempts".to_string()
                    });
                    task.last_error = Some(error.clone());
                    dead_tasks.push(DeadTaskInfo {
                        task_id: *task_id,
                        workflow_id: task.definition.workflow_id,
                        activity_id: task.definition.activity_id.clone(),
                        activity_type: task.definition.activity_type.clone(),
                        input: task.definition.input.clone(),
                        last_error: Some(error),
                    });
                    return;
                }

                let wf_id = task.definition.workflow_id;

                // Standalone tasks (workflow_id IS NULL) have no workflow event
                // stream, so the derived progress token is always 0 and the seal
                // guard would DLQ them after N reclaims purely from missing workflow
                // context. Exempt them: always treat as advanced (leave
                // progress_token NULL, never increment no_progress_count) and
                // re-queue to pending so they only DLQ via the max-attempts path
                // above (EVE-534).
                if wf_id.is_none() {
                    task.no_progress_count = 0;
                    task.release(task.visible_at);
                    reclaimed_ids.push(*task_id);
                    return;
                }

                let cur_seq = wf_id
                    .and_then(|id| highest_seq.get(&id).copied())
                    .unwrap_or(-1);
                let cur_token = cur_seq + 1; // -1 (no events) => token 0

                // A missing prior token is treated as the 0 baseline, so a task that
                // records nothing on its first attempt already counts as no-progress
                // (mirrors the Postgres COALESCE(prev_token, 0) rule).
                let prev = task.progress_token.unwrap_or(0);
                let advanced = cur_token > prev;

                task.progress_token = Some(cur_token);
                if advanced {
                    task.no_progress_count = 0;
                } else {
                    task.no_progress_count += 1;
                }

                if task.no_progress_count >= threshold {
                    task.status = TaskStatus::Dead;
                    task.last_error = Some(format!(
                        "Task sealed by the no-progress guard: no forward progress across {} consecutive recoveries (EVE-534)",
                        task.no_progress_count
                    ));
                    task.claimed_by = None;
                    task.claimed_at = None;
                    sealed_tasks.push(SealedTaskInfo {
                        task_id: *task_id,
                        workflow_id: wf_id,
                        activity_id: task.definition.activity_id.clone(),
                        activity_type: task.definition.activity_type.clone(),
                        input: task.definition.input.clone(),
                        reason: "no_progress".to_string(),
                        no_progress_count: task.no_progress_count,
                    });
                } else {
                    task.release(task.visible_at);
                    reclaimed_ids.push(*task_id);
                }
            });
        }

        Ok(ReclaimResult {
            reclaimed_ids,
            dead_tasks,
            sealed_tasks,
        })
    }

    async fn list_tasks(
        &self,
        filter: TaskFilter,
        pagination: Pagination,
    ) -> Result<Vec<TaskInfo>, StoreError> {
        let tasks = self.tasks.read();
        let mut result: Vec<_> = tasks
            .iter()
            .filter(|(_, t)| {
                // Apply status filter
                if let Some(ref status) = filter.status
                    && &t.status != status
                {
                    return false;
                }
                // Apply activity_type filter
                if let Some(ref activity_type) = filter.activity_type
                    && &t.definition.activity_type != activity_type
                {
                    return false;
                }
                // Apply workflow_id filter
                if let Some(ref wf_id) = filter.workflow_id
                    && t.definition.workflow_id.as_ref() != Some(wf_id)
                {
                    return false;
                }
                // Apply standalone_only filter
                if filter.standalone_only && t.definition.workflow_id.is_some() {
                    return false;
                }
                true
            })
            .map(|(id, t)| TaskInfo {
                id: *id,
                workflow_id: t.definition.workflow_id,
                activity_id: t.definition.activity_id.clone(),
                activity_type: t.definition.activity_type.clone(),
                status: t.status,
                priority: t.definition.options.priority,
                attempt: t.attempt,
                max_attempts: t.definition.options.retry_policy.max_attempts,
                claimed_by: t.claimed_by.clone(),
                last_error: t.last_error.clone(),
                created_at: t.created_at,
                claimed_at: t.claimed_at,
                queue: t.definition.options.queue.clone(),
            })
            .collect();
        // Sort by created_at ascending (oldest first) to show execution order
        result.sort_by_key(|task| task.created_at);

        let start = pagination.offset as usize;
        let end = (pagination.offset + pagination.limit) as usize;
        Ok(result.into_iter().skip(start).take(end - start).collect())
    }
}
