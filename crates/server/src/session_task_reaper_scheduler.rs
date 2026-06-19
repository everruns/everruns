// Durable session-task reaper schedule bootstrap.
//
// Mirrors leased_resource_scheduler.rs exactly in structure and cadence
// (every 60 seconds). The schedule seeds the `session_task_reaper` activity
// which fails tasks whose worker heartbeat has gone stale.

use anyhow::{Context, Result};
use chrono::Utc;
use everruns_durable::{
    CreateScheduleRow, Pagination, ScheduleFilter, ScheduleTargetType, UpdateField, UpdateSchedule,
    WorkflowEventStore,
};
use std::str::FromStr;
use std::sync::Arc;

const SESSION_TASK_REAPER_SCHEDULE_NAME: &str = "session-task-reaper";
pub const SESSION_TASK_REAPER_ACTIVITY: &str = "session_task_reaper";
// Every 60 seconds — same cadence as the leased-resource cleanup.
const SESSION_TASK_REAPER_CRON: &str = "0 * * * * * *";
const SESSION_TASK_REAPER_TIMEZONE: &str = "UTC";
const SESSION_TASK_REAPER_DESCRIPTION: &str =
    "Fails session tasks whose worker heartbeat has gone stale (orphan reconciler).";

/// Ensure the durable session-task reaper schedule exists with the expected definition.
pub async fn ensure_session_task_reaper_schedule(
    store: Arc<dyn WorkflowEventStore + Send + Sync>,
) -> Result<()> {
    // Seed the durable activity input from env so the retention TTL
    // (SESSION_TASK_RETENTION_TTL_SECONDS) flows through the same configuration
    // path as the reaper schedule itself (EVE-580).
    let desired_input = serde_json::to_value(
        everruns_worker::session_task_reaper::SessionTaskReaperInput::from_env(),
    )?;
    let desired_next_trigger = next_trigger()?;

    let existing = store
        .list_schedules(
            ScheduleFilter::default(),
            Pagination {
                offset: 0,
                limit: 1_000,
            },
        )
        .await?
        .into_iter()
        .find(|schedule| schedule.name == SESSION_TASK_REAPER_SCHEDULE_NAME);

    if let Some(schedule) = existing {
        let needs_update = schedule.description.as_deref() != Some(SESSION_TASK_REAPER_DESCRIPTION)
            || schedule.cron_expression != SESSION_TASK_REAPER_CRON
            || schedule.timezone != SESSION_TASK_REAPER_TIMEZONE
            || schedule.target_type != ScheduleTargetType::Activity
            || schedule.target_name != SESSION_TASK_REAPER_ACTIVITY
            || schedule.target_input != desired_input
            || !schedule.enabled
            || schedule.max_concurrent != Some(1)
            || schedule.catch_up_missed;

        if needs_update {
            store
                .update_schedule(
                    schedule.id,
                    UpdateSchedule {
                        description: UpdateField::Set(SESSION_TASK_REAPER_DESCRIPTION.to_string()),
                        cron_expression: Some(SESSION_TASK_REAPER_CRON.to_string()),
                        timezone: Some(SESSION_TASK_REAPER_TIMEZONE.to_string()),
                        target_type: Some(ScheduleTargetType::Activity),
                        target_name: Some(SESSION_TASK_REAPER_ACTIVITY.to_string()),
                        target_input: Some(desired_input),
                        enabled: Some(true),
                        max_concurrent: UpdateField::Set(1),
                        catch_up_missed: Some(false),
                        max_catch_up: UpdateField::Set(1),
                        next_trigger_at: UpdateField::Set(desired_next_trigger),
                        ..Default::default()
                    },
                )
                .await?;

            tracing::info!(
                schedule_id = %schedule.id,
                "Updated durable session-task reaper schedule"
            );
        } else {
            tracing::info!(
                schedule_id = %schedule.id,
                "Durable session-task reaper schedule already configured"
            );
        }

        return Ok(());
    }

    match store
        .create_schedule(CreateScheduleRow {
            name: SESSION_TASK_REAPER_SCHEDULE_NAME.to_string(),
            description: Some(SESSION_TASK_REAPER_DESCRIPTION.to_string()),
            cron_expression: SESSION_TASK_REAPER_CRON.to_string(),
            timezone: SESSION_TASK_REAPER_TIMEZONE.to_string(),
            target_type: ScheduleTargetType::Activity,
            target_name: SESSION_TASK_REAPER_ACTIVITY.to_string(),
            target_input: desired_input,
            enabled: true,
            max_concurrent: Some(1),
            catch_up_missed: false,
            max_catch_up: Some(1),
            retry_policy: None,
            next_trigger_at: Some(desired_next_trigger),
        })
        .await
    {
        Ok(schedule_id) => {
            tracing::info!(
                schedule_id = %schedule_id,
                "Created durable session-task reaper schedule"
            );
            Ok(())
        }
        Err(everruns_durable::StoreError::Database(error))
            if error.contains("duplicate") || error.contains("unique") =>
        {
            tracing::info!("Session-task reaper schedule already exists");
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

fn next_trigger() -> Result<chrono::DateTime<Utc>> {
    let schedule = cron::Schedule::from_str(SESSION_TASK_REAPER_CRON)
        .context("Invalid session-task reaper cron expression")?;
    schedule
        .upcoming(Utc)
        .next()
        .context("Session-task reaper cron produced no next trigger")
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_durable::InMemoryWorkflowEventStore;

    async fn list_reaper_schedules(
        store: &InMemoryWorkflowEventStore,
    ) -> Vec<everruns_durable::ScheduleRow> {
        store
            .list_schedules(
                ScheduleFilter::default(),
                Pagination {
                    offset: 0,
                    limit: 100,
                },
            )
            .await
            .expect("schedules should list")
            .into_iter()
            .filter(|schedule| schedule.name == SESSION_TASK_REAPER_SCHEDULE_NAME)
            .collect()
    }

    #[tokio::test]
    async fn ensures_expected_reaper_schedule_once() {
        let store = Arc::new(InMemoryWorkflowEventStore::new());

        ensure_session_task_reaper_schedule(store.clone())
            .await
            .expect("reaper schedule should bootstrap");
        ensure_session_task_reaper_schedule(store.clone())
            .await
            .expect("reaper schedule bootstrap should be idempotent");

        let schedules = list_reaper_schedules(store.as_ref()).await;
        assert_eq!(schedules.len(), 1);

        let schedule = &schedules[0];
        assert_eq!(
            schedule.description.as_deref(),
            Some(SESSION_TASK_REAPER_DESCRIPTION)
        );
        assert_eq!(schedule.cron_expression, SESSION_TASK_REAPER_CRON);
        assert_eq!(schedule.timezone, SESSION_TASK_REAPER_TIMEZONE);
        assert_eq!(schedule.target_type, ScheduleTargetType::Activity);
        assert_eq!(schedule.target_name, SESSION_TASK_REAPER_ACTIVITY);
        assert_eq!(
            schedule.target_input,
            serde_json::to_value(
                everruns_worker::session_task_reaper::SessionTaskReaperInput::default()
            )
            .expect("default reaper input should serialize")
        );
        assert!(schedule.enabled);
        assert_eq!(schedule.max_concurrent, Some(1));
        assert!(!schedule.catch_up_missed);
        assert_eq!(schedule.max_catch_up, Some(1));
        assert!(schedule.next_trigger_at.is_some());
    }
}
