//! Recent runs of a schedule trigger and their outcome.

use super::*;

#[derive(sqlx::FromRow)]
struct RunEvidence {
    requested_by: Option<uuid::Uuid>,
    source: String,
    trigger_revision: chrono::DateTime<Utc>,
    input_message_id: Option<uuid::Uuid>,
    configuration: Value,
}

// ============================================================================
// ListAgentTriggerRuns
// ============================================================================

#[derive(Debug, Default, Deserialize, ToSchema, serde::Serialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListAgentTriggerRuns {
    pub agent_id: String,
    pub trigger_id: String,
    /// Inclusive scheduled time.
    pub from: Option<chrono::DateTime<Utc>>,
    /// Exclusive scheduled time.
    pub to: Option<chrono::DateTime<Utc>>,
    /// Stable cursor: return executions older than this execution.
    pub before: Option<uuid::Uuid>,
    /// Page size, 1-200; default 10.
    pub limit: Option<i64>,
}

#[command(
    name = "list_agent_trigger_runs", category = "agent_triggers",
    description = "List executions in a trigger's current durable schedule binding, with time windows, attempts, authenticated manual requester and resulting session. Disabling a trigger removes durable execution history; use list_agent_trigger_deliveries for retained event history. Historical attribution can be unknown.",
    method = "GET", path = "/v1/platform/agents/{agent_id}/triggers/{trigger_id}/runs",
    policy = AGENT_VIEW,
    cli = CliRoute::new(&["agents", "triggers", "runs"], "list").with_examples(&[CliExample::new("See recent trigger runs and the sessions they started", "everruns agents triggers runs list --agent-id agent_01h9 --trigger-id trg_01h9 --limit 20")]),
)]
impl Command for ListAgentTriggerRuns {
    type Output = Vec<AgentTriggerRun>;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        super::super::deliveries::validate_window(self.from, self.to)?;
        let (_, trigger) = resolve_trigger_for_agent(ctx, &self.agent_id, &self.trigger_id).await?;
        let Some(schedule) = trigger.durable_schedule_id else {
            return Ok(Vec::new());
        };
        let store = durable_store(ctx)?;
        let cursor = if let Some(id) = self.before {
            let execution = store
                .get_schedule_execution(id)
                .await
                .map_err(|error| classify_anyhow(error.into()))?;
            if execution.schedule_id != schedule {
                return Err(CommandError::not_found("Trigger run cursor"));
            }
            Some((execution.scheduled_at, execution.id))
        } else {
            None
        };
        // THREAT[TM-TENANT]: the schedule is resolved through this org's agent;
        // the join also enforces ownership in the history query itself.
        let ids: Vec<uuid::Uuid> = sqlx::query_scalar(
            "SELECT e.id FROM durable_schedule_executions e JOIN agent_triggers t ON t.durable_schedule_id=e.schedule_id
             WHERE t.org_id=$1 AND t.id=$2 AND e.schedule_id=$3
               AND ($4::timestamptz IS NULL OR e.scheduled_at >= $4)
               AND ($5::timestamptz IS NULL OR e.scheduled_at < $5)
               AND ($6::timestamptz IS NULL OR (e.scheduled_at,e.id) < ($6,$7))
             ORDER BY e.scheduled_at DESC,e.id DESC LIMIT $8")
            .bind(ctx.org_id()).bind(trigger.id).bind(schedule).bind(self.from).bind(self.to)
            .bind(cursor.map(|c| c.0)).bind(cursor.map(|c| c.1)).bind(self.limit.unwrap_or(10).clamp(1,200))
            .fetch_all(ctx.db.database().pool()).await.map_err(|e| CommandError::internal(e.into()))?;
        let mut runs = Vec::with_capacity(ids.len());
        for id in ids {
            let execution = store
                .get_schedule_execution(id)
                .await
                .map_err(|e| classify_anyhow(e.into()))?;
            let evidence: Option<RunEvidence> = sqlx::query_as(
                "SELECT requested_by, source, trigger_revision, input_message_id, configuration FROM agent_trigger_run_evidence WHERE org_id=$1 AND trigger_id=$2 AND execution_id=$3")
                .bind(ctx.org_id()).bind(trigger.id).bind(id).fetch_optional(ctx.db.database().pool()).await
                .map_err(|e| CommandError::internal(e.into()))?;
            let mut run = AgentTriggerRun {
                input_message_id: evidence
                    .as_ref()
                    .and_then(|e| e.input_message_id)
                    .map(everruns_contracts::typed_id::MessageId::from_uuid),
                id: id.to_string(),
                status: execution.status.to_string(),
                scheduled_at: execution.scheduled_at,
                completed_at: execution.completed_at,
                error: execution.error,
                source: evidence.as_ref().map(|e| e.source.clone()),
                requested_by: evidence.as_ref().and_then(|e| e.requested_by),
                configuration: evidence.as_ref().map(|e| e.configuration.clone()),
                trigger_revision: evidence.map(|e| e.trigger_revision),
                attempts: None,
                session_id: None,
            };
            if run.source.as_deref() == Some("manual") {
                // Manual dispatch stores its resulting session in the target slot,
                // rather than creating a durable task.
                run.session_id = execution
                    .task_id
                    .or(execution.workflow_id)
                    .map(|id| SessionId::from_uuid(id).to_string());
                run.attempts = Some(1);
            } else if let Some(task_id) = execution.task_id {
                if let Ok(task) = store.get_task(task_id).await {
                    // Older manual fires used this target slot for a session ID.
                    // Only a real durable task establishes scheduled attribution.
                    run.source.get_or_insert_with(|| "schedule".into());
                    run.attempts = Some(task.attempt);
                    if run.status == "completed" {
                        apply_task_outcome(&mut run, &task);
                    }
                }
                if let Some(result) = ctx
                    .db
                    .scheduled_trigger_invocation(task_id, ctx.org_id(), trigger.id)
                    .await?
                    && let Ok(result) =
                        serde_json::from_value::<AgentTriggerInvocationResult>(result)
                {
                    run.session_id = Some(result.session_id.to_string());
                    run.input_message_id = result.input_message_id;
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
