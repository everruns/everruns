// Durable execution engine runner adapters.
// Decision: everruns-worker owns durable orchestration and maps runtime turn state onto the durable engine.
// Decision: everruns-runtime remains durable-agnostic and only exports generic turn strategy/state.

use anyhow::Result;
use async_trait::async_trait;
use everruns_core::typed_id::{AgentId, HarnessId, MessageId, SessionId};
use everruns_durable::{
    InMemoryWorkflowEventStore, PostgresWorkflowEventStore, WorkflowEvent, WorkflowEventStore,
    WorkflowSignal, WorkflowStatus,
};
pub use everruns_runtime::RuntimeTurnState as DurableTurnInput;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;
use uuid::Uuid;

use crate::grpc_durable_store::{GrpcDurableStore, WorkflowStatus as GrpcWorkflowStatus};
use crate::runner::AgentRunner;

/// Output metadata for durable turns.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DurableTurnOutput {
    pub session_id: SessionId,
    pub success: bool,
    pub error: Option<String>,
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
}

/// Direct database store for control-plane use.
pub struct DirectDurableStore {
    store: PostgresWorkflowEventStore,
}

impl DirectDurableStore {
    pub fn new(pool: sqlx::PgPool) -> Self {
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
impl DurableStoreBackend for GrpcDurableStore {
    async fn get_workflow_status(
        &mut self,
        workflow_id: Uuid,
    ) -> Result<(WorkflowStatus, Option<serde_json::Value>, Option<String>)> {
        let (status, output, error) =
            GrpcDurableStore::get_workflow_status(self, workflow_id).await?;
        Ok((grpc_to_runtime_status(status), output, error))
    }

    async fn create_workflow(
        &mut self,
        workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
    ) -> Result<Uuid> {
        GrpcDurableStore::create_workflow(self, workflow_id, workflow_type, input).await
    }

    async fn update_workflow_status(
        &mut self,
        workflow_id: Uuid,
        status: WorkflowStatus,
        output: Option<serde_json::Value>,
        error: Option<String>,
    ) -> Result<()> {
        GrpcDurableStore::update_workflow_status(
            self,
            workflow_id,
            runtime_to_grpc_status(status),
            output,
            error,
        )
        .await
    }

    async fn enqueue_task(
        &mut self,
        workflow_id: Uuid,
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
    ) -> Result<Uuid> {
        GrpcDurableStore::enqueue_task(self, workflow_id, activity_id, activity_type, input).await
    }

    async fn count_active_workflows(&mut self) -> Result<usize> {
        GrpcDurableStore::count_active_workflows(self).await
    }

    async fn cancel_pending_tasks(&mut self, _workflow_id: Uuid) -> Result<u64> {
        Ok(0)
    }

    async fn append_events(
        &mut self,
        _workflow_id: Uuid,
        _expected_sequence: i32,
        _events: Vec<WorkflowEvent>,
    ) -> Result<i32> {
        Ok(0)
    }

    async fn try_claim_workflow_for_new_turn(&mut self, _workflow_id: Uuid) -> Result<bool> {
        Ok(false)
    }

    async fn send_signal(&mut self, workflow_id: Uuid, signal: WorkflowSignal) -> Result<()> {
        GrpcDurableStore::send_signal(self, workflow_id, signal).await
    }

    async fn get_and_consume_signals(&mut self, workflow_id: Uuid) -> Result<Vec<WorkflowSignal>> {
        GrpcDurableStore::get_and_consume_signals(self, workflow_id).await
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
                activity_id,
                activity_type,
                input,
                options: Default::default(),
            })
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
                activity_id,
                activity_type,
                input,
                options: Default::default(),
            })
            .await
            .map_err(Into::into)
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

/// Durable execution engine based runner.
///
/// This runner maps runtime turn state onto the durable engine.
pub struct DurableRunner {
    store: Arc<Mutex<dyn DurableStoreBackend>>,
}

impl DurableRunner {
    pub async fn new(grpc_address: &str) -> Result<Self> {
        info!(
            grpc_address = %grpc_address,
            "Initializing durable runner (gRPC mode)"
        );

        let store = GrpcDurableStore::connect(grpc_address).await?;
        Ok(Self {
            store: Arc::new(Mutex::new(store)),
        })
    }

    pub fn new_with_pool(pool: sqlx::PgPool) -> Self {
        info!("Initializing durable runner (direct DB mode)");
        let store = DirectDurableStore::new(pool);
        Self {
            store: Arc::new(Mutex::new(store)),
        }
    }

    pub fn new_in_memory() -> Self {
        info!("Initializing durable runner (in-memory dev mode)");
        let store = InMemoryDurableStore::new();
        Self {
            store: Arc::new(Mutex::new(store)),
        }
    }

    pub fn new_with_shared_store(shared_store: Arc<InMemoryWorkflowEventStore>) -> Self {
        info!("Initializing durable runner (shared in-memory dev mode)");
        let store = InMemoryDurableStore::from_shared(shared_store);
        Self {
            store: Arc::new(Mutex::new(store)),
        }
    }

    pub async fn from_env() -> Result<Self> {
        let grpc_address =
            std::env::var("WORKER_GRPC_ADDRESS").unwrap_or_else(|_| "127.0.0.1:9001".to_string());
        Self::new(&grpc_address).await
    }
}

#[async_trait]
impl AgentRunner for DurableRunner {
    async fn start_run(
        &self,
        org_id: i64,
        session_id: SessionId,
        harness_id: HarnessId,
        agent_id: Option<AgentId>,
        input_message_id: MessageId,
        request_id: Option<String>,
    ) -> Result<()> {
        info!(
            org_id,
            session_id = %session_id,
            harness_id = %harness_id,
            ?agent_id,
            input_message_id = %input_message_id,
            "starting durable turn workflow"
        );

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
        };
        let workflow_id = session_id.uuid();
        let input_json = serde_json::to_value(&input)?;
        let mut store = self.store.lock().await;

        match store.try_claim_workflow_for_new_turn(workflow_id).await {
            Ok(true) => {
                if let Err(error) = store
                    .enqueue_task(
                        workflow_id,
                        format!("input_{}", Uuid::now_v7()),
                        "process_input".to_string(),
                        input_json,
                    )
                    .await
                {
                    let _ = store
                        .update_workflow_status(workflow_id, WorkflowStatus::Completed, None, None)
                        .await;
                    return Err(anyhow::anyhow!("Failed to enqueue task: {error}"));
                }
            }
            Ok(false) => {
                let signal = WorkflowSignal::new(
                    everruns_durable::signal_types::USER_MESSAGE,
                    serde_json::json!({
                        "input_message_id": input_message_id.to_string(),
                        "org_id": org_id,
                        "harness_id": harness_id.to_string(),
                        "agent_id": agent_id.map(|id| id.to_string()),
                    }),
                );
                if let Err(error) = store.send_signal(workflow_id, signal).await {
                    tracing::warn!(
                        session_id = %session_id,
                        %error,
                        "failed to send steering signal"
                    );
                }
                return Ok(());
            }
            Err(error) => {
                let err = error.to_string();
                if !err.contains("not found") && !err.contains("NOT_FOUND") {
                    return Err(anyhow::anyhow!("Failed to check workflow status: {error}"));
                }

                store
                    .create_workflow(workflow_id, "turn_workflow", input_json.clone())
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to create workflow: {e}"))?;

                let activity_id = format!("input_{}", Uuid::now_v7());
                let sequence = store
                    .append_events(
                        workflow_id,
                        0,
                        vec![WorkflowEvent::WorkflowStarted {
                            input: input_json.clone(),
                        }],
                    )
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to append WorkflowStarted event: {e}"))?;

                store
                    .enqueue_task(
                        workflow_id,
                        activity_id.clone(),
                        "process_input".to_string(),
                        input_json.clone(),
                    )
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to enqueue task: {e}"))?;

                store
                    .update_workflow_status(workflow_id, WorkflowStatus::Running, None, None)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to set new workflow to Running: {e}"))?;

                let _ = store
                    .append_events(
                        workflow_id,
                        sequence,
                        vec![WorkflowEvent::ActivityScheduled {
                            activity_id,
                            activity_type: "process_input".to_string(),
                            input: input_json,
                            options: everruns_durable::ActivityOptions::default(),
                        }],
                    )
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!("Failed to append ActivityScheduled event: {e}")
                    })?;
            }
        }

        Ok(())
    }

    async fn resume_after_tool_results(&self, session_id: SessionId) -> Result<()> {
        let workflow_id = session_id.uuid();
        let mut store = self.store.lock().await;

        let (status, result_json, _) = store
            .get_workflow_status(workflow_id)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get workflow status: {e}"))?;
        if status != WorkflowStatus::Completed {
            return Err(anyhow::anyhow!(
                "Cannot resume: workflow {workflow_id} is not in Completed status (status: {status:?})"
            ));
        }

        let saved_value = result_json.ok_or_else(|| {
            anyhow::anyhow!(
                "Cannot resume: workflow {workflow_id} has no saved turn input in result"
            )
        })?;
        let turn_input: DurableTurnInput = serde_json::from_value(saved_value)
            .map_err(|e| anyhow::anyhow!("Failed to parse saved turn input: {e}"))?;

        let input_json = serde_json::to_value(&turn_input)?;
        store
            .update_workflow_status(workflow_id, WorkflowStatus::Pending, None, None)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to reset workflow status: {e}"))?;

        if let Err(error) = store
            .enqueue_task(
                workflow_id,
                format!("reason_{}", Uuid::now_v7()),
                "reason".to_string(),
                input_json,
            )
            .await
        {
            let _ = store
                .update_workflow_status(
                    workflow_id,
                    WorkflowStatus::Completed,
                    Some(serde_json::to_value(&turn_input).unwrap_or_default()),
                    None,
                )
                .await;
            return Err(anyhow::anyhow!("Failed to enqueue reason task: {error}"));
        }

        Ok(())
    }

    async fn cancel_run(&self, session_id: SessionId) -> Result<()> {
        let workflow_id = session_id.uuid();
        let mut store = self.store.lock().await;
        store
            .update_workflow_status(
                workflow_id,
                WorkflowStatus::Cancelled,
                None,
                Some("User requested cancellation".to_string()),
            )
            .await
            .map_err(|e| anyhow::anyhow!("Failed to cancel workflow: {e}"))
    }

    async fn is_running(&self, session_id: SessionId) -> bool {
        let workflow_id = session_id.uuid();
        let mut store = self.store.lock().await;
        match store.get_workflow_status(workflow_id).await {
            Ok((status, _, _)) => !status.is_terminal(),
            Err(_) => false,
        }
    }

    async fn active_count(&self) -> usize {
        let mut store = self.store.lock().await;
        store.count_active_workflows().await.unwrap_or_default()
    }
}

fn grpc_to_runtime_status(status: GrpcWorkflowStatus) -> WorkflowStatus {
    match status {
        GrpcWorkflowStatus::Pending => WorkflowStatus::Pending,
        GrpcWorkflowStatus::Running => WorkflowStatus::Running,
        GrpcWorkflowStatus::Completed => WorkflowStatus::Completed,
        GrpcWorkflowStatus::Failed => WorkflowStatus::Failed,
        GrpcWorkflowStatus::Cancelled => WorkflowStatus::Cancelled,
        GrpcWorkflowStatus::ContinuedAsNew => WorkflowStatus::ContinuedAsNew,
    }
}

fn runtime_to_grpc_status(status: WorkflowStatus) -> GrpcWorkflowStatus {
    match status {
        WorkflowStatus::Pending => GrpcWorkflowStatus::Pending,
        WorkflowStatus::Running => GrpcWorkflowStatus::Running,
        WorkflowStatus::Completed => GrpcWorkflowStatus::Completed,
        WorkflowStatus::Failed => GrpcWorkflowStatus::Failed,
        WorkflowStatus::Cancelled => GrpcWorkflowStatus::Cancelled,
        WorkflowStatus::ContinuedAsNew => GrpcWorkflowStatus::ContinuedAsNew,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let claimed = shared
            .claim_task("worker-1", &["process_input".to_string()], 10)
            .await
            .expect("task should be claimable");
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].activity_type, "process_input");
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
        assert_eq!(
            signals[0].signal_type,
            everruns_durable::signal_types::USER_MESSAGE
        );
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
            .enqueue_task(
                everruns_durable::TaskDefinition {
                    workflow_id: Some(workflow_id),
                    activity_id: format!("input_{}", Uuid::now_v7()),
                    activity_type: "process_input".to_string(),
                    input: serde_json::json!({}),
                    options: Default::default(),
                },
            )
            .await
            .expect("enqueue task");
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
        assert_eq!(
            signals[0].signal_type,
            everruns_durable::signal_types::USER_MESSAGE
        );

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
        runner
            .resume_after_tool_results(session_id)
            .await
            .expect("resume should enqueue reason");

        assert_eq!(
            shared
                .get_workflow_status(session_id.uuid())
                .await
                .expect("status should load"),
            WorkflowStatus::Pending
        );

        let claimed = shared
            .claim_task("worker-1", &["reason".to_string()], 10)
            .await
            .expect("reason task should be claimable");
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].activity_type, "reason");
    }
}
