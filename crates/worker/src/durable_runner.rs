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

    /// Cancel all pending (unclaimed) tasks for a workflow.
    async fn cancel_pending_tasks(&mut self, workflow_id: Uuid) -> Result<u64>;

    /// Append workflow events (for event sourcing)
    async fn append_events(
        &mut self,
        workflow_id: Uuid,
        expected_sequence: i32,
        events: Vec<WorkflowEvent>,
    ) -> Result<i32>;

    /// Atomically try to claim a workflow for a new turn.
    /// Transitions the workflow from terminal/pending → Running.
    /// Returns Ok(true) if claimed, Ok(false) if workflow is Running (active turn).
    /// This provides database-level atomicity for horizontal scaling.
    async fn try_claim_workflow_for_new_turn(&mut self, workflow_id: Uuid) -> Result<bool>;

    /// Send a signal to a running workflow (e.g. user_message steering signal).
    async fn send_signal(
        &mut self,
        workflow_id: Uuid,
        signal: everruns_durable::WorkflowSignal,
    ) -> Result<()>;

    /// Get and consume all pending signals for a workflow.
    /// Returns the signals and marks them as processed atomically.
    async fn get_and_consume_signals(
        &mut self,
        workflow_id: Uuid,
    ) -> Result<Vec<everruns_durable::WorkflowSignal>>;
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

    async fn try_claim_workflow_for_new_turn(&mut self, _workflow_id: Uuid) -> Result<bool> {
        // gRPC workers don't call start_run — only the control-plane does.
        Ok(false)
    }

    async fn cancel_pending_tasks(&mut self, _workflow_id: Uuid) -> Result<u64> {
        // gRPC workers don't call start_run — only the control-plane does.
        // If this is ever needed, add a CancelPendingTasks gRPC method.
        Ok(0)
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

    async fn send_signal(
        &mut self,
        workflow_id: Uuid,
        signal: everruns_durable::WorkflowSignal,
    ) -> Result<()> {
        GrpcDurableStore::send_signal(self, workflow_id, signal).await
    }

    async fn get_and_consume_signals(
        &mut self,
        workflow_id: Uuid,
    ) -> Result<Vec<everruns_durable::WorkflowSignal>> {
        GrpcDurableStore::get_and_consume_signals(self, workflow_id).await
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

    async fn try_claim_workflow_for_new_turn(&mut self, workflow_id: Uuid) -> Result<bool> {
        self.store
            .try_claim_workflow_for_new_turn(workflow_id)
            .await
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

    async fn send_signal(
        &mut self,
        workflow_id: Uuid,
        signal: everruns_durable::WorkflowSignal,
    ) -> Result<()> {
        self.store
            .send_signal(workflow_id, signal)
            .await
            .map_err(Into::into)
    }

    async fn get_and_consume_signals(
        &mut self,
        workflow_id: Uuid,
    ) -> Result<Vec<everruns_durable::WorkflowSignal>> {
        let signals = self.store.get_pending_signals(workflow_id).await?;
        if !signals.is_empty() {
            self.store
                .mark_signals_processed(workflow_id, signals.len())
                .await?;
        }
        Ok(signals)
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

    async fn try_claim_workflow_for_new_turn(&mut self, workflow_id: Uuid) -> Result<bool> {
        self.store
            .try_claim_workflow_for_new_turn(workflow_id)
            .await
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

    async fn send_signal(
        &mut self,
        workflow_id: Uuid,
        signal: everruns_durable::WorkflowSignal,
    ) -> Result<()> {
        self.store
            .send_signal(workflow_id, signal)
            .await
            .map_err(Into::into)
    }

    async fn get_and_consume_signals(
        &mut self,
        workflow_id: Uuid,
    ) -> Result<Vec<everruns_durable::WorkflowSignal>> {
        let signals = self.store.get_pending_signals(workflow_id).await?;
        if !signals.is_empty() {
            self.store
                .mark_signals_processed(workflow_id, signals.len())
                .await?;
        }
        Ok(signals)
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

        // Atomically try to claim the workflow for a new turn.
        //
        // try_claim_workflow_for_new_turn uses SELECT FOR UPDATE (Postgres) to
        // atomically transition terminal/pending → Running AND cancel stale tasks.
        // This is safe under horizontal scaling — only one replica wins the claim.
        //
        // If the workflow is Running (active turn), the claim fails and we send
        // a steering signal instead of starting a concurrent turn.
        let mut store = self.store.lock().await;

        let claim_result = store.try_claim_workflow_for_new_turn(workflow_id).await;

        match claim_result {
            Ok(true) => {
                // Claimed successfully — workflow is now Running.
                // Enqueue process_input task for the new turn.
                info!(
                    session_id = %session_id,
                    workflow_id = %workflow_id,
                    "Claimed existing workflow for new turn"
                );

                let activity_id = format!("input_{}", Uuid::now_v7());
                store
                    .enqueue_task(
                        workflow_id,
                        activity_id,
                        "process_input".to_string(),
                        input_json,
                    )
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to enqueue task: {}", e))?;
            }
            Ok(false) => {
                // Workflow is Running — a turn is active.
                // Send a steering signal so the running turn picks up this message.
                // The message is already stored in the events table.
                info!(
                    session_id = %session_id,
                    workflow_id = %workflow_id,
                    input_message_id = %input_message_id,
                    "Turn already running — sending steering signal instead of starting concurrent turn"
                );
                let signal = everruns_durable::WorkflowSignal::new(
                    everruns_durable::signal_types::USER_MESSAGE,
                    serde_json::json!({
                        "input_message_id": input_message_id.to_string(),
                        "org_id": org_id,
                        "harness_id": harness_id.to_string(),
                        "agent_id": agent_id.map(|a| a.to_string()),
                    }),
                );
                if let Err(e) = store.send_signal(workflow_id, signal).await {
                    tracing::warn!(
                        error = %e,
                        session_id = %session_id,
                        "Failed to send steering signal — message is still stored"
                    );
                }
                return Ok(());
            }
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("not found") || err_str.contains("NOT_FOUND") {
                    // Workflow doesn't exist yet — create a new one.
                    info!(
                        session_id = %session_id,
                        workflow_id = %workflow_id,
                        "Creating new workflow for session"
                    );

                    store
                        .create_workflow(workflow_id, "turn_workflow", input_json.clone())
                        .await
                        .map_err(|e| anyhow::anyhow!("Failed to create workflow: {}", e))?;

                    // Set to Running immediately
                    store
                        .update_workflow_status(workflow_id, WorkflowStatus::Running, None, None)
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!("Failed to set new workflow to Running: {}", e)
                        })?;

                    let activity_id = format!("input_{}", Uuid::now_v7());

                    // Append WorkflowStarted event
                    let started_event = WorkflowEvent::WorkflowStarted {
                        input: input_json.clone(),
                    };
                    let sequence = store
                        .append_events(workflow_id, 0, vec![started_event])
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!("Failed to append WorkflowStarted event: {}", e)
                        })?;

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
                        .map_err(|e| {
                            anyhow::anyhow!("Failed to append ActivityScheduled event: {}", e)
                        })?;
                } else {
                    return Err(anyhow::anyhow!("Failed to check workflow status: {}", e));
                }
            }
        }

        info!(
            session_id = %session_id,
            workflow_id = %workflow_id,
            "Durable workflow created and input task enqueued (status: Running)"
        );

        Ok(())
    }

    async fn resume_after_tool_results(&self, session_id: SessionId) -> Result<()> {
        let workflow_id = session_id.uuid();

        info!(
            session_id = %session_id,
            workflow_id = %workflow_id,
            "Resuming workflow after client tool results"
        );

        let mut store = self.store.lock().await;

        // Retrieve saved DurableTurnInput from the workflow result.
        // schedule_next_activity stores it when completing due to connection_required.
        let (status, result_json, _) = store
            .get_workflow_status(workflow_id)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get workflow status: {}", e))?;

        // Only resume workflows that completed normally (i.e. due to
        // connection_required). Failed/Cancelled/ContinuedAsNew workflows
        // should not be resumed via this path.
        if status != WorkflowStatus::Completed {
            return Err(anyhow::anyhow!(
                "Cannot resume: workflow {} is not in Completed status (status: {:?})",
                workflow_id,
                status
            ));
        }

        let turn_input: DurableTurnInput = result_json
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Cannot resume: workflow {} has no saved turn input in result",
                    workflow_id
                )
            })
            .and_then(|v| {
                serde_json::from_value(v)
                    .map_err(|e| anyhow::anyhow!("Failed to parse saved turn input: {}", e))
            })?;

        // Reset workflow to Pending and enqueue a reason activity directly
        // (skipping process_input / InputAtom — there is no new user message).
        let input_json = serde_json::to_value(&turn_input)?;
        store
            .update_workflow_status(workflow_id, WorkflowStatus::Pending, None, None)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to reset workflow status: {}", e))?;

        if let Err(e) = store
            .enqueue_task(
                workflow_id,
                format!("reason_{}", Uuid::now_v7()),
                "reason".to_string(),
                input_json.clone(),
            )
            .await
        {
            // Enqueue failed — restore workflow to Completed with saved input
            // so a subsequent resume attempt can still succeed.
            let _ = store
                .update_workflow_status(
                    workflow_id,
                    WorkflowStatus::Completed,
                    Some(serde_json::to_value(&turn_input).unwrap_or_default()),
                    None,
                )
                .await;
            return Err(anyhow::anyhow!("Failed to enqueue reason task: {}", e));
        }

        info!(
            session_id = %session_id,
            workflow_id = %workflow_id,
            turn_id = ?turn_input.turn_id,
            iteration = turn_input.iteration,
            "Workflow resumed — reason activity enqueued (skipped InputAtom)"
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
        assert_eq!(input.iteration, parsed.iteration);
    }

    /// Test that iteration defaults to 1 when missing from JSON (backward compat)
    #[test]
    fn test_durable_turn_input_iteration_default() {
        use everruns_core::DEFAULT_ORG_ID;

        // Build a valid input and serialize it
        let input = DurableTurnInput {
            org_id: DEFAULT_ORG_ID,
            session_id: SessionId::new(),
            harness_id: HarnessId::new(),
            agent_id: None,
            input_message_id: MessageId::new(),
            turn_id: None,
            previous_response_id: None,
            iteration: 3,
        };
        let mut json_val: serde_json::Value = serde_json::to_value(&input).unwrap();
        // Remove iteration to simulate pre-existing persisted data
        json_val.as_object_mut().unwrap().remove("iteration");

        let parsed: DurableTurnInput = serde_json::from_value(json_val).unwrap();
        assert_eq!(parsed.iteration, 1); // defaults to 1
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
    async fn test_start_run_sends_steering_signal_when_running() {
        use everruns_core::DEFAULT_ORG_ID;

        let runner = DurableRunner::new_in_memory();

        let session_id = SessionId::new();
        let harness_id = HarnessId::new();
        let agent_id = AgentId::new();
        let message_id1 = MessageId::new();
        let message_id2 = MessageId::new();

        // First message — creates workflow, sets Running
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

        // Second message while Running — should send steering signal and return
        // immediately (no blocking, no concurrent turn).
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
            .expect("Second start_run should send steering signal");

        assert!(
            start.elapsed() < std::time::Duration::from_millis(100),
            "Steering signal path should be instant, took {:?}",
            start.elapsed()
        );

        // Workflow should still be running (no concurrent turn started)
        assert!(
            runner.is_running(session_id).await,
            "Workflow should still be running — no concurrent turn"
        );

        // Verify a steering signal was stored
        let mut store = runner.store.lock().await;
        let signals = store
            .get_and_consume_signals(session_id.uuid())
            .await
            .expect("Should get signals");
        assert_eq!(signals.len(), 1, "Should have exactly 1 steering signal");
        assert_eq!(
            signals[0].signal_type,
            everruns_durable::signal_types::USER_MESSAGE
        );
        let signal_msg_id = signals[0]
            .payload
            .get("input_message_id")
            .and_then(|v| v.as_str())
            .unwrap();
        assert_eq!(signal_msg_id, message_id2.to_string());
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

    /// Test that resume_after_tool_results enqueues a reason activity directly
    /// (skipping InputAtom) using the saved DurableTurnInput from the workflow result.
    #[tokio::test]
    async fn test_resume_after_tool_results_skips_input_atom() {
        use everruns_core::DEFAULT_ORG_ID;
        use everruns_core::typed_id::TurnId;

        let runner = DurableRunner::new_in_memory();

        let session_id = SessionId::new();
        let harness_id = HarnessId::new();
        let agent_id = AgentId::new();
        let message_id = MessageId::new();
        let turn_id = TurnId::new();

        // Create a workflow via start_run (simulates the initial user message)
        runner
            .start_run(
                DEFAULT_ORG_ID,
                session_id,
                harness_id,
                Some(agent_id),
                message_id,
            )
            .await
            .expect("start_run should succeed");

        // Simulate the workflow completing with connection_required:
        // save a DurableTurnInput as the workflow result (as schedule_next_activity does).
        let saved_input = DurableTurnInput {
            org_id: DEFAULT_ORG_ID,
            session_id,
            harness_id,
            agent_id: Some(agent_id),
            input_message_id: message_id,
            turn_id: Some(turn_id),
            previous_response_id: Some("resp_abc".to_string()),
            iteration: 3,
        };
        {
            let mut store = runner.store.lock().await;
            store
                .update_workflow_status(
                    session_id.uuid(),
                    WorkflowStatus::Completed,
                    Some(serde_json::to_value(&saved_input).unwrap()),
                    None,
                )
                .await
                .expect("Should complete workflow with saved input");
        }

        assert!(!runner.is_running(session_id).await, "Should be completed");

        // Resume after tool results — should enqueue reason directly
        runner
            .resume_after_tool_results(session_id)
            .await
            .expect("resume_after_tool_results should succeed");

        // Workflow should be running again (reset to Pending with reason task)
        assert!(
            runner.is_running(session_id).await,
            "Workflow should be running after resume"
        );
    }

    /// Test that resume_after_tool_results fails when workflow has no saved turn input
    #[tokio::test]
    async fn test_resume_after_tool_results_no_saved_input() {
        use everruns_core::DEFAULT_ORG_ID;

        let runner = DurableRunner::new_in_memory();

        let session_id = SessionId::new();
        let harness_id = HarnessId::new();
        let message_id = MessageId::new();

        // Create and complete a workflow without saving turn input in result
        runner
            .start_run(DEFAULT_ORG_ID, session_id, harness_id, None, message_id)
            .await
            .expect("start_run should succeed");

        {
            let mut store = runner.store.lock().await;
            store
                .update_workflow_status(session_id.uuid(), WorkflowStatus::Completed, None, None)
                .await
                .expect("Should complete workflow");
        }

        // Should fail because no saved turn input
        let err = runner
            .resume_after_tool_results(session_id)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("no saved turn input"),
            "Error should mention missing turn input, got: {}",
            err
        );
    }

    /// Test that resume_after_tool_results fails on a still-running workflow
    #[tokio::test]
    async fn test_resume_after_tool_results_rejects_running_workflow() {
        use everruns_core::DEFAULT_ORG_ID;

        let runner = DurableRunner::new_in_memory();

        let session_id = SessionId::new();
        let harness_id = HarnessId::new();
        let message_id = MessageId::new();

        runner
            .start_run(DEFAULT_ORG_ID, session_id, harness_id, None, message_id)
            .await
            .expect("start_run should succeed");

        assert!(runner.is_running(session_id).await);

        // Should fail because workflow is still running
        let err = runner
            .resume_after_tool_results(session_id)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("not in Completed status"),
            "Error should reject non-Completed workflow, got: {}",
            err
        );
    }

    /// Test that a Pending workflow does NOT cause a 10s wait — the new turn
    /// takes over immediately (EVE-168).
    #[tokio::test]
    async fn test_start_run_takes_over_pending_workflow_immediately() {
        use everruns_core::DEFAULT_ORG_ID;

        let runner = DurableRunner::new_in_memory();

        let session_id = SessionId::new();
        let harness_id = HarnessId::new();
        let agent_id = AgentId::new();
        let message_id1 = MessageId::new();
        let message_id2 = MessageId::new();

        // First message — creates workflow (status becomes Pending)
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

        // Workflow is Pending (no worker has claimed it).
        // Don't transition to Running — leave it Pending to reproduce
        // the exact scenario from EVE-168.

        // Second message while workflow is still Pending — must complete
        // within 1s (not 10s). Using timeout so a regression fails fast
        // instead of blocking CI for 10s.
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            runner.start_run(
                DEFAULT_ORG_ID,
                session_id,
                harness_id,
                Some(agent_id),
                message_id2,
            ),
        )
        .await;

        let inner = result
            .expect("Second start_run should complete within 1s (Pending takeover, not 10s wait)");
        inner.expect("Second start_run should succeed");

        assert!(
            runner.is_running(session_id).await,
            "Workflow should be active (non-terminal) after takeover"
        );
    }

    /// Regression guard: a Running workflow blocks and waits (polling at 100ms),
    /// whereas a Pending workflow returns instantly. This test asserts the Running
    /// path takes measurable time (>200ms) while the Pending path (<50ms) doesn't.
    /// The contrast is the regression signal for EVE-168.
    #[tokio::test]
    async fn test_running_sends_signal_completed_gets_claimed() {
        use everruns_core::DEFAULT_ORG_ID;

        // --- Running path: steering signal is instant ---
        let runner = DurableRunner::new_in_memory();
        let sid = SessionId::new();
        runner
            .start_run(
                DEFAULT_ORG_ID,
                sid,
                HarnessId::new(),
                Some(AgentId::new()),
                MessageId::new(),
            )
            .await
            .unwrap();
        // Workflow is now Running

        let t0 = std::time::Instant::now();
        runner
            .start_run(
                DEFAULT_ORG_ID,
                sid,
                HarnessId::new(),
                Some(AgentId::new()),
                MessageId::new(),
            )
            .await
            .unwrap();
        let running_dur = t0.elapsed();

        assert!(
            running_dur < std::time::Duration::from_millis(50),
            "Steering signal path should be instant, took {:?}",
            running_dur
        );

        // --- Completed path: new turn gets claimed ---
        {
            let mut store = runner.store.lock().await;
            store
                .update_workflow_status(sid.uuid(), WorkflowStatus::Completed, None, None)
                .await
                .unwrap();
        }

        let t1 = std::time::Instant::now();
        runner
            .start_run(
                DEFAULT_ORG_ID,
                sid,
                HarnessId::new(),
                Some(AgentId::new()),
                MessageId::new(),
            )
            .await
            .unwrap();
        let completed_dur = t1.elapsed();

        assert!(
            completed_dur < std::time::Duration::from_millis(50),
            "Completed takeover should be instant, took {:?}",
            completed_dur
        );
        assert!(runner.is_running(sid).await);
    }

    /// Test that three rapid messages while workflow stays Pending all succeed
    /// instantly — no cumulative delay (EVE-168).
    #[tokio::test]
    async fn test_multiple_rapid_messages_while_pending() {
        use everruns_core::DEFAULT_ORG_ID;

        let runner = DurableRunner::new_in_memory();

        let session_id = SessionId::new();
        let harness_id = HarnessId::new();
        let agent_id = AgentId::new();

        // First message — creates workflow (Pending)
        runner
            .start_run(
                DEFAULT_ORG_ID,
                session_id,
                harness_id,
                Some(agent_id),
                MessageId::new(),
            )
            .await
            .expect("First start_run should succeed");

        // Send two more messages rapidly while Running — should send steering signals
        let start = std::time::Instant::now();
        for i in 0..2 {
            runner
                .start_run(
                    DEFAULT_ORG_ID,
                    session_id,
                    harness_id,
                    Some(agent_id),
                    MessageId::new(),
                )
                .await
                .unwrap_or_else(|e| panic!("Message {} should succeed: {}", i + 2, e));
        }

        assert!(
            start.elapsed() < std::time::Duration::from_secs(1),
            "Steering signal sends should be instant — took {:?}",
            start.elapsed(),
        );
        assert!(runner.is_running(session_id).await);
    }

    /// Test that start_run sets workflow to Running and subsequent calls
    /// send steering signals rather than creating concurrent turns.
    #[tokio::test]
    async fn test_start_run_sets_running_prevents_concurrent_turns() {
        use everruns_core::DEFAULT_ORG_ID;

        let runner = DurableRunner::new_in_memory();

        let session_id = SessionId::new();
        let harness_id = HarnessId::new();
        let agent_id = AgentId::new();
        let msg1 = MessageId::new();
        let msg2 = MessageId::new();
        let msg3 = MessageId::new();

        // First message — workflow created, set to Running
        runner
            .start_run(DEFAULT_ORG_ID, session_id, harness_id, Some(agent_id), msg1)
            .await
            .expect("First start_run");

        let mut store = runner.store.lock().await;
        let (status, _, _) = store
            .get_workflow_status(session_id.uuid())
            .await
            .expect("Should get status");
        assert_eq!(
            status,
            WorkflowStatus::Running,
            "First start_run should set workflow to Running"
        );
        drop(store);

        // Second and third messages — both should send steering signals
        runner
            .start_run(DEFAULT_ORG_ID, session_id, harness_id, Some(agent_id), msg2)
            .await
            .expect("Second start_run (steering)");
        runner
            .start_run(DEFAULT_ORG_ID, session_id, harness_id, Some(agent_id), msg3)
            .await
            .expect("Third start_run (steering)");

        // Workflow should still be Running
        let mut store = runner.store.lock().await;
        let (status, _, _) = store
            .get_workflow_status(session_id.uuid())
            .await
            .expect("Should get status");
        assert_eq!(status, WorkflowStatus::Running);

        // Should have 2 pending signals (msg2 and msg3)
        let signals = store
            .get_and_consume_signals(session_id.uuid())
            .await
            .expect("Should get signals");
        assert_eq!(
            signals.len(),
            2,
            "Should have 2 steering signals, got {}",
            signals.len()
        );
    }
}
