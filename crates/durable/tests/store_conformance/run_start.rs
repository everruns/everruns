//! Run start: [`EventLog::start_run_with_task`] cases of the store
//! conformance suite (split out of `store_conformance_test.rs` for size).

use super::*;

pub(crate) async fn start_run<H: Harness>(
    h: &H,
    wf: Uuid,
    ty: &str,
    activity_id: &str,
) -> RunStart {
    start_steered_run(h, wf, ty, activity_id, None).await
}

pub(crate) async fn start_steered_run<H: Harness>(
    h: &H,
    wf: Uuid,
    ty: &str,
    activity_id: &str,
    steering: Option<RunSteering>,
) -> RunStart {
    h.store()
        .start_run_with_task(
            wf,
            "conformance",
            json!({ "run": activity_id }),
            task(None, ty, activity_id),
            steering,
        )
        .await
        .expect("start run")
}

/// The steering of a run start: an active run gets the signal under the
/// lock that found it active, and a new run consumes what the previous run
/// left pending of that type (only that type).
pub(crate) async fn start_run_steers_the_active_run_or_absorbs_its_signals<H: Harness>(h: H) {
    let ty = activity_type();
    let wf = Uuid::now_v7();
    let steer = |n: i64| RunSteering {
        signal_type: "steer".into(),
        payload: Some(json!({ "n": n })),
    };
    start_run(&h, wf, &ty, "first").await;
    assert_eq!(
        start_steered_run(&h, wf, &ty, "second", Some(steer(1))).await,
        RunStart::Active
    );
    assert_eq!(
        start_steered_run(&h, wf, &ty, "third", Some(steer(2))).await,
        RunStart::Active
    );
    let pending = h.store().get_pending_signals(wf).await.unwrap();
    let payloads: Vec<_> = pending.iter().map(|s| s.payload.clone()).collect();
    assert_eq!(payloads, vec![json!({ "n": 1 }), json!({ "n": 2 })]);
    assert!(pending.iter().all(|s| s.signal_type == "steer"));

    // The run ends with the steering still pending, plus another type.
    h.store()
        .send_signal(wf, WorkflowSignal::new("other", json!({})))
        .await
        .unwrap();
    h.store()
        .update_workflow_status(wf, WorkflowStatus::Completed, None, None)
        .await
        .unwrap();

    // A start without a payload sends nothing; a new run takes the steering.
    let quiet = RunSteering {
        signal_type: "steer".into(),
        payload: None,
    };
    assert!(matches!(
        start_steered_run(&h, wf, &ty, "next", Some(quiet)).await,
        RunStart::Started { created: false, .. }
    ));
    let left = h.store().get_pending_signals(wf).await.unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].signal_type, "other");
}

pub(crate) async fn start_run_creates_an_unknown_workflow<H: Harness>(h: H) {
    let ty = activity_type();
    let w = worker(&h, &ty).await;
    let wf = Uuid::now_v7();

    let RunStart::Started { task_id, created } = start_run(&h, wf, &ty, "first").await else {
        panic!("an unknown workflow starts a run");
    };
    assert!(created);
    assert_eq!(
        h.store().get_workflow_status(wf).await.unwrap(),
        WorkflowStatus::Running
    );
    let events = h.store().load_events(wf).await.unwrap();
    assert!(matches!(events[0].1, WorkflowEvent::WorkflowStarted { .. }));
    assert!(matches!(
        &events[1].1,
        WorkflowEvent::ActivityScheduled { activity_id, .. } if activity_id == "first"
    ));
    let claimed = h
        .store()
        .claim_task(&w, std::slice::from_ref(&ty), 10)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, task_id);
    assert_eq!(claimed[0].workflow_id, Some(wf));
}

pub(crate) async fn start_run_leaves_an_active_run_alone<H: Harness>(h: H) {
    let ty = activity_type();
    let w = worker(&h, &ty).await;

    // Running workflow: the second start changes nothing.
    let running = Uuid::now_v7();
    start_run(&h, running, &ty, "first").await;
    assert_eq!(
        start_run(&h, running, &ty, "second").await,
        RunStart::Active
    );
    assert_eq!(claim(&h, &w, &ty, 10).await.len(), 1);

    // Not Running, but a worker still holds one of its tasks.
    let claimed = workflow(&h).await;
    enqueue(&h, task(Some(claimed), &ty, "held")).await;
    assert_eq!(claim(&h, &w, &ty, 1).await.len(), 1);
    assert_eq!(start_run(&h, claimed, &ty, "next").await, RunStart::Active);
    assert_eq!(
        h.store().get_workflow_status(claimed).await.unwrap(),
        WorkflowStatus::Pending
    );
    assert!(claim(&h, &w, &ty, 10).await.is_empty());
}

pub(crate) async fn start_run_restarts_a_finished_workflow<H: Harness>(h: H) {
    let ty = activity_type();
    let w = worker(&h, &ty).await;
    let wf = workflow(&h).await;
    let stale = enqueue(&h, task(Some(wf), &ty, "stale")).await;
    h.store()
        .update_workflow_status(wf, WorkflowStatus::Completed, Some(json!("done")), None)
        .await
        .unwrap();

    let RunStart::Started { task_id, created } = start_run(&h, wf, &ty, "next").await else {
        panic!("a finished workflow starts a new run");
    };
    assert!(!created);
    let info = h.store().get_workflow_info(wf).await.unwrap();
    assert_eq!(info.status, WorkflowStatus::Running);
    assert_eq!(info.result, None);
    assert_eq!(status(&h, stale).await, TaskStatus::Cancelled);
    assert_eq!(claim(&h, &w, &ty, 10).await, vec![task_id]);
}

pub(crate) async fn concurrent_run_starts_elect_one_winner<H: Harness>(h: H) {
    let ty = activity_type();
    let w = worker(&h, &ty).await;
    for existing in [false, true] {
        let wf = if existing {
            let wf = workflow(&h).await;
            h.store()
                .update_workflow_status(wf, WorkflowStatus::Completed, None, None)
                .await
                .unwrap();
            wf
        } else {
            Uuid::now_v7()
        };
        let one = || start_run(&h, wf, &ty, "race");
        let (a, b, c, d) = tokio::join!(one(), one(), one(), one());
        let winners = [a, b, c, d]
            .iter()
            .filter(|r| matches!(r, RunStart::Started { .. }))
            .count();
        assert_eq!(winners, 1, "existing={existing}");
        assert_eq!(claim(&h, &w, &ty, 10).await.len(), 1, "existing={existing}");
    }
}
