// Durable execution engine runner
// Decision: Use custom PostgreSQL-backed durable engine for workflow orchestration
// Decision: AgentRunner interface for clean abstraction
// Decision: Workers communicate with control-plane via gRPC (no direct DB access)
// Decision: Control-plane uses direct database access (PostgresWorkflowEventStore)

use anyhow::Result;
use async_trait::async_trait;
use everruns_core::typed_id::{AgentId, HarnessId, MessageId, SessionId, TurnId};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;
use uuid::Uuid;

use crate::grpc_durable_store::{GrpcDurableStore, WorkflowStatus};
use crate::runner::AgentRunner;
use everruns_durable::{
    ActivityOptions, InMemoryWorkflowEventStore, PostgresWorkflowEventStore, WorkflowEvent,
    WorkflowEventStore,
};

// =============================================================================
// TurnWorkflow Input/Output
// =============================================================================

/// Input for the turn workflow
///
/// The turn_id is created by the input activity and propagated to subsequent
/// activities (reason, act) to ensure all events share the same turn context
/// for proper trace correlation in observability tools like Braintrust.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableTurnInput {
    pub org_id: i64,
    pub session_id: SessionId,
    pub harness_id: HarnessId,
    pub agent_id: Option<AgentId>,
    pub input_message_id: MessageId,
    /// Turn ID for trace correlation. Created by input activity, propagated to reason/act.
    /// Uses typed TurnId for type safety and consistent prefixed format.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    /// Previous LLM response ID for stateful continuation across reason iterations.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub previous_response_id: Option<String>,
    /// Current iteration number within this turn (1-based).
    /// Incremented each time a new reason activity is scheduled after act.
    #[serde(default = "default_iteration")]
    pub iteration: u32,
}

fn default_iteration() -> u32 {
    1
}

/// Output from the turn workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableTurnOutput {
    pub session_id: SessionId,
    pub success: bool,
    pub error: Option<String>,
}

// =============================================================================
// DurableStoreBackend Trait
// =============================================================================

/// Backend trait for durable store operations
/// Allows switching between gRPC (for workers) and direct DB (for control-plane)
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

    /// Append workflow events (for event sourcing)
    async fn append_events(
        &mut self,
        workflow_id: Uuid,
        expected_sequence: i32,
        events: Vec<WorkflowEvent>,
    ) -> Result<i32>;
}

// =============================================================================
// GrpcDurableStore Backend Implementation
// =============================================================================

#[async_trait]
impl DurableStoreBackend for GrpcDurableStore {
    async fn get_workflow_status(
        &mut self,
        workflow_id: Uuid,
    ) -> Result<(WorkflowStatus, Option<serde_json::Value>, Option<String>)> {
        GrpcDurableStore::get_workflow_status(self, workflow_id).await
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
        GrpcDurableStore::update_workflow_status(self, workflow_id, status, output, error).await
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

    async fn append_events(
        &mut self,
        _workflow_id: Uuid,
        _expected_sequence: i32,
        _events: Vec<WorkflowEvent>,
    ) -> Result<i32> {
        // gRPC mode doesn't support append_events directly yet
        // Events are appended by the control-plane
        Ok(0)
    }
}

// =============================================================================
// DirectDurableStore - wraps PostgresWorkflowEventStore for control-plane use
// =============================================================================

/// Direct database store for control-plane use
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

#[async_trait]
impl DurableStoreBackend for DirectDurableStore {
    async fn get_workflow_status(
        &mut self,
        workflow_id: Uuid,
    ) -> Result<(WorkflowStatus, Option<serde_json::Value>, Option<String>)> {
        let info = self.store.get_workflow_info(workflow_id).await?;
        Ok((
            durable_to_local_status(info.status),
            info.result,
            info.error.map(|e| format!("{:?}", e)),
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
        let durable_status = local_to_durable_status(status);
        let durable_error = error.map(everruns_durable::WorkflowError::new);
        self.store
            .update_workflow_status(workflow_id, durable_status, output, durable_error)
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
        let task = everruns_durable::TaskDefinition {
            workflow_id: Some(workflow_id),
            activity_id,
            activity_type,
            input,
            options: everruns_durable::ActivityOptions::default(),
        };
        self.store.enqueue_task(task).await.map_err(Into::into)
    }

    async fn count_active_workflows(&mut self) -> Result<usize> {
        self.store
            .count_active_workflows()
            .await
            .map(|c| c as usize)
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
}

// =============================================================================
// InMemoryDurableStore - wraps InMemoryWorkflowEventStore for dev mode
// =============================================================================

/// In-memory store for dev mode (no PostgreSQL required)
pub struct InMemoryDurableStore {
    store: Arc<InMemoryWorkflowEventStore>,
}

impl InMemoryDurableStore {
    pub fn new() -> Self {
        Self {
            store: Arc::new(InMemoryWorkflowEventStore::new()),
        }
    }

    /// Create from an existing shared store
    /// Used for DEV_MODE where the store is shared between runner and in-process worker
    pub fn from_shared(store: Arc<InMemoryWorkflowEventStore>) -> Self {
        Self { store }
    }

    /// Get a reference to the underlying store (for sharing between components)
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
impl DurableStoreBackend for InMemoryDurableStore {
    async fn get_workflow_status(
        &mut self,
        workflow_id: Uuid,
    ) -> Result<(WorkflowStatus, Option<serde_json::Value>, Option<String>)> {
        let info = self.store.get_workflow_info(workflow_id).await?;
        Ok((
            durable_to_local_status(info.status),
            info.result,
            info.error.map(|e| format!("{:?}", e)),
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
        let durable_status = local_to_durable_status(status);
        let durable_error = error.map(everruns_durable::WorkflowError::new);
        self.store
            .update_workflow_status(workflow_id, durable_status, output, durable_error)
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
        let task = everruns_durable::TaskDefinition {
            workflow_id: Some(workflow_id),
            activity_id,
            activity_type,
            input,
            options: everruns_durable::ActivityOptions::default(),
        };
        self.store.enqueue_task(task).await.map_err(Into::into)
    }

    async fn count_active_workflows(&mut self) -> Result<usize> {
        // In-memory store doesn't have count_active_workflows, return workflow count
        Ok(self.store.workflow_count())
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
}

fn durable_to_local_status(s: everruns_durable::WorkflowStatus) -> WorkflowStatus {
    match s {
        everruns_durable::WorkflowStatus::Pending => WorkflowStatus::Pending,
        everruns_durable::WorkflowStatus::Running => WorkflowStatus::Running,
        everruns_durable::WorkflowStatus::Completed => WorkflowStatus::Completed,
        everruns_durable::WorkflowStatus::Failed => WorkflowStatus::Failed,
        everruns_durable::WorkflowStatus::Cancelled => WorkflowStatus::Cancelled,
        everruns_durable::WorkflowStatus::ContinuedAsNew => WorkflowStatus::ContinuedAsNew,
    }
}

fn local_to_durable_status(s: WorkflowStatus) -> everruns_durable::WorkflowStatus {
    match s {
        WorkflowStatus::Pending => everruns_durable::WorkflowStatus::Pending,
        WorkflowStatus::Running => everruns_durable::WorkflowStatus::Running,
        WorkflowStatus::Completed => everruns_durable::WorkflowStatus::Completed,
        WorkflowStatus::Failed => everruns_durable::WorkflowStatus::Failed,
        WorkflowStatus::Cancelled => everruns_durable::WorkflowStatus::Cancelled,
        WorkflowStatus::ContinuedAsNew => everruns_durable::WorkflowStatus::ContinuedAsNew,
    }
}

// =============================================================================
// DurableRunner Implementation
// =============================================================================

/// Durable execution engine based runner
///
/// This runner uses the custom durable engine backed by PostgreSQL
/// for workflow orchestration.
/// - Workers communicate with the control-plane via gRPC
/// - Control-plane uses direct database access
pub struct DurableRunner {
    store: Arc<Mutex<dyn DurableStoreBackend>>,
}

impl DurableRunner {
    /// Create a new durable runner connected to control-plane gRPC
    /// Used by workers that connect to the control-plane
    pub async fn new(grpc_address: &str) -> Result<Self> {
        info!(
            grpc_address = %grpc_address,
            "Initializing Durable execution engine runner (gRPC mode)"
        );

        let store = GrpcDurableStore::connect(grpc_address).await?;

        info!("Durable runner initialized");

        Ok(Self {
            store: Arc::new(Mutex::new(store)),
        })
    }

    /// Create a new durable runner with direct database access
    /// Used by the control-plane which has direct database access
    pub fn new_with_pool(pool: sqlx::PgPool) -> Self {
        info!("Initializing Durable execution engine runner (direct DB mode)");

        let store = DirectDurableStore::new(pool);

        Self {
            store: Arc::new(Mutex::new(store)),
        }
    }

    /// Create a new durable runner with in-memory storage
    /// Used by the control-plane in dev mode (no PostgreSQL required)
    pub fn new_in_memory() -> Self {
        info!("Initializing Durable execution engine runner (in-memory dev mode)");

        let store = InMemoryDurableStore::new();

        Self {
            store: Arc::new(Mutex::new(store)),
        }
    }

    /// Create a new durable runner with a shared in-memory store
    /// Used by the control-plane in DEV_MODE where the store is shared
    /// between the runner and in-process worker
    pub fn new_with_shared_store(shared_store: Arc<InMemoryWorkflowEventStore>) -> Self {
        info!("Initializing Durable execution engine runner (shared in-memory dev mode)");

        let store = InMemoryDurableStore::from_shared(shared_store);

        Self {
            store: Arc::new(Mutex::new(store)),
        }
    }

    /// Create from WORKER_GRPC_ADDRESS environment variable (defaults to 127.0.0.1:9001)
    /// Used by workers
    pub async fn from_env() -> Result<Self> {
        let grpc_address =
            std::env::var("WORKER_GRPC_ADDRESS").unwrap_or_else(|_| "127.0.0.1:9001".to_string());

        Self::new(&grpc_address).await
    }
}

#[async_trait]
impl AgentRunner for DurableRunner {
    /// Start a turn workflow for the given session
    async fn start_run(
        &self,
        org_id: i64,
        session_id: SessionId,
        harness_id: HarnessId,
        agent_id: Option<AgentId>,
        input_message_id: MessageId,
    ) -> Result<()> {
        info!(
            org_id = org_id,
            session_id = %session_id,
            harness_id = %harness_id,
            ?agent_id,
            input_message_id = %input_message_id,
            "Starting durable turn workflow for session"
        );

        // Build workflow input
        // turn_id is None at workflow start - will be created by input activity
        let input = DurableTurnInput {
            org_id,
            session_id,
            harness_id,
            agent_id,
            input_message_id,
            turn_id: None,
            previous_response_id: None,
            iteration: 1,
        };

        // Create workflow instance
        // Use session_id as workflow_id for consistency
        let workflow_id = session_id.uuid();
        let input_json = serde_json::to_value(&input)?;

        // Check if workflow already exists and wait for it to complete if needed.
        // Race condition: session becomes idle (in execute_reason_activity) BEFORE
        // schedule_next_activity marks the workflow as Completed. If a new message
        // arrives during this window, the workflow is still non-terminal. We wait
        // briefly instead of silently dropping the new turn.
        let workflow_exists = {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                let mut store = self.store.lock().await;
                match store.get_workflow_status(workflow_id).await {
                    Ok((status, _, _)) => {
                        if status.is_terminal() {
                            info!(
                                session_id = %session_id,
                                workflow_id = %workflow_id,
                                status = ?status,
                                "Existing workflow is terminal, resetting for new turn"
                            );
                            break true;
                        }
                        // Workflow is non-terminal — release lock and wait
                        drop(store);
                        if std::time::Instant::now() > deadline {
                            return Err(anyhow::anyhow!(
                                "Timeout waiting for previous workflow to complete for session {}",
                                session_id
                            ));
                        }
                        info!(
                            session_id = %session_id,
                            workflow_id = %workflow_id,
                            status = ?status,
                            "Workflow still running, waiting for completion before starting new turn"
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                    Err(e) => {
                        let err_str = e.to_string();
                        if err_str.contains("not found") || err_str.contains("NOT_FOUND") {
                            break false;
                        }
                        return Err(anyhow::anyhow!("Failed to check workflow status: {}", e));
                    }
                }
            }
        };

        let mut store = self.store.lock().await;

        // Enqueue the initial activity (input processing)
        let activity_id = format!("input_{}", Uuid::now_v7());

        if workflow_exists {
            // Reset existing workflow to pending for new turn
            store
                .update_workflow_status(workflow_id, WorkflowStatus::Pending, None, None)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to reset workflow status: {}", e))?;

            // Enqueue task for the new turn (skip appending events for reset workflows
            // as the task input contains all necessary information)
            store
                .enqueue_task(
                    workflow_id,
                    activity_id,
                    "process_input".to_string(),
                    input_json,
                )
                .await
                .map_err(|e| anyhow::anyhow!("Failed to enqueue task: {}", e))?;
        } else {
            // Create new workflow
            store
                .create_workflow(workflow_id, "turn_workflow", input_json.clone())
                .await
                .map_err(|e| anyhow::anyhow!("Failed to create workflow: {}", e))?;

            // Append WorkflowStarted event
            let started_event = WorkflowEvent::WorkflowStarted {
                input: input_json.clone(),
            };
            let sequence = store
                .append_events(workflow_id, 0, vec![started_event])
                .await
                .map_err(|e| anyhow::anyhow!("Failed to append WorkflowStarted event: {}", e))?;

            // Enqueue task
            store
                .enqueue_task(
                    workflow_id,
                    activity_id.clone(),
                    "process_input".to_string(),
                    input_json.clone(),
                )
                .await
                .map_err(|e| anyhow::anyhow!("Failed to enqueue task: {}", e))?;

            // Append ActivityScheduled event
            let scheduled_event = WorkflowEvent::ActivityScheduled {
                activity_id,
                activity_type: "process_input".to_string(),
                input: input_json,
                options: ActivityOptions::default(),
            };
            let _ = store
                .append_events(workflow_id, sequence, vec![scheduled_event])
                .await
                .map_err(|e| anyhow::anyhow!("Failed to append ActivityScheduled event: {}", e))?;
        }

        info!(
            session_id = %session_id,
            workflow_id = %workflow_id,
            "Durable workflow created and input task enqueued"
        );

        Ok(())
    }

    async fn cancel_run(&self, session_id: SessionId) -> Result<()> {
        info!(session_id = %session_id, "Cancelling durable workflow");

        let workflow_id = session_id.uuid();
        let mut store = self.store.lock().await;

        // Update workflow status to cancelled
        store
            .update_workflow_status(
                workflow_id,
                WorkflowStatus::Cancelled,
                None,
                Some("User requested cancellation".to_string()),
            )
            .await
            .map_err(|e| anyhow::anyhow!("Failed to cancel workflow: {}", e))?;

        info!(
            session_id = %session_id,
            workflow_id = %workflow_id,
            "Workflow cancelled"
        );

        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::typed_id::{AgentId, HarnessId, MessageId, SessionId};

    #[test]
    fn test_durable_turn_input_serialization() {
        use everruns_core::DEFAULT_ORG_ID;

        let input = DurableTurnInput {
            org_id: DEFAULT_ORG_ID,
            session_id: SessionId::new(),
            harness_id: HarnessId::new(),
            agent_id: Some(AgentId::new()),
            input_message_id: MessageId::new(),
            turn_id: None,
            previous_response_id: None,
            iteration: 1,
        };

        let json = serde_json::to_string(&input).unwrap();
        let parsed: DurableTurnInput = serde_json::from_str(&json).unwrap();

        assert_eq!(input.org_id, parsed.org_id);
        assert_eq!(input.session_id, parsed.session_id);
        assert_eq!(input.agent_id, parsed.agent_id);
        assert_eq!(input.input_message_id, parsed.input_message_id);
        assert_eq!(input.turn_id, parsed.turn_id);
    }

    /// Test that start_run works for a new session (first message)
    #[tokio::test]
    async fn test_start_run_creates_new_workflow() {
        use everruns_core::DEFAULT_ORG_ID;

        let runner = DurableRunner::new_in_memory();

        let session_id = SessionId::new();
        let harness_id = HarnessId::new();
        let agent_id = AgentId::new();
        let message_id = MessageId::new();

        // First call should create a new workflow
        runner
            .start_run(
                DEFAULT_ORG_ID,
                session_id,
                harness_id,
                Some(agent_id),
                message_id,
            )
            .await
            .expect("First start_run should succeed");

        // Workflow should be running
        assert!(runner.is_running(session_id).await);
    }

    /// Test that start_run waits for the prior workflow to finish before reusing it.
    #[tokio::test]
    async fn test_start_run_waits_for_running_workflow_to_finish() {
        use everruns_core::DEFAULT_ORG_ID;

        let runner = DurableRunner::new_in_memory();

        let session_id = SessionId::new();
        let harness_id = HarnessId::new();
        let agent_id = AgentId::new();
        let message_id1 = MessageId::new();
        let message_id2 = MessageId::new();

        // First message
        runner
            .start_run(
                DEFAULT_ORG_ID,
                session_id,
                harness_id,
                Some(agent_id),
                message_id1,
            )
            .await
            .expect("First start_run should succeed");

        assert!(runner.is_running(session_id).await);

        let store = runner.store.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            let mut store = store.lock().await;
            store
                .update_workflow_status(session_id.uuid(), WorkflowStatus::Completed, None, None)
                .await
                .expect("Should complete existing workflow");
        });

        // Second message while still running - should wait, then reuse the workflow.
        let start = std::time::Instant::now();
        runner
            .start_run(
                DEFAULT_ORG_ID,
                session_id,
                harness_id,
                Some(agent_id),
                message_id2,
            )
            .await
            .expect("Second start_run should wait and then succeed");

        assert!(
            start.elapsed() >= std::time::Duration::from_millis(200),
            "Second start_run should wait for the running workflow to finish"
        );
        assert!(
            runner.is_running(session_id).await,
            "Workflow should be running again after reuse"
        );
    }

    /// Test that start_run resets a completed workflow for a second message
    ///
    /// This is the key test for the second message bug fix:
    /// When a workflow is completed and a new message arrives, the workflow
    /// should be reset to pending so it can process the new message.
    #[tokio::test]
    async fn test_start_run_resets_completed_workflow() {
        use everruns_core::DEFAULT_ORG_ID;

        let runner = DurableRunner::new_in_memory();

        let session_id = SessionId::new();
        let harness_id = HarnessId::new();
        let agent_id = AgentId::new();
        let message_id1 = MessageId::new();
        let message_id2 = MessageId::new();

        // First message - creates workflow
        runner
            .start_run(
                DEFAULT_ORG_ID,
                session_id,
                harness_id,
                Some(agent_id),
                message_id1,
            )
            .await
            .expect("First start_run should succeed");

        assert!(runner.is_running(session_id).await);

        // Simulate workflow completion by updating status to Completed
        {
            let mut store = runner.store.lock().await;
            store
                .update_workflow_status(session_id.uuid(), WorkflowStatus::Completed, None, None)
                .await
                .expect("Should update workflow status");
        }

        // Workflow should now be terminal (not running)
        assert!(
            !runner.is_running(session_id).await,
            "Workflow should be completed"
        );

        // Second message - should reset and create new turn
        // This is the bug fix - previously this would fail because it tried
        // to create a workflow with the same ID
        runner
            .start_run(
                DEFAULT_ORG_ID,
                session_id,
                harness_id,
                Some(agent_id),
                message_id2,
            )
            .await
            .expect("Second start_run should succeed (reset workflow)");

        // Workflow should be running again (reset to pending, new task enqueued)
        assert!(
            runner.is_running(session_id).await,
            "Workflow should be running again after reset"
        );
    }

    /// Test that start_run resets a failed workflow for retry
    #[tokio::test]
    async fn test_start_run_resets_failed_workflow() {
        use everruns_core::DEFAULT_ORG_ID;

        let runner = DurableRunner::new_in_memory();

        let session_id = SessionId::new();
        let harness_id = HarnessId::new();
        let agent_id = AgentId::new();
        let message_id1 = MessageId::new();
        let message_id2 = MessageId::new();

        // First message
        runner
            .start_run(
                DEFAULT_ORG_ID,
                session_id,
                harness_id,
                Some(agent_id),
                message_id1,
            )
            .await
            .expect("First start_run should succeed");

        // Simulate workflow failure
        {
            let mut store = runner.store.lock().await;
            store
                .update_workflow_status(
                    session_id.uuid(),
                    WorkflowStatus::Failed,
                    None,
                    Some("Test error".to_string()),
                )
                .await
                .expect("Should update workflow status");
        }

        assert!(!runner.is_running(session_id).await);

        // Second message - should reset failed workflow
        runner
            .start_run(
                DEFAULT_ORG_ID,
                session_id,
                harness_id,
                Some(agent_id),
                message_id2,
            )
            .await
            .expect("Second start_run should reset failed workflow");

        assert!(runner.is_running(session_id).await);
    }
}
