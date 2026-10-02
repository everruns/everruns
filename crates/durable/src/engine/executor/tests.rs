use super::*;
use crate::persistence::{EventLog, InMemoryWorkflowEventStore, TaskQueue, WorkerRegistry};
use serde::{Deserialize, Serialize};

// Test workflow implementation
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CounterInput {
    start: i32,
    target: i32,
}

#[derive(Debug, Serialize, Deserialize)]
struct CounterOutput {
    final_value: i32,
}

struct CounterWorkflow {
    current: i32,
    target: i32,
    completed: bool,
    failed: bool,
    error_message: Option<String>,
}

impl crate::workflow::Workflow for CounterWorkflow {
    const TYPE: &'static str = "counter_workflow";
    type Input = CounterInput;
    type Output = CounterOutput;

    fn new(input: Self::Input) -> Self {
        Self {
            current: input.start,
            target: input.target,
            completed: false,
            failed: false,
            error_message: None,
        }
    }

    fn on_start(&mut self) -> Vec<WorkflowAction> {
        if self.current >= self.target {
            self.completed = true;
            vec![WorkflowAction::complete(
                serde_json::json!({ "final_value": self.current }),
            )]
        } else {
            vec![WorkflowAction::schedule_activity(
                format!("increment-{}", self.current),
                "increment",
                serde_json::json!({ "value": self.current }),
            )]
        }
    }

    fn on_activity_completed(
        &mut self,
        _activity_id: &str,
        result: serde_json::Value,
    ) -> Vec<WorkflowAction> {
        self.current = result.get("value").and_then(|v| v.as_i64()).unwrap_or(0) as i32;

        if self.current >= self.target {
            self.completed = true;
            vec![WorkflowAction::complete(
                serde_json::json!({ "final_value": self.current }),
            )]
        } else {
            vec![WorkflowAction::schedule_activity(
                format!("increment-{}", self.current),
                "increment",
                serde_json::json!({ "value": self.current }),
            )]
        }
    }

    fn on_activity_failed(
        &mut self,
        _activity_id: &str,
        error: &ActivityError,
    ) -> Vec<WorkflowAction> {
        self.failed = true;
        self.error_message = Some(error.message.clone());
        vec![WorkflowAction::fail(crate::WorkflowError::new(
            &error.message,
        ))]
    }

    fn is_completed(&self) -> bool {
        self.completed || self.failed
    }

    fn result(&self) -> Option<Self::Output> {
        if self.completed && !self.failed {
            Some(CounterOutput {
                final_value: self.current,
            })
        } else {
            None
        }
    }

    fn error(&self) -> Option<crate::WorkflowError> {
        self.error_message.as_ref().map(crate::WorkflowError::new)
    }
}

#[tokio::test]
async fn test_start_workflow() {
    let store = InMemoryWorkflowEventStore::new();
    let mut executor = WorkflowExecutor::new(store);
    executor.register::<CounterWorkflow>();

    let input = CounterInput {
        start: 0,
        target: 3,
    };
    let workflow_id = executor
        .start_workflow::<CounterWorkflow>(input, None)
        .await
        .expect("should start workflow");

    // Verify workflow was created
    let status = executor
        .store()
        .get_workflow_status(workflow_id)
        .await
        .expect("should get status");

    assert_eq!(status, WorkflowStatus::Running);

    // Verify events were written
    let events = executor
        .store()
        .load_events(workflow_id)
        .await
        .expect("should load events");

    assert!(events.len() >= 2); // WorkflowStarted + ActivityScheduled
    assert!(matches!(events[0].1, WorkflowEvent::WorkflowStarted { .. }));
    assert!(matches!(
        events[1].1,
        WorkflowEvent::ActivityScheduled { .. }
    ));
}

#[tokio::test]
async fn test_immediate_completion() {
    let store = InMemoryWorkflowEventStore::new();
    let mut executor = WorkflowExecutor::new(store);
    executor.register::<CounterWorkflow>();

    // Start with current >= target, should complete immediately
    let input = CounterInput {
        start: 5,
        target: 3,
    };
    let workflow_id = executor
        .start_workflow::<CounterWorkflow>(input, None)
        .await
        .expect("should start workflow");

    // Verify workflow completed
    let status = executor
        .store()
        .get_workflow_status(workflow_id)
        .await
        .expect("should get status");

    assert_eq!(status, WorkflowStatus::Completed);
}

#[tokio::test]
async fn test_activity_completion() {
    let store = InMemoryWorkflowEventStore::new();
    let mut executor = WorkflowExecutor::new(store);
    executor.register::<CounterWorkflow>();

    let input = CounterInput {
        start: 0,
        target: 2,
    };
    let workflow_id = executor
        .start_workflow::<CounterWorkflow>(input, None)
        .await
        .expect("should start workflow");

    // Complete first activity (increment 0 -> 1)
    let result = executor
        .on_activity_completed(
            workflow_id,
            "increment-0",
            serde_json::json!({ "value": 1 }),
        )
        .await
        .expect("should complete activity");

    assert!(!result.completed);

    // Complete second activity (increment 1 -> 2)
    let result = executor
        .on_activity_completed(
            workflow_id,
            "increment-1",
            serde_json::json!({ "value": 2 }),
        )
        .await
        .expect("should complete activity");

    assert!(result.completed);

    // Verify final status
    let status = executor
        .store()
        .get_workflow_status(workflow_id)
        .await
        .expect("should get status");

    assert_eq!(status, WorkflowStatus::Completed);
}

/// Claim every pending `increment` task, sorted: the in-memory store does
/// not claim in FIFO order.
async fn claim_all(executor: &WorkflowExecutor<InMemoryWorkflowEventStore>) -> Vec<String> {
    let store = executor.store();
    let worker = crate::WorkerInfo::new("test-worker", ["increment"]);
    store.register_worker(worker).await.unwrap();
    store
        .claim_task("test-worker", &["increment".to_string()], 100)
        .await
        .expect("should claim tasks")
        .into_iter()
        .map(|task| task.activity_id)
        .collect()
}

#[tokio::test]
async fn test_activity_completion_schedules_follow_up_activity() {
    let store = InMemoryWorkflowEventStore::new();
    let mut executor = WorkflowExecutor::new(store);
    executor.register::<CounterWorkflow>();

    let workflow_id = executor
        .start_workflow::<CounterWorkflow>(
            CounterInput {
                start: 0,
                target: 3,
            },
            None,
        )
        .await
        .expect("should start workflow");
    assert_eq!(claim_all(&executor).await, vec!["increment-0"]);

    let result = executor
        .on_activity_completed(
            workflow_id,
            "increment-0",
            serde_json::json!({ "value": 1 }),
        )
        .await
        .expect("should complete activity");

    // The action returned by on_activity_completed must reach the queue,
    // otherwise a multi-step workflow stalls after its first activity.
    assert_eq!(result.tasks_enqueued, 1);
    assert_eq!(claim_all(&executor).await, vec!["increment-1"]);
}

#[tokio::test]
async fn test_process_workflow_is_idempotent() {
    let store = InMemoryWorkflowEventStore::new();
    let mut executor = WorkflowExecutor::new(store);
    executor.register::<CounterWorkflow>();

    let workflow_id = executor
        .start_workflow::<CounterWorkflow>(
            CounterInput {
                start: 0,
                target: 3,
            },
            None,
        )
        .await
        .expect("should start workflow");
    executor
        .on_activity_completed(
            workflow_id,
            "increment-0",
            serde_json::json!({ "value": 1 }),
        )
        .await
        .expect("should complete activity");
    let events_before = executor.store().count_events(workflow_id).await.unwrap();

    // Re-processing (e.g. after a crash or a duplicate delivery) must not
    // schedule the same follow-up twice.
    let result = executor
        .process_workflow(workflow_id)
        .await
        .expect("should process workflow");

    assert_eq!(result.tasks_enqueued, 0);
    assert_eq!(result.events_written, 0);
    assert_eq!(
        executor.store().count_events(workflow_id).await.unwrap(),
        events_before
    );
    assert_eq!(
        claim_all(&executor).await,
        vec!["increment-0", "increment-1"]
    );
}

#[tokio::test]
async fn test_process_workflow_applies_actions_left_unapplied() {
    let store = InMemoryWorkflowEventStore::new();
    let mut executor = WorkflowExecutor::new(store);
    executor.register::<CounterWorkflow>();

    let workflow_id = executor
        .start_workflow::<CounterWorkflow>(
            CounterInput {
                start: 0,
                target: 3,
            },
            None,
        )
        .await
        .expect("should start workflow");

    // Simulate a crash right after the completion event was persisted,
    // before its follow-up activity was scheduled.
    let next_seq = executor.store().count_events(workflow_id).await.unwrap() as i32;
    executor
        .store()
        .append_events(
            workflow_id,
            next_seq,
            vec![WorkflowEvent::ActivityCompleted {
                activity_id: "increment-0".into(),
                result: serde_json::json!({ "value": 1 }),
            }],
        )
        .await
        .unwrap();

    let result = executor
        .process_workflow(workflow_id)
        .await
        .expect("should process workflow");

    assert_eq!(result.tasks_enqueued, 1);
    assert_eq!(
        claim_all(&executor).await,
        vec!["increment-0", "increment-1"]
    );
}

#[tokio::test]
async fn test_completion_records_workflow_completed_event() {
    let store = InMemoryWorkflowEventStore::new();
    let mut executor = WorkflowExecutor::new(store);
    executor.register::<CounterWorkflow>();

    let workflow_id = executor
        .start_workflow::<CounterWorkflow>(
            CounterInput {
                start: 0,
                target: 1,
            },
            None,
        )
        .await
        .expect("should start workflow");
    let result = executor
        .on_activity_completed(
            workflow_id,
            "increment-0",
            serde_json::json!({ "value": 1 }),
        )
        .await
        .expect("should complete activity");
    assert!(result.completed);

    let events = executor.store().load_events(workflow_id).await.unwrap();
    let completed: Vec<_> = events
        .iter()
        .filter_map(|(_, event)| match event {
            WorkflowEvent::WorkflowCompleted { result } => Some(result.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(completed, vec![serde_json::json!({ "final_value": 1 })]);
    let info = executor
        .store()
        .get_workflow_info(workflow_id)
        .await
        .unwrap();
    assert_eq!(info.status, WorkflowStatus::Completed);
}

#[tokio::test]
async fn test_activity_failure() {
    let store = InMemoryWorkflowEventStore::new();
    let mut executor = WorkflowExecutor::new(store);
    executor.register::<CounterWorkflow>();

    let input = CounterInput {
        start: 0,
        target: 5,
    };
    let workflow_id = executor
        .start_workflow::<CounterWorkflow>(input, None)
        .await
        .expect("should start workflow");

    // Fail the activity (final failure, no retry)
    let error = ActivityError::non_retryable("increment failed").with_type("INCREMENT_ERROR");
    let result = executor
        .on_activity_failed(workflow_id, "increment-0", error, false)
        .await
        .expect("should handle failure");

    assert!(result.completed);

    // Verify workflow failed
    let status = executor
        .store()
        .get_workflow_status(workflow_id)
        .await
        .expect("should get status");

    assert_eq!(status, WorkflowStatus::Failed);
}

#[tokio::test]
async fn test_signal_handling() {
    let store = InMemoryWorkflowEventStore::new();
    let mut executor = WorkflowExecutor::new(store);
    executor.register::<CounterWorkflow>();

    let input = CounterInput {
        start: 0,
        target: 10,
    };
    let workflow_id = executor
        .start_workflow::<CounterWorkflow>(input, None)
        .await
        .expect("should start workflow");

    // Send a signal
    let signal = WorkflowSignal::new("test_signal", serde_json::json!({ "data": "hello" }));
    executor
        .send_signal(workflow_id, signal)
        .await
        .expect("should send signal");

    // Process workflow (should handle signal)
    let result = executor
        .process_workflow(workflow_id)
        .await
        .expect("should process");

    assert_eq!(result.signals_processed, 1);
}

#[tokio::test]
async fn test_cannot_signal_completed_workflow() {
    let store = InMemoryWorkflowEventStore::new();
    let mut executor = WorkflowExecutor::new(store);
    executor.register::<CounterWorkflow>();

    // Start workflow that completes immediately
    let input = CounterInput {
        start: 10,
        target: 5,
    };
    let workflow_id = executor
        .start_workflow::<CounterWorkflow>(input, None)
        .await
        .expect("should start workflow");

    // Try to send signal to completed workflow
    let signal = WorkflowSignal::new("test", serde_json::json!({}));
    let result = executor.send_signal(workflow_id, signal).await;

    assert!(matches!(result, Err(ExecutorError::WorkflowCompleted(_))));
}

#[tokio::test]
async fn test_replay_consistency() {
    let store = InMemoryWorkflowEventStore::new();
    let mut executor = WorkflowExecutor::new(store);
    executor.register::<CounterWorkflow>();

    let input = CounterInput {
        start: 0,
        target: 3,
    };
    let workflow_id = executor
        .start_workflow::<CounterWorkflow>(input, None)
        .await
        .expect("should start workflow");

    // Complete activities
    executor
        .on_activity_completed(
            workflow_id,
            "increment-0",
            serde_json::json!({ "value": 1 }),
        )
        .await
        .unwrap();
    executor
        .on_activity_completed(
            workflow_id,
            "increment-1",
            serde_json::json!({ "value": 2 }),
        )
        .await
        .unwrap();
    executor
        .on_activity_completed(
            workflow_id,
            "increment-2",
            serde_json::json!({ "value": 3 }),
        )
        .await
        .unwrap();

    // Process workflow again - should handle already completed state
    let result = executor.process_workflow(workflow_id).await.unwrap();
    assert!(result.completed);
}

mod snapshot_tests;
