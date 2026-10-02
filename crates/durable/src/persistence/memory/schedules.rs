//! Schedules implementation (see `store.rs` for the trait contract).

use super::*;

#[async_trait]
impl Schedules for InMemoryWorkflowEventStore {
    async fn create_schedule(&self, schedule: CreateScheduleRow) -> Result<Uuid, StoreError> {
        let id = Uuid::now_v7();
        let now = Utc::now();

        let row = ScheduleRow {
            id,
            name: schedule.name,
            description: schedule.description,
            cron_expression: schedule.cron_expression,
            timezone: schedule.timezone,
            target_type: schedule.target_type,
            target_name: schedule.target_name,
            target_input: schedule.target_input,
            enabled: schedule.enabled,
            max_concurrent: schedule.max_concurrent,
            catch_up_missed: schedule.catch_up_missed,
            max_catch_up: schedule.max_catch_up,
            retry_policy: schedule.retry_policy,
            last_triggered_at: None,
            next_trigger_at: schedule.next_trigger_at,
            claimed_by: None,
            claimed_at: None,
            created_at: now,
            updated_at: now,
        };

        let mut schedules = self.schedules.write();
        schedules.insert(id, ScheduleMemState { row });
        Ok(id)
    }

    async fn get_schedule(&self, id: Uuid) -> Result<ScheduleRow, StoreError> {
        let schedules = self.schedules.read();
        schedules
            .get(&id)
            .map(|s| s.row.clone())
            .ok_or(StoreError::ScheduleNotFound(id))
    }

    async fn list_schedules(
        &self,
        filter: ScheduleFilter,
        pagination: Pagination,
    ) -> Result<Vec<ScheduleRow>, StoreError> {
        let schedules = self.schedules.read();
        let mut result: Vec<_> = schedules
            .values()
            .filter(|s| {
                if filter.enabled.is_some_and(|e| s.row.enabled != e) {
                    return false;
                }
                if filter.target_type.is_some_and(|t| s.row.target_type != t) {
                    return false;
                }
                true
            })
            .map(|s| s.row.clone())
            .collect();
        result.sort_by_key(|schedule| std::cmp::Reverse(schedule.created_at));

        let start = pagination.offset as usize;
        let end = (pagination.offset + pagination.limit) as usize;
        Ok(result.into_iter().skip(start).take(end - start).collect())
    }

    async fn count_schedules(&self, filter: ScheduleFilter) -> Result<u64, StoreError> {
        let schedules = self.schedules.read();
        let count = schedules
            .values()
            .filter(|s| {
                if filter.enabled.is_some_and(|e| s.row.enabled != e) {
                    return false;
                }
                if filter.target_type.is_some_and(|t| s.row.target_type != t) {
                    return false;
                }
                true
            })
            .count();
        Ok(count as u64)
    }

    async fn update_schedule(&self, id: Uuid, update: UpdateSchedule) -> Result<(), StoreError> {
        let mut schedules = self.schedules.write();
        let state = schedules
            .get_mut(&id)
            .ok_or(StoreError::ScheduleNotFound(id))?;

        if let Some(name) = update.name {
            state.row.name = name;
        }
        update.description.apply(&mut state.row.description);
        if let Some(cron_expression) = update.cron_expression {
            state.row.cron_expression = cron_expression;
        }
        if let Some(timezone) = update.timezone {
            state.row.timezone = timezone;
        }
        if let Some(target_type) = update.target_type {
            state.row.target_type = target_type;
        }
        if let Some(target_name) = update.target_name {
            state.row.target_name = target_name;
        }
        if let Some(target_input) = update.target_input {
            state.row.target_input = target_input;
        }
        if let Some(enabled) = update.enabled {
            state.row.enabled = enabled;
        }
        update.max_concurrent.apply(&mut state.row.max_concurrent);
        if let Some(catch_up_missed) = update.catch_up_missed {
            state.row.catch_up_missed = catch_up_missed;
        }
        update.max_catch_up.apply(&mut state.row.max_catch_up);
        update.retry_policy.apply(&mut state.row.retry_policy);
        update.next_trigger_at.apply(&mut state.row.next_trigger_at);
        state.row.updated_at = Utc::now();
        Ok(())
    }

    async fn delete_schedule(&self, id: Uuid) -> Result<(), StoreError> {
        let mut schedules = self.schedules.write();
        schedules
            .remove(&id)
            .ok_or(StoreError::ScheduleNotFound(id))?;

        // Also remove executions for this schedule
        let mut executions = self.schedule_executions.write();
        executions.retain(|_, e| e.row.schedule_id != id);
        Ok(())
    }

    async fn claim_due_schedules(
        &self,
        scheduler_id: &str,
        limit: u32,
    ) -> Result<Vec<ScheduleRow>, StoreError> {
        let now = Utc::now();
        let mut schedules = self.schedules.write();

        // Find due schedules sorted by next_trigger_at
        let mut candidates: Vec<_> = schedules
            .iter_mut()
            .filter(|(_, s)| {
                s.row.enabled
                    && s.row.next_trigger_at.is_some_and(|t| t <= now)
                    && s.row.claimed_by.is_none()
            })
            .collect();
        candidates.sort_by_key(|entry| entry.1.row.next_trigger_at);

        let mut claimed = Vec::new();
        for (_, state) in candidates {
            if claimed.len() >= limit as usize {
                break;
            }
            state.row.claimed_by = Some(scheduler_id.to_string());
            state.row.claimed_at = Some(now);
            claimed.push(state.row.clone());
        }

        Ok(claimed)
    }

    async fn update_next_trigger(
        &self,
        id: Uuid,
        next: chrono::DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let mut schedules = self.schedules.write();
        let state = schedules
            .get_mut(&id)
            .ok_or(StoreError::ScheduleNotFound(id))?;
        state.row.next_trigger_at = Some(next);
        state.row.last_triggered_at = Some(Utc::now());
        state.row.claimed_by = None;
        state.row.claimed_at = None;
        state.row.updated_at = Utc::now();
        Ok(())
    }

    async fn skip_schedule_trigger(&self, id: Uuid) -> Result<(), StoreError> {
        // Just release the claim, keep next_trigger_at the same
        let mut schedules = self.schedules.write();
        let state = schedules
            .get_mut(&id)
            .ok_or(StoreError::ScheduleNotFound(id))?;
        state.row.claimed_by = None;
        state.row.claimed_at = None;
        Ok(())
    }

    async fn release_schedule(&self, id: Uuid) -> Result<(), StoreError> {
        self.skip_schedule_trigger(id).await
    }

    async fn create_schedule_execution(
        &self,
        schedule_id: Uuid,
        scheduled_at: chrono::DateTime<Utc>,
    ) -> Result<Uuid, StoreError> {
        let id = Uuid::now_v7();
        let now = Utc::now();

        let row = ScheduleExecutionRow {
            id,
            schedule_id,
            scheduled_at,
            started_at: now,
            completed_at: None,
            status: ScheduleExecutionStatus::Running,
            workflow_id: None,
            task_id: None,
            error: None,
            duration_ms: None,
            created_at: now,
        };

        let mut executions = self.schedule_executions.write();
        executions.insert(id, ScheduleExecutionMemState { row });
        Ok(id)
    }

    async fn get_schedule_execution(&self, id: Uuid) -> Result<ScheduleExecutionRow, StoreError> {
        let executions = self.schedule_executions.read();
        executions
            .get(&id)
            .map(|e| e.row.clone())
            .ok_or(StoreError::ScheduleExecutionNotFound(id))
    }

    async fn complete_schedule_execution(
        &self,
        execution_id: Uuid,
        target_id: Uuid,
        is_workflow: bool,
    ) -> Result<(), StoreError> {
        let mut executions = self.schedule_executions.write();
        let state = executions
            .get_mut(&execution_id)
            .ok_or(StoreError::ScheduleExecutionNotFound(execution_id))?;

        let now = Utc::now();
        state.row.status = ScheduleExecutionStatus::Completed;
        state.row.completed_at = Some(now);
        state.row.duration_ms = Some((now - state.row.started_at).num_milliseconds() as i32);
        if is_workflow {
            state.row.workflow_id = Some(target_id);
        } else {
            state.row.task_id = Some(target_id);
        }
        Ok(())
    }

    async fn fail_schedule_execution(
        &self,
        execution_id: Uuid,
        error: &str,
    ) -> Result<(), StoreError> {
        let mut executions = self.schedule_executions.write();
        let state = executions
            .get_mut(&execution_id)
            .ok_or(StoreError::ScheduleExecutionNotFound(execution_id))?;

        let now = Utc::now();
        state.row.status = ScheduleExecutionStatus::Failed;
        state.row.completed_at = Some(now);
        state.row.duration_ms = Some((now - state.row.started_at).num_milliseconds() as i32);
        state.row.error = Some(error.to_string());
        Ok(())
    }

    async fn skip_schedule_execution(
        &self,
        execution_id: Uuid,
        reason: &str,
    ) -> Result<(), StoreError> {
        let mut executions = self.schedule_executions.write();
        let state = executions
            .get_mut(&execution_id)
            .ok_or(StoreError::ScheduleExecutionNotFound(execution_id))?;

        let now = Utc::now();
        state.row.status = ScheduleExecutionStatus::Skipped;
        state.row.completed_at = Some(now);
        state.row.duration_ms = Some((now - state.row.started_at).num_milliseconds() as i32);
        state.row.error = Some(reason.to_string());
        Ok(())
    }

    async fn list_schedule_executions(
        &self,
        filter: ScheduleExecutionFilter,
        pagination: Pagination,
    ) -> Result<Vec<ScheduleExecutionRow>, StoreError> {
        let executions = self.schedule_executions.read();
        let mut result: Vec<_> = executions
            .values()
            .filter(|e| {
                if filter.schedule_id.is_some_and(|id| e.row.schedule_id != id) {
                    return false;
                }
                if filter.status.is_some_and(|s| e.row.status != s) {
                    return false;
                }
                true
            })
            .map(|e| e.row.clone())
            .collect();
        result.sort_by_key(|execution| std::cmp::Reverse(execution.created_at));

        let start = pagination.offset as usize;
        let end = (pagination.offset + pagination.limit) as usize;
        Ok(result.into_iter().skip(start).take(end - start).collect())
    }

    async fn count_running_executions(&self, schedule_id: Uuid) -> Result<u32, StoreError> {
        let executions = self.schedule_executions.read();
        let count = executions
            .values()
            .filter(|e| {
                e.row.schedule_id == schedule_id && e.row.status == ScheduleExecutionStatus::Running
            })
            .count();
        Ok(count as u32)
    }

    async fn get_schedule_stats(&self, schedule_id: Uuid) -> Result<ScheduleStats, StoreError> {
        let executions = self.schedule_executions.read();
        let schedule_execs: Vec<_> = executions
            .values()
            .filter(|e| e.row.schedule_id == schedule_id)
            .collect();

        let total_executions = schedule_execs.len() as u64;
        let successful_executions = schedule_execs
            .iter()
            .filter(|e| e.row.status == ScheduleExecutionStatus::Completed)
            .count() as u64;
        let failed_executions = schedule_execs
            .iter()
            .filter(|e| e.row.status == ScheduleExecutionStatus::Failed)
            .count() as u64;
        let skipped_executions = schedule_execs
            .iter()
            .filter(|e| e.row.status == ScheduleExecutionStatus::Skipped)
            .count() as u64;

        let durations: Vec<_> = schedule_execs
            .iter()
            .filter_map(|e| e.row.duration_ms)
            .collect();
        let avg_duration_ms = if durations.is_empty() {
            None
        } else {
            Some(durations.iter().map(|d| *d as u64).sum::<u64>() / durations.len() as u64)
        };

        let last_execution_status = schedule_execs
            .iter()
            .max_by_key(|e| e.row.created_at)
            .map(|e| e.row.status);

        Ok(ScheduleStats {
            total_executions,
            successful_executions,
            failed_executions,
            skipped_executions,
            avg_duration_ms,
            last_execution_status,
        })
    }

    async fn register_scheduler_instance(
        &self,
        instance: SchedulerInstanceInfo,
    ) -> Result<(), StoreError> {
        let mut instances = self.scheduler_instances.write();
        instances.insert(instance.instance_id.clone(), instance);
        Ok(())
    }

    async fn heartbeat_scheduler_instance(
        &self,
        instance_id: &str,
        schedules_processed: u64,
    ) -> Result<(), StoreError> {
        let mut instances = self.scheduler_instances.write();
        if let Some(instance) = instances.get_mut(instance_id) {
            instance.last_heartbeat_at = Utc::now();
            instance.schedules_processed = schedules_processed;
        }
        Ok(())
    }

    async fn list_scheduler_instances(&self) -> Result<Vec<SchedulerInstanceInfo>, StoreError> {
        let instances = self.scheduler_instances.read();
        Ok(instances.values().cloned().collect())
    }

    async fn deregister_scheduler_instance(&self, instance_id: &str) -> Result<(), StoreError> {
        let mut instances = self.scheduler_instances.write();
        instances.remove(instance_id);
        Ok(())
    }
}
