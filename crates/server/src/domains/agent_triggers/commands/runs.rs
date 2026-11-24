//! Recent runs of a schedule trigger and their outcome.

use super::*;

// ============================================================================
// ListAgentTriggerRuns
// ============================================================================

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ListAgentTriggerRuns {
    /// Owning agent's prefixed public identifier.
    pub agent_id: String,
    /// Agent trigger's prefixed public identifier.
    pub trigger_id: String,
}

#[command(
    name = "list_agent_trigger_runs",
    category = "agent_triggers",
    description = "List recent execution outcomes for an agent trigger.",
    method = "GET",
    path = "/v1/platform/agents/{agent_id}/triggers/{trigger_id}/runs",
    policy = AGENT_VIEW,
    cli = CliRoute::new(&["agents", "triggers", "runs"], "list").with_examples(&[CliExample::new("See whether recent firings of a trigger succeeded", "everruns agents triggers runs list --agent-id agent_01h9 --trigger-id trg_01h9")]),
)]
impl Command for ListAgentTriggerRuns {
    type Output = Vec<AgentTriggerRun>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<AgentTriggerRun>, CommandError> {
        let (_, trigger) = resolve_trigger_for_agent(ctx, &self.agent_id, &self.trigger_id).await?;
        let Some(schedule_id) = trigger.durable_schedule_id else {
            return Ok(Vec::new());
        };
        let executions = durable_store(ctx)?
            .list_schedule_executions(
                ScheduleExecutionFilter {
                    schedule_id: Some(schedule_id),
                    status: None,
                },
                DurablePagination {
                    offset: 0,
                    limit: 10,
                },
            )
            .await
            .map_err(|err| classify_anyhow(err.into()))?;
        let store = durable_store(ctx)?;
        let mut runs = Vec::with_capacity(executions.len());
        for execution in executions {
            let mut run = AgentTriggerRun {
                id: execution.id.to_string(),
                status: execution.status.to_string(),
                scheduled_at: execution.scheduled_at,
                completed_at: execution.completed_at,
                error: execution.error,
            };
            // A schedule tick "completes" once it queues its task; the run's
            // outcome is that task's. Without this a run whose task failed
            // reads as completed while no session ever started.
            if let Some(task_id) = execution.task_id
                && run.status == "completed"
            {
                match store.get_task(task_id).await {
                    Ok(task) => apply_task_outcome(&mut run, &task),
                    Err(error) => {
                        tracing::debug!(%error, %task_id, "trigger run task not readable")
                    }
                }
            }
            runs.push(run);
        }
        Ok(runs)
    }
}

/// Fold a scheduled run's task state into the run.
pub(super) fn apply_task_outcome(run: &mut AgentTriggerRun, task: &everruns_durable::TaskInfo) {
    use everruns_durable::TaskStatus;
    let (status, error) = match task.status {
        TaskStatus::Completed => return,
        TaskStatus::Pending if task.attempt > 0 => ("retrying", task.last_error.clone()),
        TaskStatus::Pending => ("queued", None),
        TaskStatus::Claimed => ("running", None),
        TaskStatus::Failed | TaskStatus::Dead => ("failed", task.last_error.clone()),
        TaskStatus::Cancelled => ("cancelled", task.last_error.clone()),
    };
    run.status = status.to_string();
    if matches!(status, "queued" | "running" | "retrying") {
        run.completed_at = None;
    }
    if error.is_some() {
        run.error = error;
    }
}
