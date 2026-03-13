// Durable leased-resource cleanup schedule bootstrap.
//
// The cleanup schedule is part of the control plane, not any individual
// provider. Any subsystem that registers leased resources automatically joins
// the same durable cleanup mechanism and observability surface.

use anyhow::{Context, Result};
use chrono::Utc;
use everruns_durable::{
    CreateScheduleRow, Pagination, ScheduleFilter, ScheduleTargetType, UpdateSchedule,
    WorkflowEventStore,
};
use std::str::FromStr;
use std::sync::Arc;

const LEASED_RESOURCE_CLEANUP_SCHEDULE_NAME: &str = "leased-resource-cleanup";
pub const LEASED_RESOURCE_CLEANUP_ACTIVITY: &str = "leased_resource_cleanup";
const LEASED_RESOURCE_CLEANUP_CRON: &str = "0 * * * * * *";
const LEASED_RESOURCE_CLEANUP_TIMEZONE: &str = "UTC";
const LEASED_RESOURCE_CLEANUP_DESCRIPTION: &str = "Generic cleanup for leased provider resources such as Daytona sandboxes and Browserless sessions.";

/// Ensure the durable leased-resource cleanup schedule exists with the expected definition.
pub async fn ensure_leased_resource_cleanup_schedule(
    store: Arc<dyn WorkflowEventStore + Send + Sync>,
) -> Result<()> {
    let desired_input = serde_json::to_value(
        everruns_worker::leased_resource_cleanup::LeasedResourceCleanupInput::default(),
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
        .find(|schedule| schedule.name == LEASED_RESOURCE_CLEANUP_SCHEDULE_NAME);

    if let Some(schedule) = existing {
        let needs_update = schedule.description.as_deref()
            != Some(LEASED_RESOURCE_CLEANUP_DESCRIPTION)
            || schedule.cron_expression != LEASED_RESOURCE_CLEANUP_CRON
            || schedule.timezone != LEASED_RESOURCE_CLEANUP_TIMEZONE
            || schedule.target_type != ScheduleTargetType::Activity
            || schedule.target_name != LEASED_RESOURCE_CLEANUP_ACTIVITY
            || schedule.target_input != desired_input
            || !schedule.enabled
            || schedule.max_concurrent != Some(1)
            || schedule.catch_up_missed;

        if needs_update {
            store
                .update_schedule(
                    schedule.id,
                    UpdateSchedule {
                        description: Some(Some(LEASED_RESOURCE_CLEANUP_DESCRIPTION.to_string())),
                        cron_expression: Some(LEASED_RESOURCE_CLEANUP_CRON.to_string()),
                        timezone: Some(LEASED_RESOURCE_CLEANUP_TIMEZONE.to_string()),
                        target_type: Some(ScheduleTargetType::Activity),
                        target_name: Some(LEASED_RESOURCE_CLEANUP_ACTIVITY.to_string()),
                        target_input: Some(desired_input),
                        enabled: Some(true),
                        max_concurrent: Some(Some(1)),
                        catch_up_missed: Some(false),
                        max_catch_up: Some(Some(1)),
                        next_trigger_at: Some(Some(desired_next_trigger)),
                        ..Default::default()
                    },
                )
                .await?;

            tracing::info!(
                schedule_id = %schedule.id,
                "Updated durable leased-resource cleanup schedule"
            );
        } else {
            tracing::info!(
                schedule_id = %schedule.id,
                "Durable leased-resource cleanup schedule already configured"
            );
        }

        return Ok(());
    }

    match store
        .create_schedule(CreateScheduleRow {
            name: LEASED_RESOURCE_CLEANUP_SCHEDULE_NAME.to_string(),
            description: Some(LEASED_RESOURCE_CLEANUP_DESCRIPTION.to_string()),
            cron_expression: LEASED_RESOURCE_CLEANUP_CRON.to_string(),
            timezone: LEASED_RESOURCE_CLEANUP_TIMEZONE.to_string(),
            target_type: ScheduleTargetType::Activity,
            target_name: LEASED_RESOURCE_CLEANUP_ACTIVITY.to_string(),
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
                "Created durable leased-resource cleanup schedule"
            );
            Ok(())
        }
        Err(everruns_durable::StoreError::Database(error))
            if error.contains("duplicate") || error.contains("unique") =>
        {
            tracing::info!("Leased-resource cleanup schedule already exists");
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

fn next_trigger() -> Result<chrono::DateTime<Utc>> {
    let schedule = cron::Schedule::from_str(LEASED_RESOURCE_CLEANUP_CRON)
        .context("Invalid leased-resource cleanup cron expression")?;
    schedule
        .upcoming(Utc)
        .next()
        .context("Leased-resource cleanup cron produced no next trigger")
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_durable::{CreateScheduleRow, InMemoryWorkflowEventStore};

    async fn list_cleanup_schedules(
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
            .filter(|schedule| schedule.name == LEASED_RESOURCE_CLEANUP_SCHEDULE_NAME)
            .collect()
    }

    #[tokio::test]
    async fn ensures_expected_cleanup_schedule_once() {
        let store = Arc::new(InMemoryWorkflowEventStore::new());

        ensure_leased_resource_cleanup_schedule(store.clone())
            .await
            .expect("cleanup schedule should bootstrap");
        ensure_leased_resource_cleanup_schedule(store.clone())
            .await
            .expect("cleanup schedule bootstrap should be idempotent");

        let schedules = list_cleanup_schedules(store.as_ref()).await;
        assert_eq!(schedules.len(), 1);

        let schedule = &schedules[0];
        assert_eq!(
            schedule.description.as_deref(),
            Some(LEASED_RESOURCE_CLEANUP_DESCRIPTION)
        );
        assert_eq!(schedule.cron_expression, LEASED_RESOURCE_CLEANUP_CRON);
        assert_eq!(schedule.timezone, LEASED_RESOURCE_CLEANUP_TIMEZONE);
        assert_eq!(schedule.target_type, ScheduleTargetType::Activity);
        assert_eq!(schedule.target_name, LEASED_RESOURCE_CLEANUP_ACTIVITY);
        assert_eq!(
            schedule.target_input,
            serde_json::to_value(
                everruns_worker::leased_resource_cleanup::LeasedResourceCleanupInput::default()
            )
            .expect("default cleanup input should serialize")
        );
        assert!(schedule.enabled);
        assert_eq!(schedule.max_concurrent, Some(1));
        assert!(!schedule.catch_up_missed);
        assert_eq!(schedule.max_catch_up, Some(1));
        assert!(schedule.next_trigger_at.is_some());
    }

    #[tokio::test]
    async fn updates_existing_schedule_drift() {
        let store = Arc::new(InMemoryWorkflowEventStore::new());

        store
            .create_schedule(CreateScheduleRow {
                name: LEASED_RESOURCE_CLEANUP_SCHEDULE_NAME.to_string(),
                description: Some("outdated".to_string()),
                cron_expression: "*/5 * * * *".to_string(),
                timezone: "America/Chicago".to_string(),
                target_type: ScheduleTargetType::Workflow,
                target_name: "wrong_target".to_string(),
                target_input: serde_json::json!({ "batch_size": 1 }),
                enabled: false,
                max_concurrent: Some(3),
                catch_up_missed: true,
                max_catch_up: Some(9),
                retry_policy: None,
                next_trigger_at: None,
            })
            .await
            .expect("outdated schedule should be created");

        ensure_leased_resource_cleanup_schedule(store.clone())
            .await
            .expect("cleanup schedule should be reconciled");

        let schedules = list_cleanup_schedules(store.as_ref()).await;
        assert_eq!(schedules.len(), 1);
        let schedule = &schedules[0];

        assert_eq!(
            schedule.description.as_deref(),
            Some(LEASED_RESOURCE_CLEANUP_DESCRIPTION)
        );
        assert_eq!(schedule.cron_expression, LEASED_RESOURCE_CLEANUP_CRON);
        assert_eq!(schedule.timezone, LEASED_RESOURCE_CLEANUP_TIMEZONE);
        assert_eq!(schedule.target_type, ScheduleTargetType::Activity);
        assert_eq!(schedule.target_name, LEASED_RESOURCE_CLEANUP_ACTIVITY);
        assert!(schedule.enabled);
        assert_eq!(schedule.max_concurrent, Some(1));
        assert!(!schedule.catch_up_missed);
        assert_eq!(schedule.max_catch_up, Some(1));
        assert!(schedule.next_trigger_at.is_some());
    }
}
