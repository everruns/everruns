//! Neutral durable turn control shared by embedded and product composition.

use std::sync::Arc;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use chrono::Utc;
use everruns_durable::{
    InMemoryWorkflowEventStore, PostgresWorkflowEventStore, WorkflowEvent, WorkflowEventStore,
    WorkflowSignal, WorkflowStatus,
};
pub use everruns_host::RuntimeTurnState as DurableTurnInput;
use everruns_provider::typed_id::{AgentId, HarnessId, MessageId, SessionId};
use tokio::sync::Mutex;
use uuid::Uuid;

/// Control-plane surface used by the server API to submit and steer turns.
///
/// This is deliberately not an agent executor or product DTO boundary. It
/// maps neutral typed identities onto the durable workflow substrate; workers
/// consume the resulting portable [`DurableTurnInput`].
#[async_trait]
pub trait RunController: Send + Sync {
    /// Start a new durable turn, or steer the existing workflow for the session.
    async fn start_run(
        &self,
        org_id: i64,
        session_id: SessionId,
        harness_id: HarnessId,
        agent_id: Option<AgentId>,
        input_message_id: MessageId,
        request_id: Option<String>,
    ) -> Result<()>;

    /// Wake a durable turn after externally supplied tool results arrive.
    async fn resume_after_tool_results(&self, session_id: SessionId) -> Result<()>;
    /// Request cancellation and remove pending work for the session.
    async fn cancel_run(&self, session_id: SessionId) -> Result<()>;
    /// Return whether the session has a running durable workflow.
    async fn is_running(&self, session_id: SessionId) -> bool;
    /// Count currently active durable workflows.
    async fn active_count(&self) -> usize;
}

/// Notification seam used to wake transport-specific worker pollers.
#[async_trait]
pub trait DurableTaskNotifier: Send + Sync {
    /// Notify workers that a task of `activity_type` may be claimed.
    async fn notify_task_available(&self, activity_type: &str);
}

/// Terminal metadata recorded by the durable driver.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DurableTurnOutput {
    /// Session whose turn reached a terminal state.
    pub session_id: SessionId,
    /// Whether the turn completed successfully.
    pub success: bool,
    /// Terminal failure detail, when present.
    pub error: Option<String>,
    /// Canonical reason the turn stopped.
    pub stop_reason: everruns_core::turn::TurnStopReason,
}

/// Store contract sufficient to drive a durable turn without server or worker
/// dependencies.
#[async_trait]
pub trait DurableStoreBackend: Send + Sync {
    /// Load the workflow's status and optional terminal output/error.
    async fn get_workflow_status(
        &mut self,
        workflow_id: Uuid,
    ) -> Result<(WorkflowStatus, Option<serde_json::Value>, Option<String>)>;
    /// Persist a workflow status transition and optional terminal data.
    async fn update_workflow_status(
        &mut self,
        workflow_id: Uuid,
        status: WorkflowStatus,
        output: Option<serde_json::Value>,
        error: Option<String>,
    ) -> Result<()>;
    /// Enqueue one activity for an existing workflow.
    async fn enqueue_task(
        &mut self,
        workflow_id: Uuid,
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
    ) -> Result<Uuid>;
    /// Atomically create a workflow and enqueue its first activity.
    async fn start_workflow_with_task(
        &mut self,
        workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
        activity_id: String,
        activity_type: String,
    ) -> Result<Uuid>;
    /// Count workflows whose status is active.
    async fn count_active_workflows(&mut self) -> Result<usize>;
    /// Cancel all pending tasks for a workflow.
    async fn cancel_pending_tasks(&mut self, workflow_id: Uuid) -> Result<u64>;
    /// Append events after the caller's expected workflow sequence.
    async fn append_events(
        &mut self,
        workflow_id: Uuid,
        expected_sequence: i32,
        events: Vec<WorkflowEvent>,
    ) -> Result<i32>;
    /// Atomically claim an idle workflow for a new turn.
    async fn try_claim_workflow_for_new_turn(&mut self, workflow_id: Uuid) -> Result<bool>;
    /// Append a signal for a running workflow.
    async fn send_signal(&mut self, workflow_id: Uuid, signal: WorkflowSignal) -> Result<()>;
    /// Read and atomically consume pending workflow signals.
    async fn get_and_consume_signals(&mut self, workflow_id: Uuid) -> Result<Vec<WorkflowSignal>>;
}

/// Direct PostgreSQL durable substrate adapter.
pub struct PostgresDurableStore {
    store: PostgresWorkflowEventStore,
}

impl PostgresDurableStore {
    /// Adapt a PostgreSQL pool to Scale's neutral durable-store contract.
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self {
            store: PostgresWorkflowEventStore::new(pool),
        }
    }
}

/// Shared in-memory durable substrate adapter for tests and embedded dev mode.
pub struct InMemoryDurableStore {
    store: Arc<InMemoryWorkflowEventStore>,
}

impl InMemoryDurableStore {
    /// Create an isolated in-memory durable store.
    pub fn new() -> Self {
        Self {
            store: Arc::new(InMemoryWorkflowEventStore::new()),
        }
    }

    /// Adapt a shared in-memory workflow store.
    pub fn from_shared(store: Arc<InMemoryWorkflowEventStore>) -> Self {
        Self { store }
    }

    /// Return the shared underlying workflow store.
    pub fn store(&self) -> Arc<InMemoryWorkflowEventStore> {
        Arc::clone(&self.store)
    }
}

impl Default for InMemoryDurableStore {
    fn default() -> Self {
        Self::new()
    }
}

macro_rules! impl_durable_store {
    ($type:ty) => {
        #[async_trait]
        impl DurableStoreBackend for $type {
            async fn get_workflow_status(
                &mut self,
                workflow_id: Uuid,
            ) -> Result<(WorkflowStatus, Option<serde_json::Value>, Option<String>)> {
                let info = self.store.get_workflow_info(workflow_id).await?;
                Ok((
                    info.status,
                    info.result,
                    info.error.map(|error| format!("{error:?}")),
                ))
            }

            async fn update_workflow_status(
                &mut self,
                workflow_id: Uuid,
                status: WorkflowStatus,
                output: Option<serde_json::Value>,
                error: Option<String>,
            ) -> Result<()> {
                self.store
                    .update_workflow_status(
                        workflow_id,
                        status,
                        output,
                        error.map(everruns_durable::WorkflowError::new),
                    )
                    .await?;
                Ok(())
            }

            async fn enqueue_task(
                &mut self,
                workflow_id: Uuid,
                activity_id: String,
                activity_type: String,
                input: serde_json::Value,
            ) -> Result<Uuid> {
                self.store
                    .enqueue_task(everruns_durable::TaskDefinition {
                        workflow_id: Some(workflow_id),
                        activity_id,
                        activity_type,
                        input,
                        options: Default::default(),
                    })
                    .await
                    .map_err(Into::into)
            }

            async fn start_workflow_with_task(
                &mut self,
                workflow_id: Uuid,
                workflow_type: &str,
                input: serde_json::Value,
                activity_id: String,
                activity_type: String,
            ) -> Result<Uuid> {
                self.store
                    .start_workflow_with_task(
                        workflow_id,
                        workflow_type,
                        input.clone(),
                        everruns_durable::TaskDefinition {
                            workflow_id: Some(workflow_id),
                            activity_id,
                            activity_type,
                            input,
                            options: Default::default(),
                        },
                    )
                    .await
                    .map_err(Into::into)
            }

            async fn count_active_workflows(&mut self) -> Result<usize> {
                self.store
                    .count_active_workflows()
                    .await
                    .map(|count| count as usize)
                    .map_err(Into::into)
            }

            async fn cancel_pending_tasks(&mut self, workflow_id: Uuid) -> Result<u64> {
                self.store
                    .cancel_pending_tasks_for_workflow(workflow_id)
                    .await
                    .map_err(Into::into)
            }

            async fn append_events(
                &mut self,
                workflow_id: Uuid,
                expected_sequence: i32,
                events: Vec<WorkflowEvent>,
            ) -> Result<i32> {
                self.store
                    .append_events(workflow_id, expected_sequence, events)
                    .await
                    .map_err(Into::into)
            }

            async fn try_claim_workflow_for_new_turn(&mut self, workflow_id: Uuid) -> Result<bool> {
                self.store
                    .try_claim_workflow_for_new_turn(workflow_id)
                    .await
                    .map_err(Into::into)
            }

            async fn send_signal(
                &mut self,
                workflow_id: Uuid,
                signal: WorkflowSignal,
            ) -> Result<()> {
                self.store
                    .send_signal(workflow_id, signal)
                    .await
                    .map_err(Into::into)
            }

            async fn get_and_consume_signals(
                &mut self,
                workflow_id: Uuid,
            ) -> Result<Vec<WorkflowSignal>> {
                self.store
                    .consume_pending_signals(workflow_id)
                    .await
                    .map_err(Into::into)
            }
        }
    };
}

impl_durable_store!(PostgresDurableStore);

#[async_trait]
impl DurableStoreBackend for InMemoryDurableStore {
    async fn get_workflow_status(
        &mut self,
        workflow_id: Uuid,
    ) -> Result<(WorkflowStatus, Option<serde_json::Value>, Option<String>)> {
        let info = self.store.get_workflow_info(workflow_id).await?;
        Ok((
            info.status,
            info.result,
            info.error.map(|error| format!("{error:?}")),
        ))
    }

    async fn update_workflow_status(
        &mut self,
        workflow_id: Uuid,
        status: WorkflowStatus,
        output: Option<serde_json::Value>,
        error: Option<String>,
    ) -> Result<()> {
        self.store
            .update_workflow_status(
                workflow_id,
                status,
                output,
                error.map(everruns_durable::WorkflowError::new),
            )
            .await?;
        Ok(())
    }

    async fn enqueue_task(
        &mut self,
        workflow_id: Uuid,
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
    ) -> Result<Uuid> {
        self.store
            .enqueue_task(everruns_durable::TaskDefinition {
                workflow_id: Some(workflow_id),
                activity_id,
                activity_type,
                input,
                options: Default::default(),
            })
            .await
            .map_err(Into::into)
    }

    async fn start_workflow_with_task(
        &mut self,
        workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
        activity_id: String,
        activity_type: String,
    ) -> Result<Uuid> {
        self.store
            .create_workflow(workflow_id, workflow_type, input.clone(), None)
            .await?;
        self.store
            .append_events(
                workflow_id,
                0,
                vec![WorkflowEvent::WorkflowStarted {
                    input: input.clone(),
                }],
            )
            .await?;
        let task_id = self
            .enqueue_task(
                workflow_id,
                activity_id.clone(),
                activity_type.clone(),
                input.clone(),
            )
            .await?;
        self.store
            .update_workflow_status(workflow_id, WorkflowStatus::Running, None, None)
            .await?;
        self.store
            .append_events(
                workflow_id,
                1,
                vec![WorkflowEvent::ActivityScheduled {
                    activity_id,
                    activity_type,
                    input,
                    options: Default::default(),
                }],
            )
            .await?;
        Ok(task_id)
    }

    async fn count_active_workflows(&mut self) -> Result<usize> {
        Ok(self.store.workflow_count())
    }

    async fn cancel_pending_tasks(&mut self, workflow_id: Uuid) -> Result<u64> {
        self.store
            .cancel_pending_tasks_for_workflow(workflow_id)
            .await
            .map_err(Into::into)
    }

    async fn append_events(
        &mut self,
        workflow_id: Uuid,
        expected_sequence: i32,
        events: Vec<WorkflowEvent>,
    ) -> Result<i32> {
        self.store
            .append_events(workflow_id, expected_sequence, events)
            .await
            .map_err(Into::into)
    }

    async fn try_claim_workflow_for_new_turn(&mut self, workflow_id: Uuid) -> Result<bool> {
        self.store
            .try_claim_workflow_for_new_turn(workflow_id)
            .await
            .map_err(Into::into)
    }

    async fn send_signal(&mut self, workflow_id: Uuid, signal: WorkflowSignal) -> Result<()> {
        self.store
            .send_signal(workflow_id, signal)
            .await
            .map_err(Into::into)
    }

    async fn get_and_consume_signals(&mut self, workflow_id: Uuid) -> Result<Vec<WorkflowSignal>> {
        self.store
            .consume_pending_signals(workflow_id)
            .await
            .map_err(Into::into)
    }
}

/// Durable, transport-neutral turn driver owned by Scale.
#[derive(Clone)]
pub struct DurableTurnDriver {
    store: Arc<Mutex<Box<dyn DurableStoreBackend>>>,
    notifier: Option<Arc<dyn DurableTaskNotifier>>,
}

impl DurableTurnDriver {
    /// Build a driver over a custom neutral durable-store backend.
    pub fn from_store(store: impl DurableStoreBackend + 'static) -> Self {
        Self {
            store: Arc::new(Mutex::new(Box::new(store))),
            notifier: None,
        }
    }

    /// Build a driver backed by PostgreSQL.
    pub fn postgres(pool: sqlx::PgPool) -> Self {
        Self::from_store(PostgresDurableStore::new(pool))
    }

    /// Build an isolated in-memory driver.
    pub fn in_memory() -> Self {
        Self::from_store(InMemoryDurableStore::new())
    }

    /// Build a driver over a shared in-memory workflow store.
    pub fn shared_in_memory(store: Arc<InMemoryWorkflowEventStore>) -> Self {
        Self::from_store(InMemoryDurableStore::from_shared(store))
    }

    /// Install a worker wake-up notifier.
    pub fn with_notifier(mut self, notifier: Arc<dyn DurableTaskNotifier>) -> Self {
        self.notifier = Some(notifier);
        self
    }

    async fn notify(&self, activity_type: &str) {
        if let Some(notifier) = &self.notifier {
            notifier.notify_task_available(activity_type).await;
        }
    }
}

#[async_trait]
impl RunController for DurableTurnDriver {
    async fn start_run(
        &self,
        org_id: i64,
        session_id: SessionId,
        harness_id: HarnessId,
        agent_id: Option<AgentId>,
        input_message_id: MessageId,
        request_id: Option<String>,
    ) -> Result<()> {
        let input = DurableTurnInput {
            org_id,
            session_id,
            harness_id,
            agent_id,
            input_message_id,
            turn_id: None,
            previous_response_id: None,
            iteration: 1,
            request_id,
            started_at: Some(Utc::now()),
            cumulative_usage: None,
            tool_call_count: 0,
            llm_call_count: 0,
            time_to_first_token_ms: None,
            final_message_id: None,
            final_answer_preview: None,
        };
        let workflow_id = session_id.uuid();
        let input_json = serde_json::to_value(&input)?;
        let notify = {
            let mut store = self.store.lock().await;
            match store.try_claim_workflow_for_new_turn(workflow_id).await {
                Ok(true) => {
                    store
                        .enqueue_task(
                            workflow_id,
                            format!("input_{}", Uuid::now_v7()),
                            "process_input".into(),
                            input_json,
                        )
                        .await?;
                    Some("process_input")
                }
                Ok(false) => {
                    store
                        .send_signal(
                            workflow_id,
                            WorkflowSignal::new(
                                everruns_durable::signal_types::USER_MESSAGE,
                                serde_json::json!({
                                    "input_message_id": input_message_id.to_string(),
                                    "org_id": org_id,
                                    "harness_id": harness_id.to_string(),
                                    "agent_id": agent_id.map(|id| id.to_string()),
                                }),
                            ),
                        )
                        .await?;
                    None
                }
                Err(error)
                    if error.to_string().contains("not found")
                        || error.to_string().contains("NOT_FOUND") =>
                {
                    store
                        .start_workflow_with_task(
                            workflow_id,
                            "turn_workflow",
                            input_json,
                            format!("input_{}", Uuid::now_v7()),
                            "process_input".into(),
                        )
                        .await?;
                    Some("process_input")
                }
                Err(error) => return Err(error),
            }
        };
        if let Some(activity_type) = notify {
            self.notify(activity_type).await;
        }
        Ok(())
    }

    async fn resume_after_tool_results(&self, session_id: SessionId) -> Result<()> {
        let workflow_id = session_id.uuid();
        let mut store = self.store.lock().await;
        let (status, saved, _) = store.get_workflow_status(workflow_id).await?;
        if status != WorkflowStatus::Completed {
            return Err(anyhow!("workflow {workflow_id} is not completed"));
        }
        let turn: DurableTurnInput = serde_json::from_value(
            saved.ok_or_else(|| anyhow!("workflow {workflow_id} has no saved turn input"))?,
        )?;
        store
            .update_workflow_status(workflow_id, WorkflowStatus::Pending, None, None)
            .await?;
        store
            .enqueue_task(
                workflow_id,
                format!("reason_{}", Uuid::now_v7()),
                "reason".into(),
                serde_json::to_value(turn)?,
            )
            .await?;
        drop(store);
        self.notify("reason").await;
        Ok(())
    }

    async fn cancel_run(&self, session_id: SessionId) -> Result<()> {
        let workflow_id = session_id.uuid();
        let mut store = self.store.lock().await;
        store.cancel_pending_tasks(workflow_id).await?;
        let message = "User requested cancellation".to_string();
        store
            .update_workflow_status(
                workflow_id,
                WorkflowStatus::Cancelled,
                Some(serde_json::to_value(DurableTurnOutput {
                    session_id,
                    success: false,
                    error: Some(message.clone()),
                    stop_reason: everruns_core::turn::TurnStopReason::Cancelled,
                })?),
                Some(message),
            )
            .await
    }

    async fn is_running(&self, session_id: SessionId) -> bool {
        self.store
            .lock()
            .await
            .get_workflow_status(session_id.uuid())
            .await
            .is_ok_and(|(status, _, _)| !status.is_terminal())
    }

    async fn active_count(&self) -> usize {
        self.store
            .lock()
            .await
            .count_active_workflows()
            .await
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn restartable_controller_steers_without_duplicate_start_events() {
        let store = Arc::new(InMemoryWorkflowEventStore::new());
        let first = DurableTurnDriver::shared_in_memory(store.clone());
        let session_id = SessionId::new();
        let harness_id = HarnessId::new();

        first
            .start_run(
                1,
                session_id,
                harness_id,
                None,
                MessageId::new(),
                Some("request-1".into()),
            )
            .await
            .unwrap();

        // A fresh driver over the same durable substrate represents a process
        // restart. A second message must become a signal, not a second
        // WorkflowStarted/ActivityScheduled pair.
        let restarted = DurableTurnDriver::shared_in_memory(store.clone());
        restarted
            .start_run(
                1,
                session_id,
                harness_id,
                None,
                MessageId::new(),
                Some("request-2".into()),
            )
            .await
            .unwrap();

        let events = store.load_events(session_id.uuid()).await.unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|(_, event)| matches!(event, WorkflowEvent::WorkflowStarted { .. }))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|(_, event)| matches!(event, WorkflowEvent::ActivityScheduled { .. }))
                .count(),
            1
        );
        let signals = store
            .consume_pending_signals(session_id.uuid())
            .await
            .unwrap();
        assert_eq!(signals.len(), 1);

        restarted.cancel_run(session_id).await.unwrap();
        assert!(!restarted.is_running(session_id).await);
        let info = store.get_workflow_info(session_id.uuid()).await.unwrap();
        assert_eq!(info.status, WorkflowStatus::Cancelled);
    }
}
