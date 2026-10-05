// Durable execution engine runner adapters.
// Decision: everruns-durable-engine owns durable orchestration and maps runtime turn state onto the durable engine.
// Decision: Core host remains durable-agnostic and only exports generic turn strategy/state.

use anyhow::Result;
use async_trait::async_trait;
use everruns_contracts::typed_id::SessionId;
pub use everruns_core::engine::TurnState as DurableTurnInput;
use everruns_durable::{
    DurableAdmin, EventLog, InMemoryWorkflowEventStore, PostgresWorkflowEventStore, SignalStore,
    TaskQueue, WorkflowEvent, WorkflowSignal, WorkflowStatus,
};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;
use uuid::Uuid;

#[async_trait]
pub trait DurableTaskNotifier: Send + Sync {
    async fn notify_task_available(&self, activity_type: &str);
}

/// Output metadata for durable turns.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DurableTurnOutput {
    pub session_id: SessionId,
    pub success: bool,
    pub error: Option<String>,
    pub stop_reason: everruns_core::turn::TurnStopReason,
}

#[async_trait]
pub trait DurableStoreBackend: Send + Sync {
    async fn get_workflow_status(
        &mut self,
        workflow_id: Uuid,
    ) -> Result<(WorkflowStatus, Option<serde_json::Value>, Option<String>)>;

    async fn create_workflow(
        &mut self,
        workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
    ) -> Result<Uuid>;

    async fn update_workflow_status(
        &mut self,
        workflow_id: Uuid,
        status: WorkflowStatus,
        output: Option<serde_json::Value>,
        error: Option<String>,
    ) -> Result<()>;

    async fn enqueue_task(
        &mut self,
        workflow_id: Uuid,
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
    ) -> Result<Uuid>;

    async fn start_workflow_with_task(
        &mut self,
        workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
        activity_id: String,
        activity_type: String,
    ) -> Result<Uuid>;

    async fn count_active_workflows(&mut self) -> Result<usize>;

    async fn cancel_pending_tasks(&mut self, workflow_id: Uuid) -> Result<u64>;

    async fn append_events(
        &mut self,
        workflow_id: Uuid,
        expected_sequence: i32,
        events: Vec<WorkflowEvent>,
    ) -> Result<i32>;

    async fn try_claim_workflow_for_new_turn(&mut self, workflow_id: Uuid) -> Result<bool>;

    async fn send_signal(&mut self, workflow_id: Uuid, signal: WorkflowSignal) -> Result<()>;

    async fn get_and_consume_signals(&mut self, workflow_id: Uuid) -> Result<Vec<WorkflowSignal>>;

    /// The output the workflow's latest `WorkflowCompleted` event recorded.
    ///
    /// A [`DurableRunner`] turn ticket reads it to fill in the turn's
    /// response and stop reason. The default returns `None`, for a store
    /// without a cheap event-log read; the ticket then falls back to the
    /// turn checkpoint (see [`crate::turn_backend`]).
    async fn latest_completion_output(
        &mut self,
        workflow_id: Uuid,
    ) -> Result<Option<serde_json::Value>> {
        let _ = workflow_id;
        Ok(None)
    }
}

/// Direct database store for control-plane use.
pub struct DirectDurableStore {
    store: PostgresWorkflowEventStore,
}

impl DirectDurableStore {
    pub fn new(pool: everruns_durable::PostgresPool) -> Self {
        Self {
            store: PostgresWorkflowEventStore::new(pool),
        }
    }
}

/// In-memory store for dev mode (no PostgreSQL required).
pub struct InMemoryDurableStore {
    store: Arc<InMemoryWorkflowEventStore>,
}

impl InMemoryDurableStore {
    pub fn new() -> Self {
        Self {
            store: Arc::new(InMemoryWorkflowEventStore::new()),
        }
    }

    pub fn from_shared(store: Arc<InMemoryWorkflowEventStore>) -> Self {
        Self { store }
    }

    pub fn store(&self) -> Arc<InMemoryWorkflowEventStore> {
        Arc::clone(&self.store)
    }
}

impl Default for InMemoryDurableStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DurableStoreBackend for DirectDurableStore {
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

    async fn create_workflow(
        &mut self,
        workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
    ) -> Result<Uuid> {
        self.store
            .create_workflow(workflow_id, workflow_type, input, None)
            .await?;
        Ok(workflow_id)
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
                options: crate::durable_turn::activity_options_for(&activity_id),
                activity_id,
                activity_type,
                input,
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
            .try_start_new_run(workflow_id)
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

    async fn latest_completion_output(
        &mut self,
        workflow_id: Uuid,
    ) -> Result<Option<serde_json::Value>> {
        crate::turn_backend::latest_completion_output(&self.store, workflow_id).await
    }
}

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

    async fn create_workflow(
        &mut self,
        workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
    ) -> Result<Uuid> {
        self.store
            .create_workflow(workflow_id, workflow_type, input, None)
            .await?;
        Ok(workflow_id)
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
                options: crate::durable_turn::activity_options_for(&activity_id),
                activity_id,
                activity_type,
                input,
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
        let _ = self
            .store
            .append_events(workflow_id, 0, vec![WorkflowEvent::started(input.clone())])
            .await?;
        let task_id = self
            .store
            .enqueue_task(everruns_durable::TaskDefinition {
                workflow_id: Some(workflow_id),
                activity_id: activity_id.clone(),
                activity_type: activity_type.clone(),
                input: input.clone(),
                options: Default::default(),
            })
            .await?;
        self.store
            .update_workflow_status(workflow_id, WorkflowStatus::Running, None, None)
            .await?;
        let _ = self
            .store
            .append_events(
                workflow_id,
                1,
                vec![WorkflowEvent::ActivityScheduled {
                    activity_id,
                    activity_type,
                    input,
                    options: everruns_durable::ActivityOptions::default(),
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
            .try_start_new_run(workflow_id)
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

    async fn latest_completion_output(
        &mut self,
        workflow_id: Uuid,
    ) -> Result<Option<serde_json::Value>> {
        crate::turn_backend::latest_completion_output(&*self.store, workflow_id).await
    }
}

/// Durable execution engine based runner.
///
/// This runner maps runtime turn state onto the durable engine.
/// It implements [`TurnBackend`](everruns_core::host::TurnBackend) (see
/// [`crate::turn_backend`]); its [`AgentRunner`](crate::AgentRunner)
/// methods are a shim over that implementation.
pub struct DurableRunner {
    pub(crate) store: Arc<Mutex<dyn DurableStoreBackend>>,
    task_notifier: Option<Arc<dyn DurableTaskNotifier>>,
}

impl DurableRunner {
    /// Build a runner over any durable store backend.
    ///
    /// Process-specific transports (the worker's gRPC store) implement
    /// [`DurableStoreBackend`] in their own crate and enter here, so this crate
    /// stays free of transport dependencies.
    pub fn from_store(store: impl DurableStoreBackend + 'static) -> Self {
        Self {
            store: Arc::new(Mutex::new(store)),
            task_notifier: None,
        }
    }

    pub fn new_with_pool(pool: everruns_durable::PostgresPool) -> Self {
        info!("Initializing durable runner (direct DB mode)");
        let store = DirectDurableStore::new(pool);
        Self {
            store: Arc::new(Mutex::new(store)),
            task_notifier: None,
        }
    }

    pub fn new_with_pool_and_task_notifier(
        pool: everruns_durable::PostgresPool,
        task_notifier: Arc<dyn DurableTaskNotifier>,
    ) -> Self {
        info!("Initializing durable runner (direct DB mode with task notifier)");
        let store = DirectDurableStore::new(pool);
        Self {
            store: Arc::new(Mutex::new(store)),
            task_notifier: Some(task_notifier),
        }
    }

    pub fn new_in_memory() -> Self {
        info!("Initializing durable runner (in-memory dev mode)");
        let store = InMemoryDurableStore::new();
        Self {
            store: Arc::new(Mutex::new(store)),
            task_notifier: None,
        }
    }

    pub fn new_with_shared_store(shared_store: Arc<InMemoryWorkflowEventStore>) -> Self {
        info!("Initializing durable runner (shared in-memory dev mode)");
        let store = InMemoryDurableStore::from_shared(shared_store);
        Self {
            store: Arc::new(Mutex::new(store)),
            task_notifier: None,
        }
    }

    pub fn with_task_notifier(mut self, task_notifier: Arc<dyn DurableTaskNotifier>) -> Self {
        self.task_notifier = Some(task_notifier);
        self
    }

    pub(crate) async fn notify_task_available(&self, activity_type: &str) {
        if let Some(task_notifier) = &self.task_notifier {
            task_notifier.notify_task_available(activity_type).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::AgentRunner;
    use everruns_contracts::typed_id::{AgentId, HarnessId, MessageId};
    use everruns_durable::WorkerRegistry;

    #[derive(Default)]
    struct RecordingTaskNotifier {
        activity_types: std::sync::Mutex<Vec<String>>,
    }

    impl RecordingTaskNotifier {
        fn activity_types(&self) -> Vec<String> {
            self.activity_types
                .lock()
                .expect("recorded activity types lock poisoned")
                .clone()
        }
    }

    #[async_trait]
    impl DurableTaskNotifier for RecordingTaskNotifier {
        async fn notify_task_available(&self, activity_type: &str) {
            self.activity_types
                .lock()
                .expect("recorded activity types lock poisoned")
                .push(activity_type.to_string());
        }
    }

    #[tokio::test]
    async fn start_run_creates_new_workflow_and_input_task() {
        let shared = Arc::new(InMemoryWorkflowEventStore::new());
        let runner = DurableRunner::new_with_shared_store(shared.clone());
        let session_id = SessionId::new();
        let harness_id = HarnessId::new();
        let message_id = MessageId::new();

        runner
            .start_run(1, session_id, harness_id, None, message_id, None)
            .await
            .expect("start_run should create workflow");

        let info = shared
            .get_workflow_info(session_id.uuid())
            .await
            .expect("workflow info should exist");
        assert_eq!(info.status, WorkflowStatus::Running);

        shared
            .register_worker(everruns_durable::WorkerInfo::new(
                "worker-1",
                Vec::<String>::new(),
            ))
            .await
            .expect("register worker");

        let claimed = shared
            .claim_task("worker-1", &["process_input".to_string()], 10)
            .await
            .expect("task should be claimable");
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].activity_type, "process_input");
    }

    #[tokio::test]
    async fn start_run_notifies_initial_process_input_task() {
        let shared = Arc::new(InMemoryWorkflowEventStore::new());
        let notifier = Arc::new(RecordingTaskNotifier::default());
        let runner = DurableRunner::new_with_shared_store(shared.clone())
            .with_task_notifier(notifier.clone());

        runner
            .start_run(
                1,
                SessionId::new(),
                HarnessId::new(),
                None,
                MessageId::new(),
                None,
            )
            .await
            .expect("start_run should create workflow");

        assert_eq!(notifier.activity_types(), vec!["process_input"]);
    }

    #[tokio::test]
    async fn start_run_steers_running_workflow() {
        let shared = Arc::new(InMemoryWorkflowEventStore::new());
        let workflow_id = Uuid::now_v7();
        shared
            .create_workflow(workflow_id, "turn_workflow", serde_json::json!({}), None)
            .await
            .expect("create workflow");
        shared
            .update_workflow_status(workflow_id, WorkflowStatus::Running, None, None)
            .await
            .expect("mark running");

        let runner = DurableRunner::new_with_shared_store(shared.clone());
        let session_id = SessionId::from_uuid(workflow_id);
        let harness_id = HarnessId::new();
        let message_id = MessageId::new();

        runner
            .start_run(1, session_id, harness_id, None, message_id, None)
            .await
            .expect("start_run should send steering signal");

        let signals = shared
            .consume_pending_signals(workflow_id)
            .await
            .expect("signals should load");
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].signal_type, crate::durable_turn::USER_MESSAGE);
    }

    #[tokio::test]
    async fn start_run_steers_pending_workflow_with_claimed_task() {
        let shared = Arc::new(InMemoryWorkflowEventStore::new());
        let workflow_id = Uuid::now_v7();
        shared
            .create_workflow(workflow_id, "turn_workflow", serde_json::json!({}), None)
            .await
            .expect("create workflow");
        shared
            .update_workflow_status(workflow_id, WorkflowStatus::Pending, None, None)
            .await
            .expect("mark pending");
        shared
            .enqueue_task(everruns_durable::TaskDefinition {
                workflow_id: Some(workflow_id),
                activity_id: format!("input_{}", Uuid::now_v7()),
                activity_type: "process_input".to_string(),
                input: serde_json::json!({}),
                options: Default::default(),
            })
            .await
            .expect("enqueue task");
        shared
            .register_worker(everruns_durable::WorkerInfo::new(
                "worker-1",
                Vec::<String>::new(),
            ))
            .await
            .expect("register worker");
        let claimed = shared
            .claim_task("worker-1", &["process_input".to_string()], 1)
            .await
            .expect("claim task");
        assert_eq!(claimed.len(), 1);

        let runner = DurableRunner::new_with_shared_store(shared.clone());
        let session_id = SessionId::from_uuid(workflow_id);

        runner
            .start_run(
                1,
                session_id,
                HarnessId::new(),
                None,
                MessageId::new(),
                None,
            )
            .await
            .expect("start_run should send steering signal");

        let signals = shared
            .consume_pending_signals(workflow_id)
            .await
            .expect("signals should load");
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].signal_type, crate::durable_turn::USER_MESSAGE);

        let additional_claimed = shared
            .claim_task("worker-2", &["process_input".to_string()], 10)
            .await
            .expect("no additional process_input tasks should be available");
        assert!(additional_claimed.is_empty());
    }

    #[tokio::test]
    async fn resume_after_tool_results_enqueues_reason() {
        let shared = Arc::new(InMemoryWorkflowEventStore::new());
        let session_id = SessionId::new();
        let saved_input = DurableTurnInput {
            org_id: 1,
            session_id,
            harness_id: HarnessId::new(),
            agent_id: Some(AgentId::new()),
            input_message_id: MessageId::new(),
            turn_id: None,
            previous_response_id: Some("resp_123".to_string()),
            iteration: 3,
            request_id: None,
            started_at: None,
            cumulative_usage: None,
            tool_call_count: 0,
            llm_call_count: 0,
            time_to_first_token_ms: None,
            final_message_id: None,
            final_answer_preview: None,
        };
        shared
            .create_workflow(
                session_id.uuid(),
                "turn_workflow",
                serde_json::to_value(&saved_input).expect("serialize input"),
                None,
            )
            .await
            .expect("create workflow");
        shared
            .update_workflow_status(
                session_id.uuid(),
                WorkflowStatus::Completed,
                Some(serde_json::to_value(&saved_input).expect("serialize result")),
                None,
            )
            .await
            .expect("mark completed");

        let runner = DurableRunner::new_with_shared_store(shared.clone());
        let resolution_id = Uuid::now_v7();
        runner
            .resume_after_tool_results(session_id, resolution_id)
            .await
            .expect("resume should enqueue reason");
        runner
            .resume_after_tool_results(session_id, resolution_id)
            .await
            .expect("retry should reuse the reason task");

        assert_eq!(
            shared
                .get_workflow_status(session_id.uuid())
                .await
                .expect("status should load"),
            WorkflowStatus::Pending
        );

        shared
            .register_worker(everruns_durable::WorkerInfo::new(
                "worker-1",
                Vec::<String>::new(),
            ))
            .await
            .expect("register worker");

        let claimed = shared
            .claim_task("worker-1", &["reason".to_string()], 10)
            .await
            .expect("reason task should be claimable");
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].activity_type, "reason");
    }

    #[tokio::test]
    async fn resume_after_tool_results_notifies_reason_task() {
        let shared = Arc::new(InMemoryWorkflowEventStore::new());
        let notifier = Arc::new(RecordingTaskNotifier::default());
        let session_id = SessionId::new();
        let saved_input = DurableTurnInput {
            org_id: 1,
            session_id,
            harness_id: HarnessId::new(),
            agent_id: Some(AgentId::new()),
            input_message_id: MessageId::new(),
            turn_id: None,
            previous_response_id: Some("resp_123".to_string()),
            iteration: 3,
            request_id: None,
            started_at: None,
            cumulative_usage: None,
            tool_call_count: 0,
            llm_call_count: 0,
            time_to_first_token_ms: None,
            final_message_id: None,
            final_answer_preview: None,
        };
        shared
            .create_workflow(
                session_id.uuid(),
                "turn_workflow",
                serde_json::to_value(&saved_input).expect("serialize input"),
                None,
            )
            .await
            .expect("create workflow");
        shared
            .update_workflow_status(
                session_id.uuid(),
                WorkflowStatus::Completed,
                Some(serde_json::to_value(&saved_input).expect("serialize result")),
                None,
            )
            .await
            .expect("mark completed");

        let runner = DurableRunner::new_with_shared_store(shared.clone())
            .with_task_notifier(notifier.clone());
        runner
            .resume_after_tool_results(session_id, Uuid::now_v7())
            .await
            .expect("resume should enqueue reason");

        assert_eq!(notifier.activity_types(), vec!["reason"]);
    }

    #[tokio::test]
    async fn cancellation_returns_structured_stop_reason() {
        let shared = Arc::new(InMemoryWorkflowEventStore::new());
        let session_id = SessionId::new();
        shared
            .create_workflow(
                session_id.uuid(),
                "turn_workflow",
                serde_json::json!({}),
                None,
            )
            .await
            .expect("create workflow");
        shared
            .enqueue_task(everruns_durable::TaskDefinition {
                workflow_id: Some(session_id.uuid()),
                activity_id: "reason-1".to_string(),
                activity_type: "reason".to_string(),
                input: serde_json::json!({}),
                options: Default::default(),
            })
            .await
            .expect("enqueue pending task");

        DurableRunner::new_with_shared_store(shared.clone())
            .cancel_run(session_id)
            .await
            .expect("cancel turn");

        let info = shared
            .get_workflow_info(session_id.uuid())
            .await
            .expect("load cancelled workflow");
        assert_eq!(info.status, WorkflowStatus::Cancelled);
        let output = info.result.expect("cancelled turn output");
        assert_eq!(output["success"], false);
        assert_eq!(output["stop_reason"], "cancelled");
        assert_eq!(output["error"], "User requested cancellation");

        shared
            .register_worker(everruns_durable::WorkerInfo::new(
                "worker-1",
                Vec::<String>::new(),
            ))
            .await
            .expect("register worker");

        let claimed = shared
            .claim_task("worker-1", &["reason".to_string()], 10)
            .await
            .expect("claim tasks after cancellation");
        assert!(claimed.is_empty(), "cancelled tasks must not be claimable");
    }
}
