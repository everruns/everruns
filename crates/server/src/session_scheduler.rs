// Session schedule poller
//
// Background loop that claims due session schedules and triggers turns.
// Each triggered schedule injects a user message into the session with
// metadata.source = "schedule", then starts a turn workflow.
//
// Monitor task integration: when a schedule fires we find the linked monitor
// task (matched by spec["schedule_id"]) and:
//   - record an outbound message on its thread ("Monitor fired at …");
//   - for one-shot schedules (cron_expression is None), transition the monitor
//     to Succeeded ("Scheduled monitor completed").
// Errors in this path are best-effort and never fail the fire loop.
//
// Orphan reconciliation: on each poll we throttle a sweep (at most once per
// ~60 s) that cancels running monitor tasks whose linked schedule is no longer
// active. This closes the gap where the schedule is canceled/deleted directly
// (not via cancel_task) so the monitor would otherwise stay `running` forever.

use crate::domains::session_schedules::SessionScheduleService;
use crate::execution_metadata;
use crate::services::EventService;
use crate::storage::{DbSessionTaskRegistry, StorageBackend};
use chrono::Utc;
use everruns_core::events::{EventContext, EventRequest, InputMessageData};
use everruns_core::session_task::{
    NewTaskMessage, SessionTaskFilter, SessionTaskRegistry, SessionTaskState, SessionTaskUpdate,
    TASK_KIND_MONITOR,
};
use everruns_core::typed_id::{MessageId, SessionId};
use everruns_core::{ContentPart, Message, MessageRole, TextContentPart};
use everruns_worker::AgentRunner;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// How often the orphaned-monitor reconciliation sweep runs regardless of the
/// poll interval. The poll interval is 15 s; sweeping every ~60 s is achieved
/// by running the sweep only when the accumulated wall-clock time since the
/// last sweep exceeds this threshold.
const ORPHAN_SWEEP_INTERVAL: Duration = Duration::from_secs(60);

/// Maximum monitor tasks to cancel per sweep pass (bounds latency per cycle).
const ORPHAN_SWEEP_LIMIT: i64 = 50;

/// Spawn the session schedule poller as a background task.
///
/// Polls every `poll_interval` for due schedules, injects messages, and
/// triggers turns. Safe for concurrent execution via `FOR UPDATE SKIP LOCKED`.
pub fn spawn_session_scheduler(
    db: Arc<StorageBackend>,
    schedule_service: Arc<SessionScheduleService>,
    event_service: Arc<EventService>,
    runner: Arc<dyn AgentRunner>,
    poll_interval: Duration,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(poll_interval);
        // Skip the immediate first tick — let the server finish starting.
        interval.tick().await;

        tracing::info!(
            poll_interval_secs = poll_interval.as_secs(),
            "Started session schedule poller"
        );

        // Track when we last ran the orphaned-monitor sweep so we can
        // throttle it independently of the (faster) poll interval.
        let mut last_sweep: Option<std::time::Instant> = None;

        loop {
            interval.tick().await;
            if let Err(e) = poll_and_trigger(&db, &schedule_service, &event_service, &runner).await
            {
                tracing::error!(error = %e, "Session schedule poll failed");
            }

            // Orphan sweep: run at most once per ORPHAN_SWEEP_INTERVAL.
            let should_sweep = last_sweep
                .map(|t| t.elapsed() >= ORPHAN_SWEEP_INTERVAL)
                .unwrap_or(true);
            if should_sweep {
                let task_registry = DbSessionTaskRegistry::new(db.clone())
                    .with_event_emitter(event_service.clone());
                reconcile_orphaned_monitors(&db, &task_registry).await;
                last_sweep = Some(std::time::Instant::now());
            }
        }
    });
}

/// One iteration of the poll loop.
async fn poll_and_trigger(
    db: &Arc<StorageBackend>,
    schedule_service: &Arc<SessionScheduleService>,
    event_service: &Arc<EventService>,
    runner: &Arc<dyn AgentRunner>,
) -> anyhow::Result<()> {
    // Build the task registry once per poll iteration; reused for all fired schedules.
    // EventService implements EventEmitter so task snapshot events piggyback on the
    // same delivery path the worker uses.
    let task_registry =
        DbSessionTaskRegistry::new(db.clone()).with_event_emitter(event_service.clone());

    let due = db.claim_due_session_schedules(10).await?;
    if due.is_empty() {
        return Ok(());
    }

    tracing::info!(count = due.len(), "Claimed due session schedules");

    for row in due {
        let schedule_id = row.id;
        let session_id = row.session_id;
        let org_id = row.org_id;
        let description = row.description.clone();

        // Mark triggered (updates next_trigger, disables one-shot, etc.)
        let is_one_shot = row.cron_expression.is_none();
        if let Err(e) = schedule_service.mark_triggered(org_id, schedule_id).await {
            tracing::error!(
                schedule_id = %schedule_id,
                error = %e,
                "Failed to mark schedule as triggered"
            );
            continue;
        }

        // Update linked monitor tasks (best-effort; never fail the fire loop).
        fire_monitor_tasks(&task_registry, session_id, schedule_id, is_one_shot).await;

        // Look up session to get harness_id and agent_id for the runner.
        let session = match db.get_session(org_id, session_id).await {
            Ok(Some(s)) => s,
            Ok(None) => {
                tracing::warn!(
                    schedule_id = %schedule_id,
                    session_id = %session_id,
                    "Session not found for schedule, skipping"
                );
                continue;
            }
            Err(e) => {
                tracing::error!(
                    schedule_id = %schedule_id,
                    session_id = %session_id,
                    error = %e,
                    "Failed to fetch session for schedule"
                );
                continue;
            }
        };

        let harness_id = match session.harness_id {
            Some(h) => h,
            None => {
                tracing::warn!(
                    schedule_id = %schedule_id,
                    session_id = %session_id,
                    "Session has no harness_id, skipping"
                );
                continue;
            }
        };

        // Build injected message with metadata.source = "schedule"
        let message_id = Uuid::now_v7();
        let message_id_typed = MessageId::from_uuid(message_id);
        let now = Utc::now();

        let mut metadata = HashMap::new();
        metadata.insert(
            "source".to_string(),
            serde_json::Value::String("schedule".to_string()),
        );
        metadata.insert(
            "schedule_id".to_string(),
            serde_json::Value::String(schedule_id.to_string()),
        );

        let core_message = Message {
            id: message_id_typed,
            role: MessageRole::User,
            content: vec![ContentPart::Text(TextContentPart::new(description))],
            phase: None,
            thinking: None,
            thinking_signature: None,
            controls: None,
            metadata: Some(metadata),
            external_actor: None,
            created_at: now,
        };

        // Emit the input.message event
        let session_id_typed: SessionId = session_id;
        if let Err(e) = event_service
            .emit(
                EventRequest::new(
                    session_id_typed,
                    EventContext::empty(),
                    InputMessageData::new(core_message),
                )
                .with_metadata(execution_metadata::scheduled_run_metadata(
                    schedule_id,
                    session.owner_principal_id,
                    session.agent_identity_id,
                )),
            )
            .await
        {
            tracing::error!(
                schedule_id = %schedule_id,
                session_id = %session_id,
                error = %e,
                "Failed to emit schedule message event"
            );
            continue;
        }

        // Trigger the turn workflow
        let runner = runner.clone();
        let agent_id = session.agent_id;
        tokio::spawn(async move {
            if let Err(e) = runner
                .start_run(
                    org_id,
                    session_id_typed,
                    harness_id,
                    agent_id,
                    message_id_typed,
                    None,
                )
                .await
            {
                tracing::error!(
                    schedule_id = %schedule_id,
                    session_id = %session_id,
                    error = %e,
                    "Failed to start turn for scheduled message"
                );
            } else {
                tracing::info!(
                    schedule_id = %schedule_id,
                    session_id = %session_id,
                    "Schedule-triggered turn started"
                );
            }
        });
    }

    Ok(())
}

/// Record a fire event on linked monitor tasks and, for one-shot schedules,
/// transition the monitor to `Succeeded`.
///
/// Best-effort: errors are logged; the caller never sees them.
async fn fire_monitor_tasks(
    registry: &DbSessionTaskRegistry,
    session_id: SessionId,
    schedule_id: everruns_core::typed_id::ScheduleId,
    is_one_shot: bool,
) {
    let schedule_id_str = schedule_id.to_string();
    let now = Utc::now();

    // List monitor tasks for this session.
    let tasks = match registry
        .list(
            session_id,
            // Active monitors are `running`; terminal ones never re-fire, so
            // filter at the DB to avoid scanning history as it grows.
            Some(&SessionTaskFilter {
                kind: Some(TASK_KIND_MONITOR.to_string()),
                state: Some(SessionTaskState::Running),
            }),
        )
        .await
    {
        Ok(tasks) => tasks,
        Err(e) => {
            tracing::warn!(
                session_id = %session_id,
                schedule_id = %schedule_id_str,
                error = %e,
                "fire_monitor_tasks: failed to list monitor tasks"
            );
            return;
        }
    };

    // Filter to the task(s) whose spec["schedule_id"] matches this schedule.
    // (The list above already restricts to running monitors.)
    let matched: Vec<_> = tasks
        .into_iter()
        .filter(|t| {
            t.spec
                .get("schedule_id")
                .and_then(|v| v.as_str())
                .is_some_and(|id| id == schedule_id_str)
        })
        .collect();

    for task in matched {
        // Record an outbound message so the monitor's thread shows fire history.
        let msg_text = format!("Monitor fired at {}.", now.to_rfc3339());
        if let Err(e) = registry
            .record_message(
                session_id,
                &task.id,
                NewTaskMessage::outbound_text(msg_text),
            )
            .await
        {
            tracing::warn!(
                task_id = %task.id,
                error = %e,
                "fire_monitor_tasks: record_message failed"
            );
        }

        // One-shot: the schedule has been disabled; the monitor is done.
        if is_one_shot
            && let Err(e) = registry
                .update(
                    session_id,
                    &task.id,
                    SessionTaskUpdate {
                        state: Some(SessionTaskState::Succeeded),
                        summary: Some("Scheduled monitor completed".into()),
                        ..Default::default()
                    },
                )
                .await
        {
            tracing::warn!(
                task_id = %task.id,
                error = %e,
                "fire_monitor_tasks: failed to transition one-shot monitor to Succeeded"
            );
        }
        // Recurring monitors stay Running until cancel_task is called.
    }
}

/// Cancel running monitor tasks whose linked schedule is inactive.
///
/// This reconciles the gap where a schedule is canceled/deleted directly
/// (not through cancel_task / the API cancel endpoint), leaving the monitor
/// task in `running` forever. The sweep runs at ~60 s cadence.
///
/// Best-effort: errors per task are logged and never abort the sweep.
pub(crate) async fn reconcile_orphaned_monitors(
    db: &Arc<StorageBackend>,
    task_registry: &DbSessionTaskRegistry,
) {
    let candidates = match db
        .list_monitor_tasks_with_inactive_schedules(ORPHAN_SWEEP_LIMIT)
        .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "reconcile_orphaned_monitors: query failed");
            return;
        }
    };

    for (session_id, task_id, schedule_id) in candidates {
        match task_registry
            .update(
                session_id,
                &task_id,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Canceled),
                    state_detail: Some("schedule canceled".to_string()),
                    ..Default::default()
                },
            )
            .await
        {
            Ok(Some(task)) if task.state == SessionTaskState::Canceled => {
                tracing::info!(
                    session_id = %session_id,
                    task_id = %task_id,
                    schedule_id = %schedule_id,
                    "reconcile_orphaned_monitors: canceled monitor with inactive schedule"
                );
            }
            Ok(_) => {
                // Already terminal (race is harmless) or not found.
            }
            Err(e) => {
                tracing::warn!(
                    session_id = %session_id,
                    task_id = %task_id,
                    schedule_id = %schedule_id,
                    error = %e,
                    "reconcile_orphaned_monitors: failed to cancel task"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::models::{CreateSessionScheduleRow, UpdateSessionScheduleRow};
    use everruns_core::session_task::{CreateSessionTask, TaskLinks, TaskWakePolicy};
    use everruns_core::typed_id::PrincipalId;
    use everruns_core::{DEFAULT_ORG_ID, ScheduleId, SessionId};

    fn make_db() -> Arc<StorageBackend> {
        Arc::new(StorageBackend::in_memory())
    }

    fn make_registry(db: Arc<StorageBackend>) -> DbSessionTaskRegistry {
        DbSessionTaskRegistry::new(db)
    }

    async fn create_monitor_task(
        db: &Arc<StorageBackend>,
        session_id: SessionId,
        schedule_id: ScheduleId,
    ) -> everruns_core::session_task::SessionTask {
        let registry = make_registry(db.clone());
        registry
            .create(CreateSessionTask {
                session_id,
                id: None,
                kind: TASK_KIND_MONITOR.to_string(),
                display_name: "Test monitor".to_string(),
                spec: serde_json::json!({ "schedule_id": schedule_id.to_string() }),
                state: SessionTaskState::Running,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap()
    }

    async fn create_schedule(db: &Arc<StorageBackend>, session_id: SessionId) -> ScheduleId {
        let row = db
            .create_session_schedule(CreateSessionScheduleRow {
                org_id: DEFAULT_ORG_ID,
                session_id,
                owner_principal_id: PrincipalId::new(),
                resolved_owner_user_id: None,
                description: "Test schedule".to_string(),
                cron_expression: Some("0 * * * *".to_string()),
                scheduled_at: None,
                timezone: "UTC".to_string(),
                next_trigger_at: None,
            })
            .await
            .unwrap();
        row.id
    }

    // -------------------------------------------------------------------------
    // Storage-level tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn query_returns_monitor_linked_to_canceled_schedule() {
        let db = make_db();
        let session_id = SessionId::new();

        let schedule_id = create_schedule(&db, session_id).await;
        let task = create_monitor_task(&db, session_id, schedule_id).await;

        // Schedule is still active — should NOT appear.
        let before = db
            .list_monitor_tasks_with_inactive_schedules(50)
            .await
            .unwrap();
        assert!(
            before.is_empty(),
            "active schedule should not appear in results"
        );

        // Cancel the schedule directly (enabled → false).
        db.update_session_schedule(
            DEFAULT_ORG_ID,
            schedule_id,
            UpdateSessionScheduleRow {
                enabled: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let after = db
            .list_monitor_tasks_with_inactive_schedules(50)
            .await
            .unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].0, session_id);
        assert_eq!(after[0].1, task.id);
        assert_eq!(after[0].2, schedule_id.to_string());
    }

    #[tokio::test]
    async fn query_returns_monitor_with_deleted_schedule() {
        let db = make_db();
        let session_id = SessionId::new();

        let schedule_id = create_schedule(&db, session_id).await;
        let task = create_monitor_task(&db, session_id, schedule_id).await;

        // Delete the schedule row entirely.
        db.delete_session_schedule(DEFAULT_ORG_ID, schedule_id)
            .await
            .unwrap();

        let results = db
            .list_monitor_tasks_with_inactive_schedules(50)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1, task.id);
    }

    #[tokio::test]
    async fn query_does_not_return_monitor_with_active_schedule() {
        let db = make_db();
        let session_id = SessionId::new();

        let schedule_id = create_schedule(&db, session_id).await;
        create_monitor_task(&db, session_id, schedule_id).await;

        // Schedule is active (enabled=true) — must not appear.
        let results = db
            .list_monitor_tasks_with_inactive_schedules(50)
            .await
            .unwrap();
        assert!(
            results.is_empty(),
            "active schedule monitor must be excluded"
        );
    }

    #[tokio::test]
    async fn query_does_not_return_monitor_without_schedule_id() {
        let db = make_db();
        let session_id = SessionId::new();

        // Monitor task with no schedule_id in spec.
        let registry = make_registry(db.clone());
        registry
            .create(CreateSessionTask {
                session_id,
                id: None,
                kind: TASK_KIND_MONITOR.to_string(),
                display_name: "No schedule".to_string(),
                spec: serde_json::json!({}), // no schedule_id
                state: SessionTaskState::Running,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();

        let results = db
            .list_monitor_tasks_with_inactive_schedules(50)
            .await
            .unwrap();
        assert!(
            results.is_empty(),
            "monitor without schedule_id must be excluded"
        );
    }

    // -------------------------------------------------------------------------
    // Sweep-level tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn sweep_cancels_monitor_with_inactive_schedule() {
        let db = make_db();
        let session_id = SessionId::new();
        let registry = make_registry(db.clone());

        let schedule_id = create_schedule(&db, session_id).await;
        let task = create_monitor_task(&db, session_id, schedule_id).await;

        // Disable the schedule directly (not via cancel_task).
        db.update_session_schedule(
            DEFAULT_ORG_ID,
            schedule_id,
            UpdateSessionScheduleRow {
                enabled: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        reconcile_orphaned_monitors(&db, &registry).await;

        let updated = registry.get(session_id, &task.id).await.unwrap().unwrap();
        assert_eq!(
            updated.state,
            SessionTaskState::Canceled,
            "sweep must cancel the orphaned monitor"
        );
        assert_eq!(
            updated.state_detail.as_deref(),
            Some("schedule canceled"),
            "state_detail must identify the reason"
        );
    }

    #[tokio::test]
    async fn sweep_leaves_monitor_with_active_schedule_untouched() {
        let db = make_db();
        let session_id = SessionId::new();
        let registry = make_registry(db.clone());

        let schedule_id = create_schedule(&db, session_id).await;
        let task = create_monitor_task(&db, session_id, schedule_id).await;

        // Schedule remains active.
        reconcile_orphaned_monitors(&db, &registry).await;

        let unchanged = registry.get(session_id, &task.id).await.unwrap().unwrap();
        assert_eq!(
            unchanged.state,
            SessionTaskState::Running,
            "active-schedule monitor must not be touched by sweep"
        );
    }
}
