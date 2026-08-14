// Monitor task executor
//
// A monitor is a long-lived task (`running` until canceled) linked to a session
// schedule. It is created by `spawn_background` when a `schedule` argument is
// supplied, and its spec carries the `schedule_id` of the backing schedule.
//
// Canceling a monitor via `cancel_task` cancels the linked schedule (so it
// stops firing) and transitions the task to `Canceled`.
//
// Decision: the executor is stateless — no in-process handle is needed because
// the schedule poller fires externally and the task record is the durable link.

use std::sync::Arc;

use everruns_core::session_task::{
    SessionTaskState, SessionTaskUpdate, TASK_KIND_MONITOR, TaskExecutor, TaskExecutorPlugin,
};

/// Executor for `monitor` tasks. Lifecycle is driven by the schedule poller
/// (server-side); the executor's only active role is cooperative cancellation.
pub struct MonitorTaskExecutor;

#[async_trait::async_trait]
impl TaskExecutor for MonitorTaskExecutor {
    fn kind(&self) -> &str {
        TASK_KIND_MONITOR
    }

    /// Cancel the monitor: disable the linked schedule then mark the task Canceled.
    ///
    /// Both operations are best-effort. The task transitions to Canceled even if
    /// the schedule store is unavailable or the cancel call fails — leaving a
    /// dangling enabled schedule is less harmful than leaving the task stuck.
    async fn cancel(
        &self,
        task: &everruns_core::session_task::SessionTask,
        context: &everruns_core::tool_context::ToolContext,
    ) -> everruns_provider::error::Result<()> {
        // Attempt to cancel the linked schedule.
        if let Some(schedule_id_str) = task.spec.get("schedule_id").and_then(|v| v.as_str()) {
            match everruns_provider::typed_id::ScheduleId::parse(schedule_id_str) {
                Ok(schedule_id) => {
                    if let Some(ref store) = context.schedule_store
                        && let Err(e) = store.cancel_schedule(task.session_id, schedule_id).await
                    {
                        tracing::warn!(
                            task_id = %task.id,
                            schedule_id = %schedule_id_str,
                            error = %e,
                            "MonitorTaskExecutor: cancel_schedule failed (best-effort)"
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        task_id = %task.id,
                        raw = %schedule_id_str,
                        error = %e,
                        "MonitorTaskExecutor: could not parse schedule_id from spec"
                    );
                }
            }
        }

        // Transition the task to Canceled regardless of schedule outcome.
        if let Some(ref registry) = context.session_task_registry
            && let Err(e) = registry
                .update(
                    task.session_id,
                    &task.id,
                    SessionTaskUpdate {
                        state: Some(SessionTaskState::Canceled),
                        summary: Some("Monitor canceled".into()),
                        ..Default::default()
                    },
                )
                .await
        {
            tracing::warn!(
                task_id = %task.id,
                error = %e,
                "MonitorTaskExecutor: failed to update task to Canceled"
            );
        }

        Ok(())
    }
}

inventory::submit! {
    TaskExecutorPlugin {
        executor: || Arc::new(MonitorTaskExecutor),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::session_tasks::tests::InMemorySessionTaskRegistry;
    use everruns_core::session_schedule::SessionSchedule;
    use everruns_core::session_task::{
        CreateSessionTask, SessionTaskRegistry, SessionTaskState, TaskLinks, TaskWakePolicy,
    };
    use everruns_core::{session_services::SessionScheduleStore, tool_context::ToolContext};
    use everruns_provider::typed_id::{ScheduleId, SessionId};
    use std::sync::{Arc, Mutex};

    // -------------------------------------------------------------------------
    // Stub SessionScheduleStore
    // -------------------------------------------------------------------------

    #[derive(Default)]
    struct StubScheduleStore {
        canceled: Mutex<Vec<ScheduleId>>,
    }

    #[async_trait::async_trait]
    impl SessionScheduleStore for StubScheduleStore {
        async fn create_schedule(
            &self,
            session_id: SessionId,
            description: String,
            cron_expression: Option<String>,
            scheduled_at: Option<chrono::DateTime<chrono::Utc>>,
            timezone: String,
        ) -> everruns_provider::error::Result<SessionSchedule> {
            let _ = (
                session_id,
                description,
                cron_expression,
                scheduled_at,
                timezone,
            );
            unimplemented!("not needed by cancel test")
        }

        async fn cancel_schedule(
            &self,
            session_id: SessionId,
            schedule_id: ScheduleId,
        ) -> everruns_provider::error::Result<SessionSchedule> {
            self.canceled.lock().unwrap().push(schedule_id);
            // Return a minimal schedule so the call succeeds.
            Ok(SessionSchedule {
                id: schedule_id,
                session_id,
                owner_principal_id: everruns_provider::typed_id::PrincipalId::new(),
                resolved_owner_user_id: None,
                owner: None,
                effective_owner: None,
                description: String::new(),
                cron_expression: None,
                scheduled_at: None,
                timezone: "UTC".into(),
                enabled: false,
                schedule_type: everruns_core::session_schedule::ScheduleType::OneShot,
                next_trigger_at: None,
                last_triggered_at: None,
                trigger_count: 0,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
        }

        async fn count_active_schedules(
            &self,
            _session_id: SessionId,
        ) -> everruns_provider::error::Result<u32> {
            Ok(0)
        }

        async fn count_active_org_schedules(&self) -> everruns_provider::error::Result<u32> {
            Ok(0)
        }

        async fn list_schedules(
            &self,
            _session_id: SessionId,
        ) -> everruns_provider::error::Result<Vec<SessionSchedule>> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn cancel_reads_schedule_id_and_transitions_to_canceled() {
        let session_id = SessionId::new();
        let schedule_id = ScheduleId::new();
        let registry = Arc::new(InMemorySessionTaskRegistry::default());
        let schedule_store = Arc::new(StubScheduleStore::default());

        // Create a monitor task with a schedule_id in its spec.
        let task = registry
            .create(CreateSessionTask {
                session_id,
                id: None,
                kind: TASK_KIND_MONITOR.to_string(),
                display_name: "Test Monitor".to_string(),
                spec: serde_json::json!({
                    "schedule_id": schedule_id.to_string(),
                }),
                state: SessionTaskState::Running,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();

        let context = ToolContext::new(session_id)
            .with_session_task_registry(registry.clone())
            .with_schedule_store(schedule_store.clone());

        let executor = MonitorTaskExecutor;
        executor.cancel(&task, &context).await.unwrap();

        // cancel_schedule was called with the correct schedule_id.
        // Drop the guard before any await.
        {
            let canceled = schedule_store.canceled.lock().unwrap();
            assert_eq!(canceled.len(), 1);
            assert_eq!(canceled[0], schedule_id);
        }

        // Task is now Canceled.
        let updated = registry
            .get(session_id, &task.id)
            .await
            .unwrap()
            .expect("task should exist");
        assert_eq!(updated.state, SessionTaskState::Canceled);
        assert_eq!(updated.summary.as_deref(), Some("Monitor canceled"));
    }

    #[tokio::test]
    async fn cancel_succeeds_without_schedule_store() {
        let session_id = SessionId::new();
        let schedule_id = ScheduleId::new();
        let registry = Arc::new(InMemorySessionTaskRegistry::default());

        let task = registry
            .create(CreateSessionTask {
                session_id,
                id: None,
                kind: TASK_KIND_MONITOR.to_string(),
                display_name: "Test Monitor".to_string(),
                spec: serde_json::json!({ "schedule_id": schedule_id.to_string() }),
                state: SessionTaskState::Running,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();

        // No schedule_store — cancel should still transition the task.
        let context = ToolContext::new(session_id).with_session_task_registry(registry.clone());

        MonitorTaskExecutor.cancel(&task, &context).await.unwrap();

        let updated = registry
            .get(session_id, &task.id)
            .await
            .unwrap()
            .expect("task should exist");
        assert_eq!(updated.state, SessionTaskState::Canceled);
    }
}
