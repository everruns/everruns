//! Dead-letter cases of the store conformance suite.

use super::*;

pub(super) async fn dead_letters_can_be_requeued<H: Harness>(h: H) {
    let ty = activity_type();
    let w = worker(&h, &ty).await;
    let wf = workflow(&h).await;
    let id = enqueue(&h, task(Some(wf), &ty, "flaky")).await;
    claim(&h, &w, &ty, 1).await;
    h.store()
        .fail_task_with_retry(id, "boom", false)
        .await
        .unwrap();
    h.store()
        .move_to_dlq(id, vec!["boom".to_string()])
        .await
        .unwrap();

    let entries = h
        .store()
        .list_dlq(
            everruns_durable::persistence::DlqFilter {
                workflow_id: Some(wf),
                activity_type: None,
            },
            Default::default(),
        )
        .await
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].original_task_id, id);

    let requeued = h.store().requeue_from_dlq(entries[0].id).await.unwrap();
    assert_ne!(requeued, id);
    assert_eq!(claim(&h, &w, &ty, 1).await, vec![requeued]);
}

pub(super) async fn requeued_dead_letters_keep_their_queue_and_options<H: Harness>(h: H) {
    let ty = activity_type();
    let w = worker(&h, &ty).await;
    let queue = format!("conformance-dlq-{}", Uuid::now_v7().simple());
    let mut options = ActivityOptions::default().with_queue(queue.clone());
    options.priority = 7;
    let id = enqueue(&h, with_options(task(None, &ty, "routed"), options)).await;
    let types = vec![ty.clone()];
    let claimed = h
        .store()
        .claim_queue_tasks(&w, Some(&queue), &types, 1)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);
    h.store()
        .fail_task_with_retry(id, "boom", false)
        .await
        .unwrap();
    h.store()
        .move_to_dlq(id, vec!["boom".to_string()])
        .await
        .unwrap();
    let entry = h
        .store()
        .list_dlq(
            everruns_durable::persistence::DlqFilter {
                workflow_id: None,
                activity_type: Some(ty.clone()),
            },
            Default::default(),
        )
        .await
        .unwrap()
        .remove(0);

    let requeued = h.store().requeue_from_dlq(entry.id).await.unwrap();

    // The requeued task goes back to its own queue, not the default one, and
    // keeps the options it was enqueued with.
    assert!(claim(&h, &w, &ty, 10).await.is_empty());
    let claimed = h
        .store()
        .claim_queue_tasks(&w, Some(&queue), &types, 10)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, requeued);
    assert_eq!(claimed[0].options.queue.as_deref(), Some(queue.as_str()));
    assert_eq!(claimed[0].options.priority, 7);
}
