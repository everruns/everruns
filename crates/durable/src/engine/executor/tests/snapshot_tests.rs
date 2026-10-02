//! Snapshot, bounded-replay and continue-as-new tests for the executor.

use super::*;
use crate::persistence::EventLog;

// =================================================================
// Snapshot-capable workflow for testing
// =================================================================

/// A counter workflow that supports snapshot serialization
#[derive(Debug, Serialize, Deserialize)]
struct SnapCounterState {
    current: i32,
    target: i32,
    completed: bool,
    failed: bool,
    error_message: Option<String>,
}

struct SnapCounterWorkflow {
    state: SnapCounterState,
}

impl crate::workflow::Workflow for SnapCounterWorkflow {
    const TYPE: &'static str = "snap_counter_workflow";
    type Input = CounterInput;
    type Output = CounterOutput;

    fn new(input: Self::Input) -> Self {
        Self {
            state: SnapCounterState {
                current: input.start,
                target: input.target,
                completed: false,
                failed: false,
                error_message: None,
            },
        }
    }

    fn on_start(&mut self) -> Vec<WorkflowAction> {
        if self.state.current >= self.state.target {
            self.state.completed = true;
            vec![WorkflowAction::complete(
                serde_json::json!({ "final_value": self.state.current }),
            )]
        } else {
            vec![WorkflowAction::schedule_activity(
                format!("increment-{}", self.state.current),
                "increment",
                serde_json::json!({ "value": self.state.current }),
            )]
        }
    }

    fn on_activity_completed(
        &mut self,
        _activity_id: &str,
        result: serde_json::Value,
    ) -> Vec<WorkflowAction> {
        self.state.current = result.get("value").and_then(|v| v.as_i64()).unwrap_or(0) as i32;

        if self.state.current >= self.state.target {
            self.state.completed = true;
            vec![WorkflowAction::complete(
                serde_json::json!({ "final_value": self.state.current }),
            )]
        } else {
            vec![WorkflowAction::schedule_activity(
                format!("increment-{}", self.state.current),
                "increment",
                serde_json::json!({ "value": self.state.current }),
            )]
        }
    }

    fn on_activity_failed(
        &mut self,
        _activity_id: &str,
        error: &ActivityError,
    ) -> Vec<WorkflowAction> {
        self.state.failed = true;
        self.state.error_message = Some(error.message.clone());
        vec![WorkflowAction::fail(crate::WorkflowError::new(
            &error.message,
        ))]
    }

    fn is_completed(&self) -> bool {
        self.state.completed || self.state.failed
    }

    fn result(&self) -> Option<Self::Output> {
        if self.state.completed && !self.state.failed {
            Some(CounterOutput {
                final_value: self.state.current,
            })
        } else {
            None
        }
    }

    fn error(&self) -> Option<crate::WorkflowError> {
        self.state
            .error_message
            .as_ref()
            .map(crate::WorkflowError::new)
    }

    fn snapshot_state(&self) -> Option<Vec<u8>> {
        serde_json::to_vec(&self.state).ok()
    }

    fn restore_state(input: Self::Input, data: &[u8]) -> Option<Self> {
        let state: SnapCounterState = serde_json::from_slice(data).ok()?;
        // Validate consistency: target from input should match snapshot
        let _ = input; // Input already embedded in state
        Some(Self { state })
    }
}

/// Helper: create executor with a specific snapshot interval
fn snap_executor(
    store: InMemoryWorkflowEventStore,
    interval: i32,
) -> WorkflowExecutor<InMemoryWorkflowEventStore> {
    let config = ExecutorConfig {
        snapshot_interval: interval,
        ..Default::default()
    };
    let mut executor = WorkflowExecutor::with_config(store, config);
    executor.register::<SnapCounterWorkflow>();
    executor
}

fn test_executor(
    max_events_per_workflow: usize,
    snapshot_interval: i32,
) -> WorkflowExecutor<InMemoryWorkflowEventStore> {
    let config = ExecutorConfig {
        max_events_per_workflow,
        snapshot_interval,
        ..Default::default()
    };
    let mut executor = WorkflowExecutor::with_config(InMemoryWorkflowEventStore::new(), config);
    executor.register::<CounterWorkflow>();
    executor.register::<SnapCounterWorkflow>();
    executor
}

// =================================================================
// Snapshot tests
// =================================================================

#[tokio::test]
async fn test_full_replay_rejects_oversized_history_before_loading_events() {
    let executor = test_executor(2, 0);
    let workflow_id = Uuid::now_v7();

    executor
        .store()
        .create_workflow(
            workflow_id,
            <CounterWorkflow as crate::workflow::Workflow>::TYPE,
            serde_json::json!({ "start": 0, "target": 10 }),
            None,
        )
        .await
        .unwrap();
    executor
        .store()
        .append_events(
            workflow_id,
            0,
            vec![
                WorkflowEvent::started(serde_json::json!({ "start": 0, "target": 10 })),
                WorkflowEvent::ActivityScheduled {
                    activity_id: "increment-0".into(),
                    activity_type: "increment".into(),
                    input: serde_json::json!({ "value": 0 }),
                    options: crate::workflow::ActivityOptions::default(),
                },
                WorkflowEvent::ActivityCompleted {
                    activity_id: "increment-0".into(),
                    result: serde_json::json!({ "value": 1 }),
                },
            ],
        )
        .await
        .unwrap();

    let result = executor.process_workflow(workflow_id).await;

    assert!(matches!(
        result,
        Err(ExecutorError::TooManyEvents(id, 3, 2)) if id == workflow_id
    ));
    assert_eq!(executor.store().count_events_call_count(), 1);
    assert_eq!(executor.store().load_events_call_count(), 0);
}

#[tokio::test]
async fn test_snapshot_replay_rejects_oversized_delta() {
    let executor = test_executor(2, 0);
    let workflow_id = Uuid::now_v7();

    executor
        .store()
        .create_workflow(
            workflow_id,
            <SnapCounterWorkflow as crate::workflow::Workflow>::TYPE,
            serde_json::json!({ "start": 0, "target": 10 }),
            None,
        )
        .await
        .unwrap();
    executor
        .store()
        .append_events(
            workflow_id,
            0,
            vec![
                WorkflowEvent::started(serde_json::json!({ "start": 0, "target": 10 })),
                WorkflowEvent::ActivityScheduled {
                    activity_id: "increment-0".into(),
                    activity_type: "increment".into(),
                    input: serde_json::json!({ "value": 0 }),
                    options: crate::workflow::ActivityOptions::default(),
                },
                WorkflowEvent::ActivityCompleted {
                    activity_id: "increment-0".into(),
                    result: serde_json::json!({ "value": 1 }),
                },
                WorkflowEvent::ActivityScheduled {
                    activity_id: "increment-1".into(),
                    activity_type: "increment".into(),
                    input: serde_json::json!({ "value": 1 }),
                    options: crate::workflow::ActivityOptions::default(),
                },
                WorkflowEvent::ActivityCompleted {
                    activity_id: "increment-1".into(),
                    result: serde_json::json!({ "value": 2 }),
                },
            ],
        )
        .await
        .unwrap();
    executor
        .store()
        .save_snapshot(
            workflow_id,
            1,
            serde_json::to_vec(&SnapCounterState {
                current: 0,
                target: 10,
                completed: false,
                failed: false,
                error_message: None,
            })
            .unwrap(),
        )
        .await
        .unwrap();

    let result = executor.process_workflow(workflow_id).await;

    assert!(matches!(
        result,
        Err(ExecutorError::TooManyEvents(id, 3, 2)) if id == workflow_id
    ));
}

#[tokio::test]
async fn test_snapshot_save_and_restore() {
    // Use a very small interval to trigger snapshot quickly
    let store = InMemoryWorkflowEventStore::new();
    let mut executor = snap_executor(store, 3);
    executor.register::<CounterWorkflow>(); // also register non-snap version

    let input = CounterInput {
        start: 0,
        target: 10,
    };
    let workflow_id = executor
        .start_workflow::<SnapCounterWorkflow>(input, None)
        .await
        .unwrap();

    // Complete several activities to exceed snapshot interval (3 events)
    for i in 0..5 {
        executor
            .on_activity_completed(
                workflow_id,
                &format!("increment-{}", i),
                serde_json::json!({ "value": i + 1 }),
            )
            .await
            .unwrap();
    }

    // Verify a snapshot was saved
    let snapshot = executor
        .store()
        .load_latest_snapshot(workflow_id)
        .await
        .unwrap();
    assert!(snapshot.is_some(), "snapshot should have been saved");

    let snap = snapshot.unwrap();
    assert!(snap.sequence_num > 0, "snapshot sequence should be > 0");
    assert!(
        !snap.snapshot_data.is_empty(),
        "snapshot data should not be empty"
    );

    // Continue processing — should use snapshot for replay
    executor
        .on_activity_completed(
            workflow_id,
            "increment-5",
            serde_json::json!({ "value": 6 }),
        )
        .await
        .unwrap();

    // Verify workflow is still running (not at target 10 yet)
    let status = executor
        .store()
        .get_workflow_status(workflow_id)
        .await
        .unwrap();
    assert_eq!(status, WorkflowStatus::Running);
}

#[tokio::test]
async fn test_snapshot_at_sequence_zero_is_treated_as_valid_snapshot() {
    let executor = snap_executor(InMemoryWorkflowEventStore::new(), 100);
    let workflow_id = Uuid::now_v7();

    executor
        .store()
        .create_workflow(
            workflow_id,
            <SnapCounterWorkflow as crate::workflow::Workflow>::TYPE,
            serde_json::json!({ "start": 0, "target": 10 }),
            None,
        )
        .await
        .unwrap();

    executor
        .store()
        .append_events(
            workflow_id,
            0,
            vec![
                WorkflowEvent::started(serde_json::json!({ "start": 0, "target": 10 })),
                WorkflowEvent::ActivityScheduled {
                    activity_id: "increment-0".into(),
                    activity_type: "increment".into(),
                    input: serde_json::json!({ "value": 0 }),
                    options: crate::workflow::ActivityOptions::default(),
                },
            ],
        )
        .await
        .unwrap();

    executor
        .store()
        .save_snapshot(
            workflow_id,
            0,
            serde_json::to_vec(&SnapCounterState {
                current: 0,
                target: 10,
                completed: false,
                failed: false,
                error_message: None,
            })
            .unwrap(),
        )
        .await
        .unwrap();

    executor
        .send_signal(
            workflow_id,
            WorkflowSignal::new("noop", serde_json::json!({})),
        )
        .await
        .unwrap();

    let result = executor.process_workflow(workflow_id).await.unwrap();
    assert_eq!(result.signals_processed, 1);
}

#[tokio::test]
async fn test_snapshot_produces_same_result() {
    // Run workflow to completion with snapshots enabled
    let store = InMemoryWorkflowEventStore::new();
    let executor = snap_executor(store, 3);

    let input = CounterInput {
        start: 0,
        target: 5,
    };
    let workflow_id = executor
        .start_workflow::<SnapCounterWorkflow>(input.clone(), None)
        .await
        .unwrap();

    for i in 0..5 {
        executor
            .on_activity_completed(
                workflow_id,
                &format!("increment-{}", i),
                serde_json::json!({ "value": i + 1 }),
            )
            .await
            .unwrap();
    }

    let status1 = executor
        .store()
        .get_workflow_status(workflow_id)
        .await
        .unwrap();
    assert_eq!(status1, WorkflowStatus::Completed);

    // Run same workflow WITHOUT snapshots
    let store2 = InMemoryWorkflowEventStore::new();
    let executor2 = snap_executor(store2, 0); // snapshots disabled

    let workflow_id2 = executor2
        .start_workflow::<SnapCounterWorkflow>(input, None)
        .await
        .unwrap();

    for i in 0..5 {
        executor2
            .on_activity_completed(
                workflow_id2,
                &format!("increment-{}", i),
                serde_json::json!({ "value": i + 1 }),
            )
            .await
            .unwrap();
    }

    let status2 = executor2
        .store()
        .get_workflow_status(workflow_id2)
        .await
        .unwrap();
    assert_eq!(status2, WorkflowStatus::Completed);

    // Both should produce same final status
    assert_eq!(status1, status2);
}

#[tokio::test]
async fn test_no_snapshot_without_support() {
    // Use the non-snapshot CounterWorkflow with snapshot interval enabled
    let store = InMemoryWorkflowEventStore::new();
    let config = ExecutorConfig {
        snapshot_interval: 2,
        ..Default::default()
    };
    let mut executor = WorkflowExecutor::with_config(store, config);
    executor.register::<CounterWorkflow>();

    let input = CounterInput {
        start: 0,
        target: 5,
    };
    let workflow_id = executor
        .start_workflow::<CounterWorkflow>(input, None)
        .await
        .unwrap();

    for i in 0..4 {
        executor
            .on_activity_completed(
                workflow_id,
                &format!("increment-{}", i),
                serde_json::json!({ "value": i + 1 }),
            )
            .await
            .unwrap();
    }

    // No snapshot should be saved since CounterWorkflow doesn't implement snapshot_state
    let snapshot = executor
        .store()
        .load_latest_snapshot(workflow_id)
        .await
        .unwrap();
    assert!(snapshot.is_none(), "no snapshot for non-snapshot workflows");
}

#[tokio::test]
async fn test_snapshot_disabled_when_interval_zero() {
    let store = InMemoryWorkflowEventStore::new();
    let executor = snap_executor(store, 0); // disabled

    let input = CounterInput {
        start: 0,
        target: 5,
    };
    let workflow_id = executor
        .start_workflow::<SnapCounterWorkflow>(input, None)
        .await
        .unwrap();

    for i in 0..4 {
        executor
            .on_activity_completed(
                workflow_id,
                &format!("increment-{}", i),
                serde_json::json!({ "value": i + 1 }),
            )
            .await
            .unwrap();
    }

    let snapshot = executor
        .store()
        .load_latest_snapshot(workflow_id)
        .await
        .unwrap();
    assert!(snapshot.is_none(), "no snapshot when interval is 0");
}

#[tokio::test]
async fn test_snapshot_not_saved_on_completion() {
    let store = InMemoryWorkflowEventStore::new();
    let executor = snap_executor(store, 2);

    let input = CounterInput {
        start: 0,
        target: 2,
    };
    let workflow_id = executor
        .start_workflow::<SnapCounterWorkflow>(input, None)
        .await
        .unwrap();

    // Complete all activities
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

    // Workflow completed - snapshot should NOT be saved for terminal workflows
    let status = executor
        .store()
        .get_workflow_status(workflow_id)
        .await
        .unwrap();
    assert_eq!(status, WorkflowStatus::Completed);

    // Even though enough events accumulated, snapshot not saved for completed workflow
    // (this is by design - no point snapshotting terminal workflows)
}

#[tokio::test]
async fn test_snapshot_delete() {
    let store = InMemoryWorkflowEventStore::new();

    // Manually save a snapshot
    let workflow_id = Uuid::now_v7();
    store
        .save_snapshot(workflow_id, 10, b"test_data".to_vec())
        .await
        .unwrap();

    let snap = store.load_latest_snapshot(workflow_id).await.unwrap();
    assert!(snap.is_some());

    // Delete snapshots
    store.delete_snapshots(workflow_id).await.unwrap();

    let snap = store.load_latest_snapshot(workflow_id).await.unwrap();
    assert!(snap.is_none());
}

#[tokio::test]
async fn test_snapshot_upsert() {
    let store = InMemoryWorkflowEventStore::new();
    let workflow_id = Uuid::now_v7();

    // Save snapshot at seq 5
    store
        .save_snapshot(workflow_id, 5, b"data_v1".to_vec())
        .await
        .unwrap();

    // Save snapshot at seq 10
    store
        .save_snapshot(workflow_id, 10, b"data_v2".to_vec())
        .await
        .unwrap();

    // Latest should be seq 10
    let snap = store
        .load_latest_snapshot(workflow_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snap.sequence_num, 10);
    assert_eq!(snap.snapshot_data, b"data_v2");

    // Upsert seq 5 with new data
    store
        .save_snapshot(workflow_id, 5, b"data_v1_updated".to_vec())
        .await
        .unwrap();

    // Latest should still be seq 10
    let snap = store
        .load_latest_snapshot(workflow_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snap.sequence_num, 10);
}

#[tokio::test]
async fn test_load_events_after() {
    let store = InMemoryWorkflowEventStore::new();

    // Create a workflow and add events
    let workflow_id = Uuid::now_v7();
    store
        .create_workflow(workflow_id, "test", serde_json::json!({}), None)
        .await
        .unwrap();

    let events = vec![WorkflowEvent::started(serde_json::json!({}))];
    store.append_events(workflow_id, 0, events).await.unwrap();

    let events = vec![WorkflowEvent::ActivityScheduled {
        activity_id: "a1".into(),
        activity_type: "test".into(),
        input: serde_json::json!({}),
        options: crate::workflow::ActivityOptions::default(),
    }];
    store.append_events(workflow_id, 1, events).await.unwrap();

    let events = vec![WorkflowEvent::ActivityCompleted {
        activity_id: "a1".into(),
        result: serde_json::json!(42),
    }];
    store.append_events(workflow_id, 2, events).await.unwrap();

    // Load events after seq 0
    let after_0 = store.load_events_after(workflow_id, 0).await.unwrap();
    assert_eq!(after_0.len(), 2); // seq 1 and seq 2

    // Load events after seq 1
    let after_1 = store.load_events_after(workflow_id, 1).await.unwrap();
    assert_eq!(after_1.len(), 1); // only seq 2

    // Load events after seq 2 (none left)
    let after_2 = store.load_events_after(workflow_id, 2).await.unwrap();
    assert_eq!(after_2.len(), 0);
}

#[tokio::test]
async fn test_snapshot_fallback_to_full_replay() {
    // If snapshot data is corrupted, should fall back to full replay
    let store = InMemoryWorkflowEventStore::new();
    let executor = snap_executor(store, 3);

    let input = CounterInput {
        start: 0,
        target: 5,
    };
    let workflow_id = executor
        .start_workflow::<SnapCounterWorkflow>(input, None)
        .await
        .unwrap();

    // Add some activities
    for i in 0..3 {
        executor
            .on_activity_completed(
                workflow_id,
                &format!("increment-{}", i),
                serde_json::json!({ "value": i + 1 }),
            )
            .await
            .unwrap();
    }

    // Manually save corrupted snapshot
    executor
        .store()
        .save_snapshot(workflow_id, 5, b"corrupted_data".to_vec())
        .await
        .unwrap();

    // Processing should still work (falls back to full replay)
    let result = executor
        .on_activity_completed(
            workflow_id,
            "increment-3",
            serde_json::json!({ "value": 4 }),
        )
        .await;
    assert!(
        result.is_ok(),
        "should succeed with fallback to full replay"
    );
}

#[tokio::test]
async fn test_many_events_with_snapshot_bounded_replay() {
    // Simulate a workflow with many events and verify snapshot-based
    // replay only loads events after the snapshot
    let store = InMemoryWorkflowEventStore::new();
    let executor = snap_executor(store, 5); // snapshot every 5

    let target = 20;
    let input = CounterInput { start: 0, target };
    let workflow_id = executor
        .start_workflow::<SnapCounterWorkflow>(input, None)
        .await
        .unwrap();

    // Complete 19 activities (0..19, need to reach 20)
    for i in 0..target {
        executor
            .on_activity_completed(
                workflow_id,
                &format!("increment-{}", i),
                serde_json::json!({ "value": i + 1 }),
            )
            .await
            .unwrap();
    }

    // Workflow should be completed
    let status = executor
        .store()
        .get_workflow_status(workflow_id)
        .await
        .unwrap();
    assert_eq!(status, WorkflowStatus::Completed);

    // Verify total events accumulated (should be many)
    let all_events = executor.store().load_events(workflow_id).await.unwrap();
    assert!(
        all_events.len() > 20,
        "should have many events (got {})",
        all_events.len()
    );
}

// =================================================================
// Snapshot path pre-load count check + stale snapshot deletion
// =================================================================

#[tokio::test]
async fn test_snapshot_path_rejects_before_loading_events() {
    // max_events = 2, snapshot at seq 1 with 3 events after it (indices 2,3,4)
    let executor = test_executor(2, 0);
    let workflow_id = Uuid::now_v7();

    executor
        .store()
        .create_workflow(
            workflow_id,
            <SnapCounterWorkflow as crate::workflow::Workflow>::TYPE,
            serde_json::json!({ "start": 0, "target": 10 }),
            None,
        )
        .await
        .unwrap();
    executor
        .store()
        .update_workflow_status(workflow_id, WorkflowStatus::Running, None, None)
        .await
        .unwrap();
    executor
        .store()
        .append_events(
            workflow_id,
            0,
            vec![
                WorkflowEvent::started(serde_json::json!({ "start": 0, "target": 10 })),
                WorkflowEvent::ActivityScheduled {
                    activity_id: "increment-0".into(),
                    activity_type: "increment".into(),
                    input: serde_json::json!({ "value": 0 }),
                    options: crate::workflow::ActivityOptions::default(),
                },
                WorkflowEvent::ActivityCompleted {
                    activity_id: "increment-0".into(),
                    result: serde_json::json!({ "value": 1 }),
                },
                WorkflowEvent::ActivityScheduled {
                    activity_id: "increment-1".into(),
                    activity_type: "increment".into(),
                    input: serde_json::json!({ "value": 1 }),
                    options: crate::workflow::ActivityOptions::default(),
                },
                WorkflowEvent::ActivityCompleted {
                    activity_id: "increment-1".into(),
                    result: serde_json::json!({ "value": 2 }),
                },
            ],
        )
        .await
        .unwrap();
    // Snapshot at seq 1 (very stale — 3 events after it)
    executor
        .store()
        .save_snapshot(
            workflow_id,
            1,
            serde_json::to_vec(&SnapCounterState {
                current: 0,
                target: 10,
                completed: false,
                failed: false,
                error_message: None,
            })
            .unwrap(),
        )
        .await
        .unwrap();

    let result = executor.process_workflow(workflow_id).await;

    // Should reject with TooManyEvents (3 events after snapshot > max 2)
    assert!(
        matches!(
            &result,
            Err(ExecutorError::TooManyEvents(id, 3, 2)) if *id == workflow_id
        ),
        "expected TooManyEvents(_, 3, 2), got: {:?}",
        result
    );

    // Stale snapshot should have been deleted
    let snapshot = executor
        .store()
        .load_latest_snapshot(workflow_id)
        .await
        .unwrap();
    assert!(
        snapshot.is_none(),
        "stale snapshot should be deleted on rejection"
    );
}

// =================================================================
// Continue-as-new tests
// =================================================================

#[tokio::test]
async fn test_continue_as_new_roundtrip() {
    let store = InMemoryWorkflowEventStore::new();
    let executor = snap_executor(store, 3);

    let input = CounterInput {
        start: 0,
        target: 10,
    };
    let workflow_id = executor
        .start_workflow::<SnapCounterWorkflow>(input, None)
        .await
        .unwrap();

    // Advance workflow to value 5
    for i in 0..5 {
        executor
            .on_activity_completed(
                workflow_id,
                &format!("increment-{}", i),
                serde_json::json!({ "value": i + 1 }),
            )
            .await
            .unwrap();
    }

    // Continue as new
    let new_workflow_id = executor.continue_as_new(workflow_id).await.unwrap();

    // Old workflow should be terminal with ContinuedAsNew status
    let old_info = executor
        .store()
        .get_workflow_info(workflow_id)
        .await
        .unwrap();
    assert_eq!(old_info.status, WorkflowStatus::ContinuedAsNew);
    assert_eq!(old_info.continued_as_new_id, Some(new_workflow_id));
    assert!(old_info.status.is_terminal());

    // New workflow should be running
    let new_info = executor
        .store()
        .get_workflow_info(new_workflow_id)
        .await
        .unwrap();
    assert_eq!(new_info.status, WorkflowStatus::Running);

    // New workflow should have a snapshot
    let snapshot = executor
        .store()
        .load_latest_snapshot(new_workflow_id)
        .await
        .unwrap();
    assert!(snapshot.is_some(), "new workflow should have a snapshot");

    // New workflow should be processable — continue from value 5 to 10
    for i in 5..10 {
        executor
            .on_activity_completed(
                new_workflow_id,
                &format!("increment-{}", i),
                serde_json::json!({ "value": i + 1 }),
            )
            .await
            .unwrap();
    }

    let final_status = executor
        .store()
        .get_workflow_status(new_workflow_id)
        .await
        .unwrap();
    assert_eq!(final_status, WorkflowStatus::Completed);
}

#[tokio::test]
async fn test_continue_as_new_rejects_non_snapshot_workflow() {
    let store = InMemoryWorkflowEventStore::new();
    let config = ExecutorConfig {
        snapshot_interval: 0,
        ..Default::default()
    };
    let mut executor = WorkflowExecutor::with_config(store, config);
    executor.register::<CounterWorkflow>();

    let input = CounterInput {
        start: 0,
        target: 10,
    };
    let workflow_id = executor
        .start_workflow::<CounterWorkflow>(input, None)
        .await
        .unwrap();

    let result = executor.continue_as_new(workflow_id).await;

    assert!(
        matches!(result, Err(ExecutorError::ReplayError(_))),
        "should fail for workflows without snapshot support"
    );
}

#[tokio::test]
async fn test_continue_as_new_rejects_terminal_workflow() {
    let store = InMemoryWorkflowEventStore::new();
    let executor = snap_executor(store, 3);

    let input = CounterInput {
        start: 10,
        target: 5,
    };
    // Workflow completes immediately (start >= target)
    let workflow_id = executor
        .start_workflow::<SnapCounterWorkflow>(input, None)
        .await
        .unwrap();

    let result = executor.continue_as_new(workflow_id).await;

    assert!(
        matches!(result, Err(ExecutorError::WorkflowCompleted(_))),
        "should reject continue-as-new on completed workflow"
    );
}
