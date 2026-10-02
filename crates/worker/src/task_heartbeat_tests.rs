//! The task heartbeat tells an explicit turn cancel apart from losing the task
//! (EVE-1134). Only the former may stop provider work: after an ownership loss
//! the next owner re-attaches to the same background response.

use std::sync::Arc;
use std::time::Duration;

use everruns_durable::persistence::{
    InMemoryWorkflowEventStore, TaskDefinition, WorkflowEventStore, WorkflowStatus,
};
use everruns_durable::workflow::ActivityOptions;
use tokio::sync::watch;
use uuid::Uuid;

use crate::task_heartbeat::spawn_task_heartbeat;

async fn fired(signal: &mut watch::Receiver<bool>) -> bool {
    tokio::time::timeout(Duration::from_secs(2), signal.wait_for(|fired| *fired))
        .await
        .is_ok_and(|result| result.is_ok())
}

#[tokio::test]
async fn cancelled_workflow_fires_both_the_task_and_the_turn_cancel() {
    let store = Arc::new(InMemoryWorkflowEventStore::new());
    let workflow_id = Uuid::now_v7();
    store
        .create_workflow(workflow_id, "turn", serde_json::json!({}), None)
        .await
        .unwrap();
    let task_id = store
        .enqueue_task(TaskDefinition {
            workflow_id: Some(workflow_id),
            activity_id: "reason-1".into(),
            activity_type: "reason".into(),
            input: serde_json::json!({}),
            options: ActivityOptions::default(),
        })
        .await
        .unwrap();
    store
        .register_worker(everruns_durable::WorkerInfo::new(
            "worker-1",
            Vec::<String>::new(),
        ))
        .await
        .expect("register worker");
    store
        .claim_task("worker-1", &["reason".to_string()], 1)
        .await
        .unwrap();
    let (stop, handle, (mut task_cancel, mut turn_cancel)) = spawn_task_heartbeat(
        store.clone(),
        task_id,
        "worker-1".into(),
        Duration::from_millis(10),
    );
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(!*turn_cancel.borrow(), "a running turn is not cancelled");

    store
        .update_workflow_status(workflow_id, WorkflowStatus::Cancelled, None, None)
        .await
        .unwrap();
    assert!(
        fired(&mut turn_cancel).await,
        "the turn cancel reaches the worker"
    );
    assert!(fired(&mut task_cancel).await);
    let _ = stop.send(());
    handle.await.unwrap();
}

#[tokio::test]
async fn failed_heartbeat_cancels_the_task_but_not_the_turn() {
    // An unknown task makes every heartbeat fail, as a store outage would.
    let store = Arc::new(InMemoryWorkflowEventStore::new());
    let (stop, handle, (mut task_cancel, turn_cancel)) = spawn_task_heartbeat(
        store,
        Uuid::now_v7(),
        "worker-1".into(),
        Duration::from_millis(10),
    );
    assert!(fired(&mut task_cancel).await);
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(
        !*turn_cancel.borrow(),
        "losing the store is not a reason to cancel the provider response"
    );
    let _ = stop.send(());
    handle.await.unwrap();
}
