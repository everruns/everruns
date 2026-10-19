//! Runs one claimed durable task: the activities that are not turns.
//!
//! The durable queue carries leased-resource cleanup, the session task
//! reaper, and scheduled channel and agent-trigger invocations; turns run on
//! session actor leases (`actor_runtime`). Each claimed task gets the same
//! lifecycle: skip it when its workflow was cancelled, record the start,
//! heartbeat the claim while the activity runs, then complete it or fail it
//! (retryable unless the error says otherwise).
//!
//! Decision: the activity bodies take no cancellation. They are short
//! control-plane calls, so a reclaimed or cancelled task finishes its call and
//! its completion is refused (`TaskNotOwned`), rather than being cut off
//! mid-write.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use everruns_contracts::error::AgentLoopError;
use tokio::sync::oneshot;
use tracing::{debug, info, warn};

use crate::activities::{ScheduledAgentTriggerInput, ScheduledChannelInput, activity_types};
use crate::durable::{ClaimedTask, WorkflowStatus};
use crate::task_store::TaskStore;
use crate::worker_adapters::WorkerAdapters;

/// Runs claimed tasks of the worker's activity types against a [`TaskStore`].
pub struct TaskDriver<S: TaskStore, A: WorkerAdapters> {
    store: Arc<S>,
    adapters: A,
    worker_id: String,
    heartbeat_interval: Duration,
}

impl<S: TaskStore, A: WorkerAdapters> Clone for TaskDriver<S, A> {
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            adapters: self.adapters.clone(),
            worker_id: self.worker_id.clone(),
            heartbeat_interval: self.heartbeat_interval,
        }
    }
}

impl<S: TaskStore, A: WorkerAdapters> TaskDriver<S, A> {
    /// A driver that settles tasks as `worker_id` and heartbeats each one
    /// every `heartbeat_interval`.
    pub fn new(
        store: Arc<S>,
        adapters: A,
        worker_id: impl Into<String>,
        heartbeat_interval: Duration,
    ) -> Self {
        Self {
            store,
            adapters,
            worker_id: worker_id.into(),
            heartbeat_interval,
        }
    }

    /// Execute one claimed task to completion or failure.
    ///
    /// # Errors
    ///
    /// Returns the activity's error after recording the failure in the store.
    pub async fn execute_task(&self, task: &ClaimedTask) -> Result<()> {
        info!(
            task_id = %task.id,
            workflow_id = ?task.workflow_id,
            activity_type = %task.activity_type,
            attempt = task.attempt,
            "Executing task"
        );

        // The claim reports the status of a workflow-bound task.
        if task.workflow_status == Some(WorkflowStatus::Cancelled) {
            info!(task_id = %task.id, "Workflow cancelled, skipping task");
            let _ = self
                .store
                .fail_task(task, "Workflow cancelled", false)
                .await;
            return Ok(());
        }

        self.store
            .record_activity_started(task, &self.worker_id)
            .await;
        let (stop_heartbeat, heartbeat) = spawn_task_heartbeat(
            self.store.clone(),
            task.id,
            self.worker_id.clone(),
            self.heartbeat_interval,
        );
        let result = run_activity(&self.adapters, task).await;
        let _ = stop_heartbeat.send(());
        let _ = heartbeat.await;

        match result {
            Ok(output) => {
                match self
                    .store
                    .complete_task(task, &self.worker_id, output)
                    .await
                {
                    Ok(()) => info!(
                        task_id = %task.id,
                        activity_type = %task.activity_type,
                        "Task completed successfully"
                    ),
                    Err(e) => warn!(task_id = %task.id, error = %e, "Task completion rejected"),
                }
                Ok(())
            }
            Err(error) => {
                let message = failure_message(task, &error);
                let retryable = !is_non_retryable(&error);
                self.store
                    .fail_task(task, &message, retryable)
                    .await
                    .map_err(|store_error| {
                        anyhow::anyhow!("Failed to persist task failure: {store_error}")
                    })?;
                Err(error)
            }
        }
    }
}

/// Run `task`'s activity on `adapters`.
async fn run_activity<A: WorkerAdapters>(
    adapters: &A,
    task: &ClaimedTask,
) -> Result<serde_json::Value> {
    fn input<T: serde::de::DeserializeOwned>(task: &ClaimedTask) -> Result<T> {
        serde_json::from_value(task.input.clone())
            .map_err(|e| anyhow::anyhow!("Failed to parse {} input: {}", task.activity_type, e))
    }
    match task.activity_type.as_str() {
        activity_types::LEASED_RESOURCE_CLEANUP => {
            crate::leased_resource_cleanup::execute_cleanup_activity(adapters, &input(task)?).await
        }
        activity_types::SESSION_TASK_REAPER => {
            crate::session_task_reaper::execute_reaper_activity(adapters, &input(task)?).await
        }
        activity_types::INVOKE_SCHEDULED_CHANNEL => {
            let input: ScheduledChannelInput = input(task)?;
            adapters
                .invoke_scheduled_channel(input.org_id, &input.app_id, &input.channel_id)
                .await
                .map_err(anyhow::Error::from)
        }
        activity_types::INVOKE_AGENT_TRIGGER => {
            let input: ScheduledAgentTriggerInput = input(task)?;
            adapters
                .invoke_agent_trigger(input.org_id, &input.agent_id, &input.trigger_id)
                .await
                .map_err(anyhow::Error::from)
        }
        other => Err(anyhow::anyhow!("Unknown activity type: {other}")),
    }
}

/// Keep the claim alive until the returned sender fires.
fn spawn_task_heartbeat<S: TaskStore>(
    store: Arc<S>,
    task_id: uuid::Uuid,
    worker_id: String,
    interval: Duration,
) -> (oneshot::Sender<()>, tokio::task::JoinHandle<()>) {
    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        let mut ticks = tokio::time::interval(interval);
        loop {
            tokio::select! {
                _ = ticks.tick() => {
                    match store.heartbeat_task(task_id, &worker_id).await {
                        Ok(response) if !response.accepted || response.should_cancel => {
                            // Reclaimed or cancelled: the completion will be
                            // refused; stop renewing a claim this worker lost.
                            warn!(%task_id, "Task claim lost or cancelled");
                            break;
                        }
                        Ok(_) => debug!(%task_id, "Task heartbeat sent"),
                        Err(error) => warn!(%task_id, %error, "Failed to send task heartbeat"),
                    }
                }
                _ = &mut stop_rx => break,
            }
        }
    });
    (stop_tx, handle)
}

/// The failure string persisted on the task: its identity, then the full
/// error chain.
pub(crate) fn failure_message(task: &ClaimedTask, error: &anyhow::Error) -> String {
    let mut chain: Vec<String> = Vec::new();
    for cause in error.chain() {
        let message = cause.to_string();
        if !message.is_empty() && chain.last() != Some(&message) {
            chain.push(message);
        }
    }
    let mut metadata = format!(
        "activity_type={} task_id={} attempt={}/{}",
        task.activity_type, task.id, task.attempt, task.max_attempts
    );
    if let Some(workflow_id) = task.workflow_id {
        metadata.push_str(&format!(" workflow_id={workflow_id}"));
    }
    format!("{metadata} | error_chain={}", chain.join(": "))
}

/// Typed retry semantics survive anyhow's activity error boundary.
fn is_non_retryable(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<AgentLoopError>()
            .is_some_and(AgentLoopError::is_non_retryable)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task() -> ClaimedTask {
        ClaimedTask {
            id: uuid::Uuid::nil(),
            activity_type: "session_task_reaper".to_string(),
            attempt: 2,
            max_attempts: 3,
            ..Default::default()
        }
    }

    #[test]
    fn failure_message_keeps_identity_and_the_whole_chain() {
        let error = anyhow::anyhow!("connection reset").context("reaper failed");
        assert_eq!(
            failure_message(&task(), &error),
            "activity_type=session_task_reaper task_id=00000000-0000-0000-0000-000000000000 \
             attempt=2/3 | error_chain=reaper failed: connection reset"
        );
    }

    #[test]
    fn typed_non_retryable_errors_are_not_retried() {
        let fatal = anyhow::Error::from(AgentLoopError::NoMessages).context("reaper failed");
        assert!(is_non_retryable(&fatal));
        assert!(!is_non_retryable(&anyhow::anyhow!("transient")));
    }
}
