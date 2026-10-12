//! End to end: the turn driver runs a whole turn over the in-memory durable
//! store with the in-process runtime as its host, the way a framework durable
//! backend will. The worker runs the same driver over gRPC.

use std::sync::Arc;
use std::time::Duration;

use everruns_contracts::tool_types::ToolCall;
use everruns_contracts::typed_id::TurnId;
use everruns_llmsim::{LlmSimConfig, LlmSimRuntimeExt};
use uuid::Uuid;

use crate::core::InputMessage;
use crate::durable::{
    ActivityOptions, EventLog, InMemoryWorkflowEventStore, TaskDefinition, TaskQueue, WorkerInfo,
    WorkerRegistry, WorkflowStatus,
};
use crate::durable_runner::DurableTurnInput;
use crate::engine::ReasonInput;
use crate::host::{
    AcceptedTurnInput, InProcessRuntime, RuntimeHostAdapter, in_process_internal_org_id,
};
use crate::task_heartbeat::CancelSignals;
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

#[tokio::test]
async fn driver_runs_a_tool_turn_to_completion_on_the_memory_store() {
    // First reason calls a tool, the act step answers it, the second reason
    // gives the final answer: process_input -> act -> reason, each claimed
    // from the queue.
    let (claimed, store) = run_tool_turn(false).await;
    assert_eq!(claimed, ["process_input", "act", "reason"]);
    assert_eq!(completed_steps(&store).await, 3);
}

#[tokio::test]
async fn a_chaining_driver_runs_every_step_from_one_claim() {
    // The first claim runs the whole turn: each next step is enqueued claimed
    // by the same worker, and nothing is left in the queue.
    let (claimed, store) = run_tool_turn(true).await;
    assert_eq!(claimed, ["process_input"]);
    assert_eq!(completed_steps(&store).await, 3);
}

/// Completed task rows: every step keeps its own row when chained.
async fn completed_steps(store: &InMemoryWorkflowEventStore) -> usize {
    TaskQueue::list_tasks(store, Default::default(), Default::default())
        .await
        .unwrap()
        .into_iter()
        .filter(|task| task.status == crate::durable::TaskStatus::Completed)
        .count()
}

/// Run the tool turn, claiming from the queue until it is empty. Returns the
/// activity types claimed from the queue, and the store.
async fn run_tool_turn(chain: bool) -> (Vec<String>, Arc<InMemoryWorkflowEventStore>) {
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
    let org_id = in_process_internal_org_id(&snapshot.organization_id);

    // The caller persists the input message before it enqueues the turn.
    let input = AcceptedTurnInput::new(InputMessage::user("hello"));
    let input_message_id = input.message_id();
    runtime
        .append_accepted_inputs(session_id, TurnId::new(), vec![input])
        .await
        .unwrap();

    let store = Arc::new(InMemoryWorkflowEventStore::new());
    let workflow_id = Uuid::now_v7();
    store
        .create_workflow(workflow_id, "turn", serde_json::json!({}), None)
        .await
        .unwrap();
    EventLog::update_workflow_status(&*store, workflow_id, WorkflowStatus::Running, None, None)
        .await
        .unwrap();
    let turn_input = DurableTurnInput {
        org_id,
        session_id,
        harness_id: snapshot.harness_id,
        agent_id: snapshot.agent_id,
        input_message_id,
        turn_id: None,
        previous_response_id: None,
        iteration: 1,
        request_id: None,
        started_at: None,
        cumulative_usage: None,
        tool_call_count: 0,
        llm_call_count: 0,
        time_to_first_token_ms: None,
        final_message_id: None,
        final_answer_preview: None,
    };
    store
        .enqueue_task(TaskDefinition {
            workflow_id: Some(workflow_id),
            activity_id: format!("input_{}", Uuid::now_v7()),
            activity_type: "process_input".into(),
            input: serde_json::to_value(&turn_input).unwrap(),
            options: ActivityOptions::default(),
        })
        .await
        .unwrap();
    WorkerRegistry::register_worker(&*store, WorkerInfo::new("worker", TURN_ACTIVITIES))
        .await
        .unwrap();

    let driver = TurnTaskDriver::new(
        store.clone(),
        RuntimeHosts(runtime.clone()),
        "worker",
        Duration::from_secs(30),
    )
    .chain_steps(chain);
    let activity_types = TURN_ACTIVITIES.map(String::from);
    let mut ran = Vec::new();
    for _ in 0..10 {
        let tasks = TaskQueue::claim_task(&*store, "worker", &activity_types, 1)
            .await
            .unwrap();
        let Some(task) = tasks.into_iter().next() else {
            break;
        };
        driver.execute_task(&task).await.unwrap();
        ran.push(task.activity_type);
    }

    assert_eq!(
        EventLog::get_workflow_status(&*store, workflow_id)
            .await
            .unwrap(),
        WorkflowStatus::Completed
    );
    let messages = runtime.messages(session_id).await.unwrap();
    let last = messages.last().unwrap();
    assert!(
        last.content_to_llm_string().contains("durable done"),
        "final answer persisted: {last:?}"
    );
    (ran, store)
}

/// EVE-1235: a turn whose session was deleted under it stops quietly. The
/// task fails once without retry, nothing is re-queued, and the driver does
/// not report a task failure (the worker would log it as an error).
#[tokio::test]
async fn a_turn_for_a_deleted_session_stops_without_error() {
    let runtime = InProcessRuntime::builder()
        .llm_sim_as_default(LlmSimConfig::sequence(vec!["unused".into()]))
        .single_session(|session| session)
        .build()
        .await
        .unwrap();
    let live_session = runtime.default_session_id().unwrap();
    let snapshot = runtime
        .load_resolved_turn(0, live_session)
        .await
        .unwrap()
        .snapshot;

    let store = Arc::new(InMemoryWorkflowEventStore::new());
    let workflow_id = Uuid::now_v7();
    store
        .create_workflow(workflow_id, "turn", serde_json::json!({}), None)
        .await
        .unwrap();
    EventLog::update_workflow_status(&*store, workflow_id, WorkflowStatus::Running, None, None)
        .await
        .unwrap();
    // A session the runtime no longer has: every load and emit for it fails
    // with `SessionNotFound`, as the control plane answers after a delete.
    let turn_input = DurableTurnInput {
        org_id: in_process_internal_org_id(&snapshot.organization_id),
        session_id: everruns_contracts::typed_id::SessionId::new(),
        harness_id: snapshot.harness_id,
        agent_id: snapshot.agent_id,
        input_message_id: everruns_contracts::typed_id::MessageId::new(),
        turn_id: Some(TurnId::new()),
        previous_response_id: None,
        iteration: 1,
        request_id: None,
        started_at: None,
        cumulative_usage: None,
        tool_call_count: 0,
        llm_call_count: 0,
        time_to_first_token_ms: None,
        final_message_id: None,
        final_answer_preview: None,
    };
    store
        .enqueue_task(TaskDefinition {
            workflow_id: Some(workflow_id),
            activity_id: format!("input_{}", Uuid::now_v7()),
            activity_type: "process_input".into(),
            input: serde_json::to_value(&turn_input).unwrap(),
            options: ActivityOptions::default(),
        })
        .await
        .unwrap();
    WorkerRegistry::register_worker(&*store, WorkerInfo::new("worker", TURN_ACTIVITIES))
        .await
        .unwrap();

    let driver = TurnTaskDriver::new(
        store.clone(),
        RuntimeHosts(runtime),
        "worker",
        Duration::from_secs(30),
    );
    let activity_types = TURN_ACTIVITIES.map(String::from);
    let task = TaskQueue::claim_task(&*store, "worker", &activity_types, 1)
        .await
        .unwrap()
        .pop()
        .unwrap();

    driver
        .execute_task(&task)
        .await
        .expect("a deleted session ends the turn, not the task with an error");

    let tasks = TaskQueue::list_tasks(&*store, Default::default(), Default::default())
        .await
        .unwrap();
    assert_eq!(
        tasks.len(),
        1,
        "nothing scheduled after the deleted session"
    );
    // Failed without retry: dead-lettered, as any non-retryable failure.
    assert_eq!(tasks[0].status, crate::durable::TaskStatus::Dead);
    assert!(
        TaskQueue::claim_task(&*store, "worker", &activity_types, 1)
            .await
            .unwrap()
            .is_empty(),
        "the step is not retried"
    );
}
