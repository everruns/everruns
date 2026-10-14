//! Ownership and cancellation fencing for detached background attempts.

use super::*;
use std::collections::HashSet;

pub(super) const BACKGROUND_OWNER_PREFIX: &str = "background-tool:";

// PID alone cannot distinguish this process from a previous run whose PID was reused.
static BACKGROUND_OWNER: OnceLock<String> = OnceLock::new();
static ACTIVE_BACKGROUND_TASKS: OnceLock<Mutex<HashSet<BackgroundTaskKey>>> = OnceLock::new();
type BackgroundTaskKey = (SessionId, String, i32);

pub(super) fn background_owner() -> &'static str {
    BACKGROUND_OWNER.get_or_init(|| {
        format!(
            "{BACKGROUND_OWNER_PREFIX}{}:{}",
            std::process::id(),
            uuid::Uuid::now_v7().simple()
        )
    })
}

pub(super) fn active_background_tasks() -> &'static Mutex<HashSet<BackgroundTaskKey>> {
    ACTIVE_BACKGROUND_TASKS.get_or_init(|| Mutex::new(HashSet::new()))
}

/// The spawned future owns this guard, including before its first poll.
/// Completion, cooperative cancellation, panic, and abort all drop it.
pub(super) struct BackgroundTaskRegistration {
    key: BackgroundTaskKey,
}

impl BackgroundTaskRegistration {
    pub(super) fn register(task: &SessionTask) -> Result<Self> {
        let key = (task.session_id, task.id.clone(), task.attempt);
        let mut active = active_background_tasks()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !active.insert(key.clone()) {
            return Err(AgentLoopError::tool(
                "background task attempt is already active",
            ));
        }
        Ok(Self { key })
    }
}

impl Drop for BackgroundTaskRegistration {
    fn drop(&mut self) {
        active_background_tasks()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.key);
    }
}

pub(crate) fn background_task_ownership(task: &SessionTask) -> TaskOwnership {
    if task.kind != everruns_core::session_task::TASK_KIND_BACKGROUND_TOOL
        || task.worker_id.as_deref() != Some(background_owner())
    {
        return TaskOwnership::Unknown;
    }
    let active = active_background_tasks()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if active.contains(&(task.session_id, task.id.clone(), task.attempt)) {
        TaskOwnership::Alive
    } else {
        TaskOwnership::Lost
    }
}

/// Only local runtimes may probe this PID. Remote workers cannot interpret it.
pub(crate) fn background_task_owner_process(task: &SessionTask) -> Option<u32> {
    if task.kind != everruns_core::session_task::TASK_KIND_BACKGROUND_TOOL {
        return None;
    }
    let marker = task
        .worker_id
        .as_deref()?
        .strip_prefix(BACKGROUND_OWNER_PREFIX)?;
    let (pid, token) = marker.split_once(':')?;
    let process_id: u32 = pid.parse().ok()?;
    let instance = uuid::Uuid::parse_str(token).ok()?;
    // Accept only the canonical marker emitted here, not arbitrary worker IDs.
    if process_id == 0
        || process_id > i32::MAX as u32
        || process_id.to_string() != pid
        || instance.is_nil()
        || instance.simple().to_string() != token
    {
        return None;
    }
    Some(process_id)
}

pub(super) async fn claim_background_task(
    registry: &dyn everruns_core::session_task::SessionTaskRegistry,
    task: &SessionTask,
) -> Result<BackgroundTaskRegistration> {
    // THREAT[TM-DURABLE-001]: Persist ownership only after registering this exact attempt.
    // Recovery must not see our persisted marker without a live registration.
    let registration = BackgroundTaskRegistration::register(task)?;
    let claimed = registry
        .update(
            task.session_id,
            &task.id,
            everruns_core::session_task::SessionTaskUpdate {
                worker_id: Some(background_owner().to_owned()),
                heartbeat_at: Some(chrono::Utc::now()),
                expected_attempt: Some(task.attempt),
                expected_recovery: Some(
                    everruns_core::session_task::SessionTaskRecoveryGuard::from_task(task),
                ),
                ..Default::default()
            },
        )
        .await?;
    match claimed {
        Some(claimed)
            if claimed.attempt == task.attempt
                && claimed.worker_id.as_deref() == Some(background_owner())
                && !claimed.state.is_terminal() =>
        {
            Ok(registration)
        }
        _ => {
            // Cancellation may win between creation and claim. We have not polled
            // any tool future, so only this unchanged, unowned attempt can be
            // acknowledged as canceled. A new owner or attempt remains untouched.
            if task.worker_id.is_none()
                && let Some(current) = registry.get(task.session_id, &task.id).await?
                && current.attempt == task.attempt
                && current.state == task.state
                && current.worker_id == task.worker_id
                && current.heartbeat_at == task.heartbeat_at
                && current.cancel_requested_at.is_some()
            {
                everruns_core::session_task::recover_orphaned_task(
                    registry,
                    &current,
                    TaskOwnership::Lost,
                )
                .await?;
            }
            Err(AgentLoopError::tool(
                "background task ownership claim was superseded",
            ))
        }
    }
}

pub(super) async fn watch_background_task(
    registry: &dyn everruns_core::session_task::SessionTaskRegistry,
    session_id: SessionId,
    task_id: &str,
    attempt: i32,
) {
    loop {
        let Ok(Some(task)) = registry.get(session_id, task_id).await else {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            continue;
        };
        if task.attempt != attempt
            || task.state.is_terminal()
            || task.cancel_requested_at.is_some()
            || background_task_ownership(&task) != TaskOwnership::Alive
        {
            break;
        }
        // A concurrent owner change or recovery cannot be overwritten by this heartbeat.
        let _ = registry
            .update(
                session_id,
                task_id,
                everruns_core::session_task::SessionTaskUpdate {
                    worker_id: Some(background_owner().to_owned()),
                    heartbeat_at: Some(chrono::Utc::now()),
                    expected_attempt: Some(attempt),
                    expected_recovery: Some(
                        everruns_core::session_task::SessionTaskRecoveryGuard::from_task(&task),
                    ),
                    ..Default::default()
                },
            )
            .await;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

pub(super) async fn run_owned_background_task<F>(
    registry: &dyn everruns_core::session_task::SessionTaskRegistry,
    session_id: SessionId,
    task_id: &str,
    attempt: i32,
    tool_future: F,
) -> std::result::Result<BackgroundOutcome, ToolExecutionResult>
where
    F: std::future::Future<Output = std::result::Result<BackgroundOutcome, ToolExecutionResult>>,
{
    // Check cancellation before polling a tool that may immediately cause effects.
    let task = registry
        .get(session_id, task_id)
        .await
        .map_err(ToolExecutionResult::internal_error)?;
    if task.is_none_or(|task| {
        task.attempt != attempt
            || task.state.is_terminal()
            || task.cancel_requested_at.is_some()
            || background_task_ownership(&task) != TaskOwnership::Alive
    }) {
        return Err(ToolExecutionResult::tool_error(BACKGROUND_CANCEL_SENTINEL));
    }
    tokio::select! {
        biased;
        () = watch_background_task(registry, session_id, task_id, attempt) =>
            Err(ToolExecutionResult::tool_error(BACKGROUND_CANCEL_SENTINEL)),
        result = tool_future => result,
    }
}
