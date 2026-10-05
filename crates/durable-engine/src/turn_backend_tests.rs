//! `DurableRunner` as a `TurnBackend` on the in-memory durable store: turns
//! start eagerly, tickets follow the workflow to its end, and the
//! `AgentRunner` shim goes through the same path.

use std::sync::Arc;
use std::time::Duration;

use everruns_contracts::error::AgentLoopError;
use everruns_contracts::tool_types::ToolCall;
use everruns_contracts::typed_id::{HarnessId, MessageId, SessionId, TurnId};
use everruns_llmsim::{LlmSimConfig, LlmSimRuntimeExt};
use uuid::Uuid;

use crate::core::InputMessage;
use crate::core::turn::TurnStopReason;
use crate::durable::{
    EventLog, InMemoryWorkflowEventStore, TaskQueue, WorkerInfo, WorkerRegistry, WorkflowError,
    WorkflowEvent, WorkflowSignal, WorkflowStatus,
};
use crate::durable_runner::{
    DurableRunner, DurableStoreBackend, InMemoryDurableStore, WorkflowEndSignal,
};
use crate::engine::ReasonInput;
use crate::host::{
    AcceptedTurnInput, InProcessRuntime, PersistedTurn, RuntimeHostAdapter, TurnBackend, TurnInput,
    TurnRequest, TurnTicket, in_process_internal_org_id,
};
use crate::runner::AgentRunner;
use crate::task_heartbeat::CancelSignals;
use crate::task_store::TaskStore;
use crate::turn_backend::TICKET_FALLBACK_POLL_INTERVAL;
use crate::turn_driver::{TurnTaskDriver, TurnTaskHost};

/// The in-process runtime as the driver's host: every step runs on it.
#[derive(Clone)]
struct RuntimeHosts(InProcessRuntime);

impl TurnTaskHost for RuntimeHosts {
    type Host = InProcessRuntime;

    fn host(&self) -> InProcessRuntime {
        self.0.clone()
    }

    fn reason_host(&self, _input: &ReasonInput, _cancel: CancelSignals) -> InProcessRuntime {
        self.0.clone()
    }
}

const TURN_ACTIVITIES: [&str; 3] = ["process_input", "reason", "act"];

fn persisted_message(session_id: SessionId) -> TurnRequest {
    TurnRequest::new(
        session_id,
        TurnId::new(),
        TurnInput::Persisted(Box::new(PersistedTurn::Message {
            org_id: 1,
            harness_id: HarnessId::new(),
            agent_id: None,
            input_message_id: MessageId::new(),
            request_id: None,
        })),
    )
}

/// Polling the ticket briefly must not resolve it: the workflow still runs.
async fn assert_pending(ticket: &mut TurnTicket) {
    assert!(
        tokio::time::timeout(Duration::from_millis(150), ticket)
            .await
            .is_err(),
        "the ticket resolves only when the workflow ends"
    );
}

async fn shared_runner() -> (Arc<InMemoryWorkflowEventStore>, DurableRunner) {
    let store = Arc::new(InMemoryWorkflowEventStore::new());
    WorkerRegistry::register_worker(&*store, WorkerInfo::new("worker", TURN_ACTIVITIES))
        .await
        .unwrap();
    let runner = DurableRunner::new_with_shared_store(store.clone());
    (store, runner)
}

async fn claimed_activity_types(store: &InMemoryWorkflowEventStore) -> Vec<String> {
    let activity_types = TURN_ACTIVITIES.map(String::from);
    TaskQueue::claim_task(store, "worker", &activity_types, 10)
        .await
        .unwrap()
        .into_iter()
        .map(|task| task.activity_type)
        .collect()
}

#[tokio::test]
async fn persisted_message_starts_the_workflow_before_the_ticket_is_polled() {
    let (store, runner) = shared_runner().await;
    let session_id = SessionId::new();

    let ticket = runner
        .start_turn(persisted_message(session_id))
        .await
        .unwrap();
    // Dropped unpolled, as the server's fire-and-forget callers do.
    drop(ticket);

    assert!(TurnBackend::is_running(&runner, session_id).await);
    assert_eq!(TurnBackend::active_count(&runner).await, 1);
    assert_eq!(claimed_activity_types(&store).await, ["process_input"]);
}

#[tokio::test]
async fn ticket_resolves_with_the_turn_the_driver_completes() {
    // First reason calls a tool, the act step answers it, the second reason
    // gives the final answer.
    let runtime = InProcessRuntime::builder()
        .llm_sim_as_default(
            LlmSimConfig::sequence(vec!["checking".into(), "durable done".into()])
                .with_tool_call_sequence(vec![
                    vec![ToolCall {
                        id: "call-1".into(),
                        name: "no_such_tool".into(),
                        arguments: serde_json::json!({}),
                    }],
                    vec![],
                ]),
        )
        .single_session(|session| session)
        .build()
        .await
        .unwrap();
    let session_id = runtime.default_session_id().unwrap();
    let snapshot = runtime
        .load_resolved_turn(0, session_id)
        .await
        .unwrap()
        .snapshot;

    // The server persists the message before it starts the turn.
    let input = AcceptedTurnInput::new(InputMessage::user("hello"));
    let input_message_id = input.message_id();
    runtime
        .append_accepted_inputs(session_id, TurnId::new(), vec![input])
        .await
        .unwrap();

    let (store, runner) = shared_runner().await;
    let request_turn_id = TurnId::new();
    let mut ticket = runner
        .start_turn(TurnRequest::new(
            session_id,
            request_turn_id,
            TurnInput::Persisted(Box::new(PersistedTurn::Message {
                org_id: in_process_internal_org_id(&snapshot.organization_id),
                harness_id: snapshot.harness_id,
                agent_id: snapshot.agent_id,
                input_message_id,
                request_id: None,
            })),
        ))
        .await
        .unwrap();
    assert_eq!(ticket.session_id(), session_id);
    assert_eq!(ticket.turn_id(), request_turn_id);
    assert_pending(&mut ticket).await;

    let driver = TurnTaskDriver::new(
        store.clone(),
        RuntimeHosts(runtime.clone()),
        "worker",
        Duration::from_secs(30),
    );
    let activity_types = TURN_ACTIVITIES.map(String::from);
    let mut ran = Vec::new();
    while let Some(task) = TaskQueue::claim_task(&*store, "worker", &activity_types, 1)
        .await
        .unwrap()
        .into_iter()
        .next()
    {
        driver.execute_task(&task).await.unwrap();
        ran.push(task.activity_type);
        assert!(ran.len() < 10, "the turn ends: {ran:?}");
    }
    assert_eq!(ran, ["process_input", "act", "reason"]);

    let result = tokio::time::timeout(Duration::from_secs(5), ticket)
        .await
        .expect("the ticket sees the completed workflow")
        .unwrap();
    assert!(result.success, "{result:?}");
    assert_eq!(result.response, "durable done");
    assert_eq!(result.stop_reason, TurnStopReason::EndTurn);
    assert_eq!(result.tool_calls_count, 1);
    assert_ne!(
        result.turn_id, request_turn_id,
        "the turn keeps the id its input step assigned"
    );
    assert!(!TurnBackend::is_running(&runner, session_id).await);
}

#[tokio::test]
async fn cancel_ends_the_ticket_with_cancelled() {
    let (store, runner) = shared_runner().await;
    let session_id = SessionId::new();
    let other = SessionId::new();
    let mut ticket = runner
        .start_turn(persisted_message(session_id))
        .await
        .unwrap();
    assert_pending(&mut ticket).await;

    assert!(runner.cancel(session_id).await.unwrap());
    let error = ticket.await.expect_err("a cancelled turn has no result");
    assert!(matches!(error, AgentLoopError::Cancelled), "{error}");
    assert!(!TurnBackend::is_running(&runner, session_id).await);
    assert!(
        claimed_activity_types(&store).await.is_empty(),
        "cancelled tasks are not claimable"
    );

    // Nothing runs any more: the flag says so, and cancelling stays an Ok.
    assert!(!runner.cancel(session_id).await.unwrap());
    assert!(!TurnBackend::is_running(&runner, other).await);
}

#[tokio::test]
async fn failed_workflow_resolves_as_a_failed_turn() {
    let (store, runner) = shared_runner().await;
    let session_id = SessionId::new();
    let ticket = runner
        .start_turn(persisted_message(session_id))
        .await
        .unwrap();

    EventLog::update_workflow_status(
        &*store,
        session_id.uuid(),
        WorkflowStatus::Failed,
        None,
        Some(WorkflowError::new("provider unavailable")),
    )
    .await
    .unwrap();

    let result = ticket.await.unwrap();
    assert!(!result.success);
    assert_eq!(result.stop_reason, TurnStopReason::Error);
    assert!(
        result
            .error
            .as_deref()
            .is_some_and(|error| error.contains("provider unavailable")),
        "{result:?}"
    );
}

#[tokio::test]
async fn completion_without_a_stop_reason_resolves_as_end_turn() {
    // A turn parked on client-side tool results completes its workflow with
    // the step's raw output, which carries no stop reason.
    let (store, runner) = shared_runner().await;
    let session_id = SessionId::new();
    let ticket = runner
        .start_turn(persisted_message(session_id))
        .await
        .unwrap();

    TaskStore::complete_workflow(
        &*store,
        session_id.uuid(),
        serde_json::json!({ "success": true, "text": "need a tool" }),
        None,
        None,
    )
    .await
    .unwrap();

    let result = ticket.await.unwrap();
    assert!(result.success, "{result:?}");
    assert_eq!(result.stop_reason, TurnStopReason::EndTurn);
    assert_eq!(result.response, "need a tool");
}

#[tokio::test]
async fn unpersisted_inputs_are_rejected_without_starting_a_workflow() {
    let (_store, runner) = shared_runner().await;
    let session_id = SessionId::new();
    let inputs = [
        TurnInput::Message(Box::new(AcceptedTurnInput::new(InputMessage::user("hi")))),
        TurnInput::ResumeInterrupted,
        TurnInput::ToolResults(Vec::new()),
    ];
    for input in inputs {
        let error = runner
            .start_turn(TurnRequest::new(session_id, TurnId::new(), input))
            .await
            .expect_err("the durable runner serves only persisted input");
        assert!(matches!(error, AgentLoopError::Configuration(_)), "{error}");
        assert!(
            error.to_string().contains("TurnInput::Persisted"),
            "{error}"
        );
    }
    assert!(!TurnBackend::is_running(&runner, session_id).await);
    assert_eq!(TurnBackend::active_count(&runner).await, 0);
}

#[tokio::test]
async fn agent_runner_shim_runs_through_the_turn_backend() {
    let (store, runner) = shared_runner().await;
    let session_id = SessionId::new();

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
        .unwrap();
    // The shim's start is the backend's: the same workflow, seen both ways.
    assert!(TurnBackend::is_running(&runner, session_id).await);
    assert!(AgentRunner::is_running(&runner, session_id).await);
    assert_eq!(AgentRunner::active_count(&runner).await, 1);
    assert_eq!(claimed_activity_types(&store).await, ["process_input"]);

    AgentRunner::cancel_run(&runner, session_id).await.unwrap();
    assert_eq!(
        EventLog::get_workflow_status(&*store, session_id.uuid())
            .await
            .unwrap(),
        WorkflowStatus::Cancelled
    );
    assert!(!TurnBackend::is_running(&runner, session_id).await);

    // A store failure keeps the message the server always logged.
    let error = runner
        .resume_after_tool_results(SessionId::new(), Uuid::now_v7())
        .await
        .expect_err("no workflow to resume");
    assert!(
        error
            .to_string()
            .starts_with("Failed to get workflow status"),
        "{error}"
    );
}

/// Let a spawned ticket run until it waits on its workflow.
async fn settle() {
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
}

// Paused time: if the ticket waited on a timer instead of the end signal,
// the idle runtime would jump the clock to that timer.
#[tokio::test(start_paused = true)]
async fn memory_ticket_wakes_on_completion_without_waiting_for_a_poll() {
    let (store, runner) = shared_runner().await;
    let session_id = SessionId::new();
    let ticket = runner
        .start_turn(persisted_message(session_id))
        .await
        .unwrap();
    let ticket = tokio::spawn(ticket);
    settle().await;
    assert!(!ticket.is_finished(), "the workflow still runs");

    let ended_at = tokio::time::Instant::now();
    TaskStore::complete_workflow(
        &*store,
        session_id.uuid(),
        serde_json::json!({ "success": true, "text": "woken", "stop_reason": "end_turn" }),
        None,
        None,
    )
    .await
    .unwrap();
    let result = ticket.await.unwrap().unwrap();

    assert_eq!(result.response, "woken");
    assert!(
        ended_at.elapsed() < crate::turn_backend::TICKET_POLL_INTERVAL,
        "the ticket resolved {:?} after the workflow ended",
        ended_at.elapsed()
    );
}

/// The memory store behind a [`DurableStoreBackend`] whose workflow-end
/// signal never fires: a lost wakeup, or an end another process wrote.
struct MissedWakeups(InMemoryDurableStore);

#[async_trait::async_trait]
impl DurableStoreBackend for MissedWakeups {
    async fn get_workflow_status(
        &mut self,
        workflow_id: Uuid,
    ) -> anyhow::Result<(WorkflowStatus, Option<serde_json::Value>, Option<String>)> {
        self.0.get_workflow_status(workflow_id).await
    }

    async fn create_workflow(
        &mut self,
        workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
    ) -> anyhow::Result<Uuid> {
        self.0
            .create_workflow(workflow_id, workflow_type, input)
            .await
    }

    async fn update_workflow_status(
        &mut self,
        workflow_id: Uuid,
        status: WorkflowStatus,
        output: Option<serde_json::Value>,
        error: Option<String>,
    ) -> anyhow::Result<()> {
        self.0
            .update_workflow_status(workflow_id, status, output, error)
            .await
    }

    async fn enqueue_task(
        &mut self,
        workflow_id: Uuid,
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
    ) -> anyhow::Result<Uuid> {
        self.0
            .enqueue_task(workflow_id, activity_id, activity_type, input)
            .await
    }

    async fn start_workflow_with_task(
        &mut self,
        workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
        activity_id: String,
        activity_type: String,
    ) -> anyhow::Result<Uuid> {
        self.0
            .start_workflow_with_task(
                workflow_id,
                workflow_type,
                input,
                activity_id,
                activity_type,
            )
            .await
    }

    async fn count_active_workflows(&mut self) -> anyhow::Result<usize> {
        self.0.count_active_workflows().await
    }

    async fn cancel_pending_tasks(&mut self, workflow_id: Uuid) -> anyhow::Result<u64> {
        self.0.cancel_pending_tasks(workflow_id).await
    }

    async fn append_events(
        &mut self,
        workflow_id: Uuid,
        expected_sequence: i32,
        events: Vec<WorkflowEvent>,
    ) -> anyhow::Result<i32> {
        self.0
            .append_events(workflow_id, expected_sequence, events)
            .await
    }

    async fn try_claim_workflow_for_new_turn(&mut self, workflow_id: Uuid) -> anyhow::Result<bool> {
        self.0.try_claim_workflow_for_new_turn(workflow_id).await
    }

    async fn send_signal(
        &mut self,
        workflow_id: Uuid,
        signal: WorkflowSignal,
    ) -> anyhow::Result<()> {
        self.0.send_signal(workflow_id, signal).await
    }

    async fn get_and_consume_signals(
        &mut self,
        workflow_id: Uuid,
    ) -> anyhow::Result<Vec<WorkflowSignal>> {
        self.0.get_and_consume_signals(workflow_id).await
    }

    async fn latest_completion_output(
        &mut self,
        workflow_id: Uuid,
    ) -> anyhow::Result<Option<serde_json::Value>> {
        self.0.latest_completion_output(workflow_id).await
    }

    fn workflow_end_signal(&self, _workflow_id: Uuid) -> Option<WorkflowEndSignal> {
        Some(Box::pin(std::future::pending()))
    }
}

#[tokio::test(start_paused = true)]
async fn ticket_resolves_on_the_fallback_poll_when_no_wakeup_comes() {
    let store = Arc::new(InMemoryWorkflowEventStore::new());
    let runner = DurableRunner::from_store(MissedWakeups(InMemoryDurableStore::from_shared(
        store.clone(),
    )));
    let session_id = SessionId::new();
    let ticket = runner
        .start_turn(persisted_message(session_id))
        .await
        .unwrap();
    let ticket = tokio::spawn(ticket);
    settle().await;
    assert!(!ticket.is_finished(), "the workflow still runs");

    let ended_at = tokio::time::Instant::now();
    EventLog::update_workflow_status(
        &*store,
        session_id.uuid(),
        WorkflowStatus::Failed,
        None,
        Some(WorkflowError::new("ended elsewhere")),
    )
    .await
    .unwrap();
    let result = ticket.await.unwrap().unwrap();

    assert!(!result.success);
    assert_eq!(result.stop_reason, TurnStopReason::Error);
    let waited = ended_at.elapsed();
    assert!(
        waited <= TICKET_FALLBACK_POLL_INTERVAL,
        "the fallback poll picked the end up, after {waited:?}"
    );
    assert!(
        waited > crate::turn_backend::TICKET_POLL_INTERVAL,
        "without a wakeup only the fallback poll sees the end, after {waited:?}"
    );
}
