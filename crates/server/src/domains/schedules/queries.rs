use crate::domains::common::{CommandError, Ctx};
use anyhow::anyhow;
use chrono::{DateTime, Utc};
use everruns_durable::{
    ScheduleExecutionStatus, ScheduleTargetType, StoreError, WorkflowEventStore,
};
use std::str::FromStr;
use std::sync::Arc;

pub fn ensure_platform_user(ctx: &Ctx) -> Result<(), CommandError> {
    if ctx.caller.is_platform_user {
        Ok(())
    } else {
        Err(CommandError::Forbidden(
            "Platform user access is required".to_string(),
        ))
    }
}

pub fn store(ctx: &Ctx) -> Result<Arc<dyn WorkflowEventStore + Send + Sync>, CommandError> {
    ctx.workflow_store
        .clone()
        .ok_or_else(|| CommandError::Internal(anyhow!("Durable execution store not available")))
}

pub fn parse_target_type(value: &str) -> Result<ScheduleTargetType, CommandError> {
    match value {
        "workflow" => Ok(ScheduleTargetType::Workflow),
        "activity" => Ok(ScheduleTargetType::Activity),
        _ => Err(CommandError::bad_request(
            "Invalid target type. Must be 'workflow' or 'activity'",
        )),
    }
}

pub fn optional_target_type(
    value: Option<&str>,
) -> Result<Option<ScheduleTargetType>, CommandError> {
    value.map(parse_target_type).transpose()
}

pub fn parse_execution_status(value: &str) -> Result<ScheduleExecutionStatus, CommandError> {
    match value {
        "pending" => Ok(ScheduleExecutionStatus::Pending),
        "running" => Ok(ScheduleExecutionStatus::Running),
        "completed" => Ok(ScheduleExecutionStatus::Completed),
        "failed" => Ok(ScheduleExecutionStatus::Failed),
        "skipped" => Ok(ScheduleExecutionStatus::Skipped),
        _ => Err(CommandError::bad_request(
            "Invalid execution status. Must be one of pending, running, completed, failed, skipped",
        )),
    }
}

pub fn calculate_next_trigger(
    cron_expression: &str,
) -> Result<Option<DateTime<Utc>>, CommandError> {
    let normalized = normalize_cron_expression(cron_expression)?;
    let schedule = cron::Schedule::from_str(&normalized)
        .map_err(|e| CommandError::bad_request(format!("Invalid cron expression: {e}")))?;
    Ok(schedule.upcoming(Utc).next())
}

pub fn normalize_cron_expression(cron_expression: &str) -> Result<String, CommandError> {
    let fields = cron_expression.split_whitespace().collect::<Vec<_>>();
    let normalized = match fields.len() {
        5 => format!("0 {} *", fields.join(" ")),
        7 => fields.join(" "),
        _ => {
            return Err(CommandError::bad_request(
                "Cron expression must be either 5 fields (min hour day month weekday) or 7 fields (sec min hour day month weekday year)",
            ));
        }
    };
    cron::Schedule::from_str(&normalized)
        .map_err(|e| CommandError::bad_request(format!("Invalid cron expression: {e}")))?;
    Ok(normalized)
}

pub fn map_store_error(error: StoreError) -> CommandError {
    match error {
        StoreError::ScheduleNotFound(_) => CommandError::not_found("Schedule"),
        StoreError::ScheduleExecutionNotFound(_) => CommandError::not_found("Execution"),
        StoreError::InvalidCronExpression(message) => {
            CommandError::bad_request(format!("Invalid cron expression: {message}"))
        }
        StoreError::CronIntervalTooShort { actual, minimum } => CommandError::bad_request(format!(
            "Cron interval too short: {actual}s < {minimum}s minimum"
        )),
        StoreError::ScheduleLimitExceeded { limit, .. } => {
            CommandError::conflict(format!("Schedule limit exceeded: max {limit} schedules"))
        }
        other => CommandError::Internal(anyhow!(other)),
    }
}
