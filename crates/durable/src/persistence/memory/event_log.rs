//! EventLog implementation (see `store.rs` for the trait contract).

use super::*;

#[async_trait]
impl EventLog for InMemoryWorkflowEventStore {
    async fn create_workflow(
        &self,
        workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
        _trace_context: Option<&TraceContext>,
    ) -> Result<(), StoreError> {
        let mut workflows = self.workflows.write();
        workflows.insert(
            workflow_id,
            WorkflowState {
                workflow_type: workflow_type.to_string(),
                status: WorkflowStatus::Pending,
                input,
                result: None,
                error: None,
                events: vec![],
                signals: vec![],
                created_at: Utc::now(),
                started_at: None,
                completed_at: None,
                continued_as_new_id: None,
            },
        );
        Ok(())
    }

    async fn get_workflow_status(&self, workflow_id: Uuid) -> Result<WorkflowStatus, StoreError> {
        let workflows = self.workflows.read();
        workflows
            .get(&workflow_id)
            .map(|w| w.status)
            .ok_or(StoreError::WorkflowNotFound(workflow_id))
    }

    async fn get_workflow_info(&self, workflow_id: Uuid) -> Result<WorkflowInfo, StoreError> {
        let workflows = self.workflows.read();
        let workflow = workflows
            .get(&workflow_id)
            .ok_or(StoreError::WorkflowNotFound(workflow_id))?;

        Ok(WorkflowInfo {
            id: workflow_id,
            workflow_type: workflow.workflow_type.clone(),
            status: workflow.status,
            input: workflow.input.clone(),
            result: workflow.result.clone(),
            error: workflow.error.clone(),
            continued_as_new_id: workflow.continued_as_new_id,
        })
    }

    async fn append_events(
        &self,
        workflow_id: Uuid,
        expected_sequence: i32,
        events: Vec<WorkflowEvent>,
    ) -> Result<i32, StoreError> {
        let mut workflows = self.workflows.write();
        let workflow = workflows
            .get_mut(&workflow_id)
            .ok_or(StoreError::WorkflowNotFound(workflow_id))?;

        let current_sequence = workflow.events.len() as i32;
        if current_sequence != expected_sequence {
            return Err(StoreError::ConcurrencyConflict {
                expected: expected_sequence,
                actual: current_sequence,
            });
        }

        workflow.events.extend(events);
        Ok(workflow.events.len() as i32)
    }

    async fn load_events(
        &self,
        workflow_id: Uuid,
    ) -> Result<Vec<(i32, WorkflowEvent)>, StoreError> {
        #[cfg(test)]
        self.load_events_calls.fetch_add(1, Ordering::Relaxed);

        let workflows = self.workflows.read();
        let workflow = workflows
            .get(&workflow_id)
            .ok_or(StoreError::WorkflowNotFound(workflow_id))?;

        Ok(workflow
            .events
            .iter()
            .enumerate()
            .map(|(i, e)| (i as i32, e.clone()))
            .collect())
    }

    async fn count_events(&self, workflow_id: Uuid) -> Result<usize, StoreError> {
        #[cfg(test)]
        self.count_events_calls.fetch_add(1, Ordering::Relaxed);

        let workflows = self.workflows.read();
        let workflow = workflows
            .get(&workflow_id)
            .ok_or(StoreError::WorkflowNotFound(workflow_id))?;

        Ok(workflow.events.len())
    }

    async fn count_events_after(
        &self,
        workflow_id: Uuid,
        after_sequence: i32,
    ) -> Result<usize, StoreError> {
        let workflows = self.workflows.read();
        let workflow = workflows
            .get(&workflow_id)
            .ok_or(StoreError::WorkflowNotFound(workflow_id))?;

        Ok(workflow
            .events
            .iter()
            .enumerate()
            .filter(|(i, _)| (*i as i32) > after_sequence)
            .count())
    }

    async fn load_events_after(
        &self,
        workflow_id: Uuid,
        after_sequence: i32,
    ) -> Result<Vec<(i32, WorkflowEvent)>, StoreError> {
        let workflows = self.workflows.read();
        let workflow = workflows
            .get(&workflow_id)
            .ok_or(StoreError::WorkflowNotFound(workflow_id))?;

        Ok(workflow
            .events
            .iter()
            .enumerate()
            .filter(|(i, _)| (*i as i32) > after_sequence)
            .map(|(i, e)| (i as i32, e.clone()))
            .collect())
    }

    async fn save_snapshot(
        &self,
        workflow_id: Uuid,
        sequence_num: i32,
        snapshot_data: Vec<u8>,
    ) -> Result<(), StoreError> {
        let mut snapshots = self.snapshots.write();
        let entry = snapshots.entry(workflow_id).or_default();

        // UPSERT: replace if same sequence_num exists
        if let Some(existing) = entry.iter_mut().find(|s| s.sequence_num == sequence_num) {
            existing.snapshot_data = snapshot_data;
            existing.created_at = Utc::now();
        } else {
            entry.push(SnapshotMemState {
                sequence_num,
                snapshot_data,
                created_at: Utc::now(),
            });
            entry.sort_by_key(|s| s.sequence_num);
        }

        Ok(())
    }

    async fn load_latest_snapshot(
        &self,
        workflow_id: Uuid,
    ) -> Result<Option<WorkflowSnapshot>, StoreError> {
        let snapshots = self.snapshots.read();
        let snapshot = snapshots
            .get(&workflow_id)
            .and_then(|entries| entries.last())
            .map(|s| WorkflowSnapshot {
                workflow_id,
                sequence_num: s.sequence_num,
                snapshot_data: s.snapshot_data.clone(),
                created_at: s.created_at,
            });
        Ok(snapshot)
    }

    async fn delete_snapshots(&self, workflow_id: Uuid) -> Result<(), StoreError> {
        self.snapshots.write().remove(&workflow_id);
        Ok(())
    }

    async fn update_workflow_status(
        &self,
        workflow_id: Uuid,
        status: WorkflowStatus,
        result: Option<serde_json::Value>,
        error: Option<WorkflowError>,
    ) -> Result<(), StoreError> {
        let mut workflows = self.workflows.write();
        let workflow = workflows
            .get_mut(&workflow_id)
            .ok_or(StoreError::WorkflowNotFound(workflow_id))?;

        workflow.status = status;
        if matches!(status, WorkflowStatus::Pending) {
            workflow.result = result;
            workflow.error = None;
            workflow.started_at = None;
            workflow.completed_at = None;
        } else {
            workflow.result = result;
            workflow.error = error;
            match status {
                WorkflowStatus::Running => {
                    if workflow.started_at.is_none() {
                        workflow.started_at = Some(Utc::now());
                    }
                }
                WorkflowStatus::Completed
                | WorkflowStatus::Failed
                | WorkflowStatus::Cancelled
                | WorkflowStatus::ContinuedAsNew => {
                    if workflow.completed_at.is_none() {
                        workflow.completed_at = Some(Utc::now());
                    }
                }
                WorkflowStatus::Pending => {} // handled above
            }
        }
        Ok(())
    }

    async fn try_fail_workflow(
        &self,
        workflow_id: Uuid,
        error: WorkflowError,
    ) -> Result<bool, StoreError> {
        let mut workflows = self.workflows.write();
        let workflow = workflows
            .get_mut(&workflow_id)
            .ok_or(StoreError::WorkflowNotFound(workflow_id))?;
        if workflow.status != WorkflowStatus::Running {
            return Ok(false);
        }

        workflow.status = WorkflowStatus::Failed;
        workflow.error = Some(error);
        workflow.completed_at = Some(Utc::now());
        Ok(true)
    }

    async fn continue_as_new(
        &self,
        old_workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
        snapshot_data: Vec<u8>,
    ) -> Result<Uuid, StoreError> {
        let new_workflow_id = Uuid::now_v7();
        let mut workflows = self.workflows.write();

        // Verify old workflow exists and is running
        let old_workflow = workflows
            .get_mut(&old_workflow_id)
            .ok_or(StoreError::WorkflowNotFound(old_workflow_id))?;

        if old_workflow.status.is_terminal() {
            return Err(StoreError::Database(format!(
                "workflow {old_workflow_id} is already terminal"
            )));
        }

        // Mark old workflow as continued
        old_workflow.status = WorkflowStatus::ContinuedAsNew;
        old_workflow.completed_at = Some(Utc::now());
        old_workflow.continued_as_new_id = Some(new_workflow_id);

        // Clear old events (archive)
        old_workflow.events.clear();

        // Create new workflow with a WorkflowStarted event
        let now = Utc::now();
        workflows.insert(
            new_workflow_id,
            WorkflowState {
                workflow_type: workflow_type.to_string(),
                status: WorkflowStatus::Running,
                input: input.clone(),
                result: None,
                error: None,
                events: vec![WorkflowEvent::started(input.clone())],
                signals: vec![],
                created_at: now,
                started_at: Some(now),
                completed_at: None,
                continued_as_new_id: None,
            },
        );

        // Save snapshot on the new workflow at sequence 0 (before the start event)
        drop(workflows);
        let mut snapshots = self.snapshots.write();
        // Delete old workflow snapshots
        snapshots.remove(&old_workflow_id);
        // Save snapshot on new workflow
        snapshots
            .entry(new_workflow_id)
            .or_default()
            .push(SnapshotMemState {
                sequence_num: 0,
                snapshot_data,
                created_at: now,
            });

        Ok(new_workflow_id)
    }

    async fn try_start_new_run(&self, workflow_id: Uuid) -> Result<bool, StoreError> {
        {
            let tasks = self.tasks.read();
            let has_claimed_task = tasks.ids_for_workflow(workflow_id).iter().any(|id| {
                tasks
                    .get(id)
                    .is_some_and(|t| t.status == TaskStatus::Claimed)
            });
            if has_claimed_task {
                return Ok(false);
            }
        }

        {
            let mut workflows = self.workflows.write();
            let workflow = workflows
                .get_mut(&workflow_id)
                .ok_or(StoreError::WorkflowNotFound(workflow_id))?;

            if workflow.status == WorkflowStatus::Running {
                return Ok(false);
            }

            // Claim: set to Running, clear transient fields
            workflow.status = WorkflowStatus::Running;
            workflow.result = None;
            workflow.error = None;
            workflow.started_at = Some(chrono::Utc::now());
            workflow.completed_at = None;
        }

        // Cancel stale pending tasks
        self.cancel_pending_tasks_for_workflow(workflow_id).await?;
        Ok(true)
    }

    async fn cancel_workflow(&self, workflow_id: Uuid) -> Result<(), StoreError> {
        {
            let mut workflows = self.workflows.write();
            let workflow = workflows
                .get_mut(&workflow_id)
                .filter(|w| matches!(w.status, WorkflowStatus::Pending | WorkflowStatus::Running))
                .ok_or(StoreError::WorkflowNotFound(workflow_id))?;
            workflow.status = WorkflowStatus::Cancelled;
            workflow.completed_at = Some(Utc::now());
        }
        self.cancel_pending_tasks_for_workflow(workflow_id).await?;
        Ok(())
    }
}
