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
        Err(CommandError::forbidden(
            "Platform user access is required".to_string(),
        ))
    }
}

pub fn store(ctx: &Ctx) -> Result<Arc<dyn WorkflowEventStore + Send + Sync>, CommandError> {
    ctx.workflow_store
        .clone()
        .ok_or_else(|| CommandError::internal(anyhow!("Durable execution store not available")))
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
        // Standard cron weekdays (0-6 from Sunday) become day names; see
        // `standard_cron_to_crate_form`. Seven fields are the `cron` crate's
        // own form and keep its numbering (1-7 from Sunday).
        5 => everruns_core::session_schedule::standard_cron_to_crate_form(cron_expression)
            .expect("five fields"),
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
        other => CommandError::internal(anyhow!(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, TimeZone, Weekday};

    fn next_weekday(cron_expression: &str) -> Weekday {
        let normalized = normalize_cron_expression(cron_expression).expect("valid cron");
        let schedule = cron::Schedule::from_str(&normalized).expect("crate form parses");
        // A Monday at noon.
        let after = Utc.with_ymd_and_hms(2026, 10, 12, 12, 0, 0).unwrap();
        schedule.after(&after).next().expect("next run").weekday()
    }

    /// Standard cron counts weekdays 0-6 from Sunday (EVE-1250): `0 18 * * 0`
    /// was rejected and `0 9 * * 1` fired on Sundays.
    #[test]
    fn five_field_weekdays_count_from_sunday_zero() {
        assert_eq!(next_weekday("0 18 * * 0"), Weekday::Sun);
        assert_eq!(next_weekday("0 18 * * 7"), Weekday::Sun);
        assert_eq!(next_weekday("0 9 * * 1"), Weekday::Mon);
        assert_eq!(next_weekday("0 9 * * 2"), Weekday::Tue);
        assert_eq!(
            normalize_cron_expression("0 9 * * 1-5").unwrap(),
            "0 0 9 * * MON-FRI *"
        );
        // Seven fields are the `cron` crate's form and pass through unchanged.
        assert_eq!(
            normalize_cron_expression("0 0 9 * * 2-6 *").unwrap(),
            "0 0 9 * * 2-6 *"
        );
        assert!(normalize_cron_expression("0 9 * *").is_err());
    }
}
