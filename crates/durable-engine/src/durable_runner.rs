// Durable execution engine runner adapters.
// Decision: everruns-durable-engine owns durable orchestration and maps runtime turn state onto the durable engine.
// Decision: Core host remains durable-agnostic and only exports generic turn strategy/state.

use async_trait::async_trait;
use everruns_contracts::typed_id::SessionId;
pub use everruns_core::engine::TurnState as DurableTurnInput;
use everruns_durable::{InMemoryWorkflowEventStore, PostgresWorkflowEventStore};
use std::sync::Arc;
use tracing::info;

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

/// Durable execution engine based runner.
///
/// This runner maps runtime turn state onto the durable engine.
/// It implements [`TurnBackend`](everruns_core::host::TurnBackend) (see
/// [`crate::turn_backend`]), the only way to start, continue or cancel its
/// turns.
pub struct DurableRunner {
    pub(crate) store: Arc<dyn crate::turn_store::TurnStore>,
    task_notifier: Option<Arc<dyn DurableTaskNotifier>>,
}

impl DurableRunner {
    /// Build a runner over any turn store.
    ///
    /// Process-specific transports (the worker's gRPC store) implement
    /// [`TurnStore`](crate::turn_store::TurnStore) in their own crate and
    /// enter here, so this crate stays free of transport dependencies.
    pub fn from_store(store: impl crate::turn_store::TurnStore) -> Self {
        Self::from_shared(Arc::new(store))
    }

    /// Build a runner over a turn store others (a backend's workers) share.
    pub(crate) fn from_shared(store: Arc<dyn crate::turn_store::TurnStore>) -> Self {
        Self {
            store,
            task_notifier: None,
        }
    }

    pub fn new_with_pool(pool: everruns_durable::PostgresPool) -> Self {
        info!("Initializing durable runner (direct DB mode)");
        Self::from_store(PostgresWorkflowEventStore::new(pool))
    }

    pub fn new_with_pool_and_task_notifier(
        pool: everruns_durable::PostgresPool,
        task_notifier: Arc<dyn DurableTaskNotifier>,
    ) -> Self {
        info!("Initializing durable runner (direct DB mode with task notifier)");
        Self::from_store(PostgresWorkflowEventStore::new(pool)).with_task_notifier(task_notifier)
    }

    pub fn new_in_memory() -> Self {
        info!("Initializing durable runner (in-memory dev mode)");
        Self::from_store(InMemoryWorkflowEventStore::new())
    }

    pub fn new_with_shared_store(shared_store: Arc<InMemoryWorkflowEventStore>) -> Self {
        info!("Initializing durable runner (shared in-memory dev mode)");
        Self::from_shared(shared_store)
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
    use everruns_contracts::typed_id::{AgentId, HarnessId, MessageId, TurnId};
    use everruns_core::host::{TurnBackend, TurnInput, TurnRequest, TurnScope};
    use everruns_durable::{EventLog, SignalStore, TaskQueue, WorkerRegistry, WorkflowStatus};
    use uuid::Uuid;

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

    /// The platform server's calls: a stored message with its scope, a
    /// recorded tool resolution, and a cancel, each on the `TurnBackend`.
    #[async_trait]
    trait ServerTurns {
        async fn start_run(
            &self,
            org_id: i64,
            session_id: SessionId,
            harness_id: HarnessId,
            agent_id: Option<AgentId>,
            message_id: MessageId,
            request_id: Option<String>,
        ) -> everruns_contracts::error::Result<()>;
        async fn resume_after_tool_results(
            &self,
            session_id: SessionId,
            resolution_id: Uuid,
        ) -> everruns_contracts::error::Result<()>;
        async fn cancel_run(&self, session_id: SessionId) -> everruns_contracts::error::Result<()>;
    }

    #[async_trait]
    impl ServerTurns for DurableRunner {
        async fn start_run(
            &self,
            org_id: i64,
            session_id: SessionId,
            harness_id: HarnessId,
            agent_id: Option<AgentId>,
            message_id: MessageId,
            request_id: Option<String>,
        ) -> everruns_contracts::error::Result<()> {
            let request = TurnRequest::new(
                session_id,
                TurnId::new(),
                TurnInput::StoredMessage { message_id },
            )
            .with_scope(TurnScope::new(org_id, harness_id, agent_id))
            .with_request_id(request_id);
            self.start_turn(request).await.map(drop)
        }

        async fn resume_after_tool_results(
            &self,
            session_id: SessionId,
            resolution_id: Uuid,
        ) -> everruns_contracts::error::Result<()> {
            let request = TurnRequest::new(
                session_id,
                TurnId::new(),
                TurnInput::RecordedToolResults { resolution_id },
            );
            self.start_turn(request).await.map(drop)
        }

        async fn cancel_run(&self, session_id: SessionId) -> everruns_contracts::error::Result<()> {
            self.cancel(session_id).await.map(drop)
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
            issue_count: 0,
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
            issue_count: 0,
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

    #[tokio::test]
    async fn concurrent_sends_start_one_turn_and_steer_the_rest() {
        // No runner lock: the store's atomic start elects the turn.
        let shared = Arc::new(InMemoryWorkflowEventStore::new());
        let runner = DurableRunner::new_with_shared_store(shared.clone());
        let session_id = SessionId::new();
        let harness_id = HarnessId::new();
        let send = || runner.start_run(1, session_id, harness_id, None, MessageId::new(), None);

        let (a, b, c, d) = tokio::join!(send(), send(), send(), send());
        for result in [a, b, c, d] {
            result.expect("start_run");
        }

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
            .expect("claim");
        assert_eq!(claimed.len(), 1);
        let signals = shared
            .consume_pending_signals(session_id.uuid())
            .await
            .expect("signals");
        assert_eq!(signals.len(), 3);
    }
}
