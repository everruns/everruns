// Session schedule poller
//
// Background loop that claims due session schedules and triggers turns.
// Each triggered schedule injects a user message into the session with
// metadata.source = "schedule", then starts a turn workflow.
//
// Monitor task integration: when a schedule fires we find the linked monitor
// task (matched by spec["schedule_id"]) and:
//   - if the monitor has a probe (spec["tool"] + spec["arguments"]), execute
//     the tool directly and record the result on the monitor's thread.
//     Probe monitors skip the agent turn — the probe runs autonomously.
//   - otherwise record a plain "Monitor fired at …" outbound message.
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
use everruns_core::tool_context::ToolContext;
use everruns_core::tools::ToolRegistry;
use everruns_core::{ContentPart, Message, MessageRole, TextContentPart};
use everruns_provider::typed_id::{MessageId, SessionId};
use everruns_worker::AgentRunner;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use uuid::Uuid;

/// How often the orphaned-monitor reconciliation sweep runs regardless of the
/// poll interval. The poll interval is 15 s; sweeping every ~60 s is achieved
/// by running the sweep only when the accumulated wall-clock time since the
/// last sweep exceeds this threshold.
const ORPHAN_SWEEP_INTERVAL: Duration = Duration::from_secs(60);

/// Maximum monitor tasks to cancel per sweep pass (bounds latency per cycle).
const ORPHAN_SWEEP_LIMIT: i64 = 50;

/// Build the deliberately narrow tool registry used by autonomous monitor
/// probes. Portable context-free tools are composed by the product rather than
/// being hard-coded into the core registry.
pub(crate) fn monitor_probe_tool_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::with_monitor_probe_defaults();
    everruns_builtins::register_monitor_tools(&mut registry);
    registry
}

/// Spawn the session schedule poller as a background task.
///
/// Polls every `poll_interval` for due schedules, injects messages, and
/// triggers turns. Safe for concurrent execution via `FOR UPDATE SKIP LOCKED`.
///
/// When `probe_tool_registry` is provided, monitor tasks that carry a probe
/// spec (`spec["tool"]` + `spec["arguments"]`) execute the probe directly
/// instead of delegating to an agent turn.
pub fn spawn_session_scheduler(
    db: Arc<StorageBackend>,
    schedule_service: Arc<SessionScheduleService>,
    event_service: Arc<EventService>,
    runner: Arc<dyn AgentRunner>,
    probe_tool_registry: Option<Arc<ToolRegistry>>,
    poll_interval: Duration,
) -> JoinHandle<()> {
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
            if let Err(e) = poll_and_trigger(
                &db,
                &schedule_service,
                &event_service,
                &runner,
                probe_tool_registry.as_deref(),
            )
            .await
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
    })
}

/// One iteration of the poll loop.
async fn poll_and_trigger(
    db: &Arc<StorageBackend>,
    schedule_service: &Arc<SessionScheduleService>,
    event_service: &Arc<EventService>,
    runner: &Arc<dyn AgentRunner>,
    probe_tool_registry: Option<&ToolRegistry>,
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
        // Returns true when a probe ran — in that case the monitor is autonomous
        // and we skip the agent-mediated session turn.
        let probe_ran = fire_monitor_tasks(
            &task_registry,
            session_id,
            schedule_id,
            is_one_shot,
            probe_tool_registry,
        )
        .await;
        if probe_ran {
            tracing::debug!(
                schedule_id = %schedule_id,
                session_id = %session_id,
                "Monitor probe ran; skipping agent turn"
            );
            continue;
        }

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
/// When `tool_registry` is provided and a matched task has `spec["tool"]` +
/// `spec["arguments"]`, executes the probe tool directly and records the
/// result (success or error) as the outbound message on the monitor's thread.
///
/// Returns `true` when at least one probe ran so the caller can skip the
/// agent-mediated session turn — probes are autonomous.
///
/// Best-effort: errors are logged; the caller never sees them.
async fn fire_monitor_tasks(
    registry: &DbSessionTaskRegistry,
    session_id: SessionId,
    schedule_id: everruns_provider::typed_id::ScheduleId,
    is_one_shot: bool,
    tool_registry: Option<&ToolRegistry>,
) -> bool {
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
            return false;
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

    let mut any_probe_ran = false;

    for task in matched {
        // Probe execution: if the task spec carries a tool + args and we have
        // a tool registry, run the probe directly instead of posting a plain
        // "Monitor fired" message.
        let probe_result = run_probe_for_task(&task, session_id, tool_registry).await;

        let outbound_text = match probe_result {
            ProbeOutcome::Ran { output } => {
                any_probe_ran = true;
                output
            }
            ProbeOutcome::Skipped => {
                // No probe configured or registry unavailable — fall back to
                // the legacy placeholder message so the monitor thread is
                // never silent.
                format!("Monitor fired at {}.", now.to_rfc3339())
            }
        };

        if let Err(e) = registry
            .record_message(
                session_id,
                &task.id,
                NewTaskMessage::outbound_text(outbound_text),
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

    any_probe_ran
}

enum ProbeOutcome {
    /// Probe executed; `output` is the formatted result text for the thread.
    Ran { output: String },
    /// No probe configured or tool registry not available; caller records the
    /// legacy "Monitor fired" placeholder instead.
    Skipped,
}

/// Attempt to run the probe tool stored in a monitor task's spec.
///
/// Returns `Ran` with the formatted result when the tool is found and runs
/// (whether it succeeds or returns an error — both produce an observation).
/// Returns `Skipped` when the spec has no tool, the registry lacks it, or
/// the registry itself is `None`.
async fn run_probe_for_task(
    task: &everruns_core::SessionTask,
    session_id: SessionId,
    tool_registry: Option<&ToolRegistry>,
) -> ProbeOutcome {
    let Some(registry) = tool_registry else {
        return ProbeOutcome::Skipped;
    };

    let tool_name = match task.spec.get("tool").and_then(|v| v.as_str()) {
        Some(name) if !name.is_empty() => name,
        _ => return ProbeOutcome::Skipped,
    };

    let tool_args = task
        .spec
        .get("arguments")
        .cloned()
        .unwrap_or(serde_json::Value::Object(Default::default()));

    let Some(tool) = registry.get(tool_name) else {
        tracing::debug!(
            task_id = %task.id,
            tool = %tool_name,
            "Monitor probe: tool not in probe registry, skipping"
        );
        return ProbeOutcome::Skipped;
    };

    let ctx = ToolContext::new(session_id);
    let result = tool.execute_with_context(tool_args, &ctx).await;

    let output = match result {
        everruns_core::ToolExecutionResult::Success(ref val) => {
            // Try the MCP `{"content":[{"text":"…"}]}` envelope first, then a
            // top-level string, then pretty-print the raw JSON so structured
            // tool outputs (e.g. `get_current_time` → `{"datetime":…}`) are
            // never silently discarded.
            let content = val
                .get("content")
                .and_then(|c| c.as_array())
                .and_then(|arr| arr.first())
                .and_then(|item| item.get("text"))
                .and_then(|t| t.as_str())
                .map(str::to_owned)
                .or_else(|| val.as_str().map(str::to_owned))
                .unwrap_or_else(|| {
                    serde_json::to_string_pretty(val)
                        .unwrap_or_else(|_| "(probe completed with no text output)".to_owned())
                });
            format!(
                "Probe `{tool_name}` succeeded at {}.\n\n{}",
                Utc::now().to_rfc3339(),
                content
            )
        }
        everruns_core::ToolExecutionResult::ToolError(ref msg) => {
            format!(
                "Probe `{tool_name}` failed at {}.\n\nError: {msg}",
                Utc::now().to_rfc3339()
            )
        }
        // InternalError and ConnectionRequired cannot produce a useful probe
        // observation — fall back so the caller records the legacy placeholder
        // and starts a normal scheduled agent turn.
        everruns_core::ToolExecutionResult::InternalError(_)
        | everruns_core::ToolExecutionResult::ConnectionRequired { .. } => {
            return ProbeOutcome::Skipped;
        }
        everruns_core::ToolExecutionResult::SuccessWithImages { ref result, .. } => {
            // Pretty-print the JSON result; note image count for context.
            let text = result.as_str().map(str::to_owned).unwrap_or_else(|| {
                serde_json::to_string_pretty(result)
                    .unwrap_or_else(|_| "(probe completed with image output)".to_owned())
            });
            format!(
                "Probe `{tool_name}` succeeded at {}.\n\n{}",
                Utc::now().to_rfc3339(),
                text
            )
        }
    };

    tracing::debug!(
        task_id = %task.id,
        tool = %tool_name,
        "Monitor probe ran"
    );

    ProbeOutcome::Ran { output }
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
    use everruns_core::DEFAULT_ORG_ID;
    use everruns_core::session_task::{
        CreateSessionTask, SessionTaskRegistry, TaskLinks, TaskMessagePart, TaskWakePolicy,
    };
    use everruns_provider::typed_id::{PrincipalId, ScheduleId, SessionId};

    fn make_db() -> Arc<StorageBackend> {
        Arc::new(StorageBackend::in_memory())
    }

    fn message_text(msg: &everruns_core::session_task::TaskMessage) -> String {
        msg.content
            .iter()
            .filter_map(|p| {
                if let TaskMessagePart::Text { text } = p {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("")
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
        create_schedule_with_cron(db, session_id, Some("0 * * * *".to_string())).await
    }

    async fn create_schedule_with_cron(
        db: &Arc<StorageBackend>,
        session_id: SessionId,
        cron_expression: Option<String>,
    ) -> ScheduleId {
        let row = db
            .create_session_schedule(CreateSessionScheduleRow {
                org_id: DEFAULT_ORG_ID,
                session_id,
                owner_principal_id: PrincipalId::new(),
                resolved_owner_user_id: None,
                description: "Test schedule".to_string(),
                cron_expression,
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
    async fn query_returns_disabled_never_triggered_one_shot_schedule() {
        let db = make_db();
        let session_id = SessionId::new();

        let schedule_id = create_schedule_with_cron(&db, session_id, None).await;
        let task = create_monitor_task(&db, session_id, schedule_id).await;

        // Direct cancellation disables a one-shot without trigger metadata; the
        // orphan sweep must still reconcile its linked monitor.
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

        let results = db
            .list_monitor_tasks_with_inactive_schedules(50)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, session_id);
        assert_eq!(results[0].1, task.id);
        assert_eq!(results[0].2, schedule_id.to_string());
    }

    #[tokio::test]
    async fn query_does_not_return_fired_disabled_one_shot_schedule() {
        let db = make_db();
        let session_id = SessionId::new();

        let schedule_id = create_schedule_with_cron(&db, session_id, None).await;
        create_monitor_task(&db, session_id, schedule_id).await;

        // One-shot firing disables the schedule and records trigger metadata
        // before the monitor is marked Succeeded. The orphan sweep must not
        // race that path and cancel it.
        db.update_session_schedule(
            DEFAULT_ORG_ID,
            schedule_id,
            UpdateSessionScheduleRow {
                enabled: Some(false),
                last_triggered_at: Some(Utc::now()),
                trigger_count_increment: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let results = db
            .list_monitor_tasks_with_inactive_schedules(50)
            .await
            .unwrap();
        assert!(
            results.is_empty(),
            "fired disabled one-shot schedule monitor must be excluded"
        );
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

    // -------------------------------------------------------------------------
    // Probe execution tests
    // -------------------------------------------------------------------------

    async fn create_probe_monitor_task(
        db: &Arc<StorageBackend>,
        session_id: SessionId,
        schedule_id: ScheduleId,
        tool_name: &str,
        tool_args: serde_json::Value,
    ) -> everruns_core::session_task::SessionTask {
        let registry = make_registry(db.clone());
        registry
            .create(CreateSessionTask {
                session_id,
                id: None,
                kind: TASK_KIND_MONITOR.to_string(),
                display_name: "Probe monitor".to_string(),
                spec: serde_json::json!({
                    "schedule_id": schedule_id.to_string(),
                    "tool": tool_name,
                    "arguments": tool_args,
                }),
                state: SessionTaskState::Running,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn monitor_probe_registry_excludes_network_and_filesystem_tools() {
        let registry = monitor_probe_tool_registry();

        assert!(
            registry.has("get_current_time"),
            "context-free probe tools remain available"
        );
        assert!(
            !registry.has("web_fetch"),
            "scheduled probes must not run network-capable tools without the worker/API ToolContext"
        );
        assert!(
            !registry.has("read_file"),
            "scheduled probes must not run filesystem tools without the worker/API ToolContext"
        );
    }

    #[tokio::test]
    async fn probe_monitor_records_tool_result_not_placeholder() {
        let db = make_db();
        let session_id = SessionId::new();
        let schedule_id = ScheduleId::new();

        let task = create_probe_monitor_task(
            &db,
            session_id,
            schedule_id,
            "get_current_time",
            serde_json::json!({}),
        )
        .await;

        let registry = make_registry(db.clone());
        let tool_registry = Arc::new(monitor_probe_tool_registry());

        let probe_ran = fire_monitor_tasks(
            &registry,
            session_id,
            schedule_id,
            false,
            Some(&tool_registry),
        )
        .await;

        assert!(
            probe_ran,
            "fire_monitor_tasks must return true when a probe ran"
        );

        // The outbound message must be the tool result, not the "Monitor fired" placeholder.
        let messages = registry
            .list_messages(session_id, &task.id, Some(10), None)
            .await
            .unwrap();
        assert_eq!(messages.len(), 1, "exactly one message must be recorded");
        let body = message_text(&messages[0]);
        assert!(
            body.contains("get_current_time"),
            "probe message must mention the tool name, got: {body}"
        );
        assert!(
            !body.contains("Monitor fired at"),
            "probe message must not be the legacy placeholder, got: {body}"
        );
        // Verify the actual tool payload is present: get_current_time returns
        // {"datetime": "…", "format": "iso8601", "timezone": "UTC"}.
        assert!(
            body.contains("datetime"),
            "probe message must contain the tool result payload, got: {body}"
        );
    }

    #[tokio::test]
    async fn plain_monitor_without_probe_records_placeholder() {
        let db = make_db();
        let session_id = SessionId::new();
        let schedule_id = ScheduleId::new();

        let task = create_monitor_task(&db, session_id, schedule_id).await;

        let registry = make_registry(db.clone());
        let tool_registry = Arc::new(monitor_probe_tool_registry());

        let probe_ran = fire_monitor_tasks(
            &registry,
            session_id,
            schedule_id,
            false,
            Some(&tool_registry),
        )
        .await;

        assert!(
            !probe_ran,
            "fire_monitor_tasks must return false for a plain (non-probe) monitor"
        );

        let messages = registry
            .list_messages(session_id, &task.id, Some(10), None)
            .await
            .unwrap();
        assert_eq!(messages.len(), 1);
        let body = message_text(&messages[0]);
        assert!(
            body.contains("Monitor fired at"),
            "plain monitor must get the legacy placeholder, got: {body}"
        );
    }

    #[tokio::test]
    async fn probe_monitor_without_registry_records_placeholder() {
        let db = make_db();
        let session_id = SessionId::new();
        let schedule_id = ScheduleId::new();

        let task = create_probe_monitor_task(
            &db,
            session_id,
            schedule_id,
            "get_current_time",
            serde_json::json!({}),
        )
        .await;

        let registry = make_registry(db.clone());

        // No probe registry supplied → must fall back to "Monitor fired" placeholder.
        let probe_ran = fire_monitor_tasks(&registry, session_id, schedule_id, false, None).await;

        assert!(!probe_ran, "no registry → probe_ran must be false");

        let messages = registry
            .list_messages(session_id, &task.id, Some(10), None)
            .await
            .unwrap();
        assert_eq!(messages.len(), 1);
        assert!(
            message_text(&messages[0]).contains("Monitor fired at"),
            "must fall back to placeholder when registry is absent"
        );
    }
}
