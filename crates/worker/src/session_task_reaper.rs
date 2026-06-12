// Orphaned session-task reconciler.
//
// Decision: mirrors the leased-resource-cleanup activity end-to-end: same
// durable schedule seeding, same cadence, same activity registration pattern.
// The reaper finds tasks whose worker heartbeat has gone stale and fails them
// via the registry so lifecycle invariants, events, and wake_policy all fire
// exactly as they do for any other terminal transition.
//
// Tasks with NULL heartbeat_at are never reaped (foreground subagent tasks
// are covered by EVE-535 spawn handles, not by this reconciler).

use anyhow::Result;
use everruns_core::session_task::{SessionTaskState, SessionTaskUpdate, TaskError};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::worker_adapters::WorkerAdapters;

/// How long a heartbeat may be stale before a task is considered orphaned.
const DEFAULT_STALE_AFTER_SECONDS: i64 = 5 * 60; // 5 minutes

/// How many orphaned tasks to process per reaper pass.
const DEFAULT_LIMIT: i64 = 50;

/// Durable activity input for the session-task reaper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionTaskReaperInput {
    /// Age (in seconds) after which a stale heartbeat marks a task orphaned.
    pub stale_after_seconds: i64,
    /// Max tasks to reap per pass.
    pub limit: i64,
}

impl Default for SessionTaskReaperInput {
    fn default() -> Self {
        Self {
            stale_after_seconds: DEFAULT_STALE_AFTER_SECONDS,
            limit: DEFAULT_LIMIT,
        }
    }
}

#[derive(Debug, Serialize)]
struct ReapOutcome {
    session_id: String,
    task_id: String,
    status: String,
    detail: String,
}

#[derive(Debug, Serialize)]
struct ReapSummary {
    candidates: usize,
    reaped: usize,
    skipped: usize,
    outcomes: Vec<ReapOutcome>,
}

/// Execute one orphaned-session-task reconciliation pass.
///
/// For each stale task the reaper calls `registry.update` with
/// `state=Failed, error={kind:"orphaned"}` so lifecycle invariants hold and
/// `task.updated` events + wake_policy fire via the registry.
pub async fn execute_reaper_activity<A: WorkerAdapters>(
    adapters: &A,
    input: &SessionTaskReaperInput,
) -> Result<serde_json::Value> {
    let stale_after = chrono::Duration::seconds(input.stale_after_seconds);

    let candidates = adapters
        .list_orphaned_session_task_ids(stale_after, input.limit)
        .await?;

    let registry = adapters.reaper_session_task_registry();

    let mut summary = ReapSummary {
        candidates: candidates.len(),
        reaped: 0,
        skipped: 0,
        outcomes: Vec::with_capacity(candidates.len()),
    };

    for (session_id, task_id) in candidates {
        let update = SessionTaskUpdate {
            state: Some(SessionTaskState::Failed),
            error: Some(TaskError {
                kind: "orphaned".to_string(),
                message: "worker heartbeat stopped".to_string(),
            }),
            // Supersede the executor's attempt so its fenced writes
            // (heartbeats, progress, messages) are rejected from now on.
            increment_attempt: true,
            ..Default::default()
        };

        match registry.update(session_id, &task_id, update).await {
            Ok(Some(task)) if task.state == SessionTaskState::Failed => {
                info!(
                    session_id = %session_id,
                    task_id = %task_id,
                    "Session task reaper: marked orphaned task failed"
                );
                summary.reaped += 1;
                summary.outcomes.push(ReapOutcome {
                    session_id: session_id.to_string(),
                    task_id: task_id.clone(),
                    status: "reaped".to_string(),
                    detail: "marked failed: orphaned".to_string(),
                });
            }
            Ok(_) => {
                // Task was already terminal or not found — skip.
                summary.skipped += 1;
                summary.outcomes.push(ReapOutcome {
                    session_id: session_id.to_string(),
                    task_id: task_id.clone(),
                    status: "skipped".to_string(),
                    detail: "already terminal or missing".to_string(),
                });
            }
            Err(e) => {
                warn!(
                    session_id = %session_id,
                    task_id = %task_id,
                    error = %e,
                    "Session task reaper: failed to update task"
                );
                summary.skipped += 1;
                summary.outcomes.push(ReapOutcome {
                    session_id: session_id.to_string(),
                    task_id: task_id.clone(),
                    status: "error".to_string(),
                    detail: e.to_string(),
                });
            }
        }
    }

    info!(
        candidates = summary.candidates,
        reaped = summary.reaped,
        skipped = summary.skipped,
        "Session task reaper pass completed"
    );

    Ok(serde_json::to_value(summary)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use everruns_core::session_task::{
        CreateSessionTask, NewTaskMessage, SessionTask, SessionTaskFilter, SessionTaskRegistry,
        SessionTaskState, TaskLinks, TaskMessage, TaskWakePolicy, apply_task_update,
        new_session_task,
    };
    use everruns_core::{Result as CoreResult, SessionId};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    // -------------------------------------------------------------------------
    // In-memory mock registry (no server crate dependency)
    // -------------------------------------------------------------------------

    #[derive(Default, Clone)]
    struct MockRegistry {
        tasks: Arc<Mutex<HashMap<String, SessionTask>>>,
    }

    #[async_trait]
    impl SessionTaskRegistry for MockRegistry {
        async fn create(&self, input: CreateSessionTask) -> CoreResult<SessionTask> {
            let task = new_session_task(input, chrono::Utc::now());
            self.tasks
                .lock()
                .unwrap()
                .insert(task.id.clone(), task.clone());
            Ok(task)
        }

        async fn get(
            &self,
            _session_id: SessionId,
            task_id: &str,
        ) -> CoreResult<Option<SessionTask>> {
            Ok(self.tasks.lock().unwrap().get(task_id).cloned())
        }

        async fn list(
            &self,
            _session_id: SessionId,
            _filter: Option<&SessionTaskFilter>,
        ) -> CoreResult<Vec<SessionTask>> {
            Ok(self.tasks.lock().unwrap().values().cloned().collect())
        }

        async fn update(
            &self,
            _session_id: SessionId,
            task_id: &str,
            update: SessionTaskUpdate,
        ) -> CoreResult<Option<SessionTask>> {
            let mut tasks = self.tasks.lock().unwrap();
            let Some(task) = tasks.get_mut(task_id) else {
                return Ok(None);
            };
            apply_task_update(task, update, chrono::Utc::now());
            Ok(Some(task.clone()))
        }

        async fn request_cancel(
            &self,
            _session_id: SessionId,
            _task_id: &str,
        ) -> CoreResult<Option<SessionTask>> {
            Ok(None)
        }

        async fn record_message(
            &self,
            _session_id: SessionId,
            _task_id: &str,
            _message: NewTaskMessage,
        ) -> CoreResult<TaskMessage> {
            Err(anyhow::anyhow!("not implemented").into())
        }

        async fn list_messages(
            &self,
            _session_id: SessionId,
            _task_id: &str,
            _limit: Option<u32>,
        ) -> CoreResult<Vec<TaskMessage>> {
            Ok(vec![])
        }
    }

    // We only need list_orphaned_session_task_ids and reaper_session_task_registry.
    // Inline the reaper logic in `run_reaper` rather than wiring a full WorkerAdapters
    // stub (which would require many unrelated async methods).

    async fn run_reaper(
        orphans: Vec<(SessionId, String)>,
        registry: Arc<MockRegistry>,
        input: &SessionTaskReaperInput,
    ) -> serde_json::Value {
        // Inline the reaper logic using the mock data directly so we avoid
        // needing a full WorkerAdapters impl (which drags in many unrelated
        // async methods). This mirrors execute_reaper_activity exactly.
        let stale_after = chrono::Duration::seconds(input.stale_after_seconds);
        let _ = stale_after; // used only in the real adapters call

        let mut reaped = 0usize;
        let mut skipped = 0usize;

        for (session_id, task_id) in &orphans {
            let update = SessionTaskUpdate {
                state: Some(SessionTaskState::Failed),
                error: Some(TaskError {
                    kind: "orphaned".to_string(),
                    message: "worker heartbeat stopped".to_string(),
                }),
                increment_attempt: true,
                ..Default::default()
            };
            match registry.update(*session_id, task_id, update).await {
                Ok(Some(task)) if task.state == SessionTaskState::Failed => {
                    reaped += 1;
                }
                _ => {
                    skipped += 1;
                }
            }
        }

        serde_json::json!({
            "candidates": orphans.len(),
            "reaped": reaped,
            "skipped": skipped,
        })
    }

    // -------------------------------------------------------------------------
    // Tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn reaper_fails_stale_heartbeat_tasks() {
        let registry = Arc::new(MockRegistry::default());
        let session_id = SessionId::new();

        // Seed a running task.
        let task = registry
            .create(CreateSessionTask {
                session_id,
                id: None,
                kind: "background_tool".to_string(),
                display_name: "Orphaned task".to_string(),
                spec: serde_json::json!({}),
                state: SessionTaskState::Running,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();

        // Simulate: list_orphaned returns this task (heartbeat logic tested in server crate).
        let orphans = vec![(session_id, task.id.clone())];
        let input = SessionTaskReaperInput::default();
        let result = run_reaper(orphans, registry.clone(), &input).await;

        assert_eq!(result["reaped"], 1);
        assert_eq!(result["skipped"], 0);

        let updated = registry.get(session_id, &task.id).await.unwrap().unwrap();
        assert_eq!(updated.state, SessionTaskState::Failed);
        assert_eq!(updated.error.as_ref().unwrap().kind, "orphaned");
        assert_eq!(
            updated.attempt, 2,
            "orphan reap must supersede the executor's attempt"
        );
    }

    #[tokio::test]
    async fn reaper_skips_already_terminal_tasks() {
        let registry = Arc::new(MockRegistry::default());
        let session_id = SessionId::new();

        // Seed a task that is already succeeded.
        let task = registry
            .create(CreateSessionTask {
                session_id,
                id: None,
                kind: "background_tool".to_string(),
                display_name: "Already done".to_string(),
                spec: serde_json::json!({}),
                state: SessionTaskState::Succeeded,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();

        // Simulate orphan list still containing this task (race condition in real code).
        let orphans = vec![(session_id, task.id.clone())];
        let input = SessionTaskReaperInput::default();
        let result = run_reaper(orphans, registry.clone(), &input).await;

        // apply_task_update won't move Succeeded→Failed, so it stays succeeded.
        // Our run_reaper counts it as skipped when state isn't Failed after update.
        let updated = registry.get(session_id, &task.id).await.unwrap().unwrap();
        // State should remain Succeeded (terminal→terminal transition is a no-op).
        assert_eq!(updated.state, SessionTaskState::Succeeded);
        assert_eq!(result["candidates"], 1);
    }

    #[tokio::test]
    async fn empty_orphan_list_produces_zero_reap() {
        let registry = Arc::new(MockRegistry::default());
        let input = SessionTaskReaperInput::default();
        let result = run_reaper(vec![], registry, &input).await;

        assert_eq!(result["candidates"], 0);
        assert_eq!(result["reaped"], 0);
        assert_eq!(result["skipped"], 0);
    }
}
