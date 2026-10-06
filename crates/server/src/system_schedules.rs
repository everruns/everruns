// Durable system schedules the server bootstraps on every start.
//
// Decision: each control-plane job is a declarative `ScheduleSpec`, kept in
// shape by `everruns_durable::ensure_schedule`, so adding a job is one spec,
// not another copy of find-compare-update-create. The durable scheduler then
// fires each schedule once per cluster per trigger.
//
// Both jobs here run on workers (`TaskWorkerConfig` claims their activity
// types), every 60 seconds:
// - `leased-resource-cleanup`: generic cleanup for leased provider resources,
//   so any subsystem that registers leased resources joins the same durable
//   cleanup and observability surface.
// - `session-task-reaper`: fails session tasks whose worker heartbeat went
//   stale; its input carries the retention TTL from env (EVE-580).

use anyhow::Result;
use everruns_durable::{Cadence, ScheduleSpec, WorkflowEventStore, ensure_schedule};
use std::sync::Arc;

pub const LEASED_RESOURCE_CLEANUP_SCHEDULE: &str = "leased-resource-cleanup";
pub const LEASED_RESOURCE_CLEANUP_ACTIVITY: &str = "leased_resource_cleanup";
pub const SESSION_TASK_REAPER_SCHEDULE: &str = "session-task-reaper";
pub const SESSION_TASK_REAPER_ACTIVITY: &str = "session_task_reaper";
/// Every 60 seconds, on the minute.
const EVERY_MINUTE: &str = "0 * * * * * *";

pub fn leased_resource_cleanup_spec() -> Result<ScheduleSpec> {
    let input = serde_json::to_value(
        everruns_worker::leased_resource_cleanup::LeasedResourceCleanupInput::default(),
    )?;
    Ok(ScheduleSpec::activity(
        LEASED_RESOURCE_CLEANUP_SCHEDULE,
        LEASED_RESOURCE_CLEANUP_ACTIVITY,
        Cadence::Cron(EVERY_MINUTE.to_string()),
        input,
    )
    .with_description(
        "Generic cleanup for leased provider resources such as Daytona sandboxes and Browserless sessions.",
    ))
}

pub fn session_task_reaper_spec() -> Result<ScheduleSpec> {
    let input = serde_json::to_value(
        everruns_worker::session_task_reaper::SessionTaskReaperInput::from_env(),
    )?;
    Ok(ScheduleSpec::activity(
        SESSION_TASK_REAPER_SCHEDULE,
        SESSION_TASK_REAPER_ACTIVITY,
        Cadence::Cron(EVERY_MINUTE.to_string()),
        input,
    )
    .with_description(
        "Fails session tasks whose worker heartbeat has gone stale (orphan reconciler).",
    ))
}

/// Ensure every worker-run system schedule. A failure is logged per schedule
/// and does not stop the others.
pub async fn ensure_worker_schedules(store: &Arc<dyn WorkflowEventStore + Send + Sync>) {
    for spec in [leased_resource_cleanup_spec(), session_task_reaper_spec()] {
        let result = match spec {
            Ok(spec) => ensure_schedule(store.as_ref(), &spec)
                .await
                .map(|_| ())
                .map_err(anyhow::Error::from),
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            tracing::error!(%error, "Failed to bootstrap durable system schedule");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_durable::{
        InMemoryWorkflowEventStore, Pagination, ScheduleFilter, ScheduleRow, ScheduleTargetType,
        Schedules,
    };

    async fn schedules(store: &InMemoryWorkflowEventStore) -> Vec<ScheduleRow> {
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
    }

    #[tokio::test]
    async fn bootstraps_both_worker_schedules_once() {
        let store = Arc::new(InMemoryWorkflowEventStore::new());
        let dyn_store: Arc<dyn WorkflowEventStore + Send + Sync> = store.clone();

        ensure_worker_schedules(&dyn_store).await;
        ensure_worker_schedules(&dyn_store).await;

        let rows = schedules(&store).await;
        assert_eq!(rows.len(), 2, "idempotent: {rows:?}");
        for (name, activity) in [
            (
                LEASED_RESOURCE_CLEANUP_SCHEDULE,
                LEASED_RESOURCE_CLEANUP_ACTIVITY,
            ),
            (SESSION_TASK_REAPER_SCHEDULE, SESSION_TASK_REAPER_ACTIVITY),
        ] {
            let row = rows.iter().find(|r| r.name == name).expect(name);
            assert_eq!(row.cron_expression, EVERY_MINUTE);
            assert_eq!(row.timezone, "UTC");
            assert_eq!(row.target_type, ScheduleTargetType::Activity);
            assert_eq!(row.target_name, activity);
            assert!(row.enabled);
            assert_eq!(row.max_concurrent, Some(1));
            assert!(!row.catch_up_missed);
            assert_eq!(row.max_catch_up, Some(1));
            assert!(row.next_trigger_at.is_some());
        }
        let cleanup = rows
            .iter()
            .find(|r| r.name == LEASED_RESOURCE_CLEANUP_SCHEDULE)
            .unwrap();
        assert_eq!(
            cleanup.target_input,
            serde_json::to_value(
                everruns_worker::leased_resource_cleanup::LeasedResourceCleanupInput::default()
            )
            .unwrap()
        );
    }

    #[test]
    fn worker_claims_both_activity_types() {
        let types = everruns_worker::TaskWorkerConfig::default().activity_types;
        assert!(types.iter().any(|t| t == LEASED_RESOURCE_CLEANUP_ACTIVITY));
        assert!(types.iter().any(|t| t == SESSION_TASK_REAPER_ACTIVITY));
    }
}
