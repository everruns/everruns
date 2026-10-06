//! `DurableBackend` on PostgreSQL: routing on a shared queue and ending the
//! workflows a gone backend left behind.
//!
//! These run against `DATABASE_URL` and return early, passing, when it is
//! unset, unless `EVERRUNS_REQUIRE_POSTGRES_TESTS` is set (CI's durable
//! PostgreSQL shard sets it), which makes the missing URL a failure. A set
//! URL that does not connect fails them. They share the database
//! with whatever else uses it: every session and routing key is fresh, and
//! nothing is truncated.

use std::time::Duration;

use everruns_contracts::tool_types::ToolCall;
use everruns_contracts::typed_id::{SessionId, TurnId};
use everruns_durable::{
    Pagination, PostgresWorkflowEventStore, TaskFilter, TaskInfo, TaskQueue, TaskStatus,
    WorkerInfo, WorkerRegistry, WorkflowStatus,
};
use everruns_llmsim::{LlmSimConfig, LlmSimRuntimeExt};

use crate::core::InputMessage;
use crate::durable_backend::{DurableBackend, LEFT_BEHIND};
use crate::host::{
    AcceptedTurnInput, InProcessRuntime, TurnBackend, TurnInput, TurnRequest, TurnTicket,
};
use crate::task_store::TaskStore;

/// The test database, or `None` (the test skips) without `DATABASE_URL`.
async fn store() -> Option<PostgresWorkflowEventStore> {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        assert!(
            !require_postgres(),
            "EVERRUNS_REQUIRE_POSTGRES_TESTS is set but DATABASE_URL is not"
        );
        eprintln!("DATABASE_URL is unset; skipping the PostgreSQL durable backend test");
        return None;
    };
    Some(
        PostgresWorkflowEventStore::connect(&url)
            .await
            .expect("DATABASE_URL connects and takes the durable schema"),
    )
}

/// Whether a missing `DATABASE_URL` fails instead of skipping. Decision: an
/// env flag, the shape of `EVERRUNS_REQUIRE_LIVE_TESTS`, rather than a cargo
/// feature, so the CI job that has a database cannot report a vacuous pass.
/// The facade's `backend_conformance` suite reads the same flag; the check is
/// repeated there instead of shared because the two crates have no common
/// test-only dependency to host it.
fn require_postgres() -> bool {
    std::env::var("EVERRUNS_REQUIRE_POSTGRES_TESTS").is_ok_and(|v| {
        let v = v.trim();
        !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false")
    })
}

async fn runtime(sim: LlmSimConfig) -> (InProcessRuntime, SessionId) {
    let runtime = InProcessRuntime::builder()
        .llm_sim_as_default(sim)
        .single_session(|session| session)
        .build()
        .await
        .unwrap();
    let session_id = runtime.default_session_id().unwrap();
    (runtime, session_id)
}

fn message(session_id: SessionId, text: &str) -> TurnRequest {
    TurnRequest::new(
        session_id,
        TurnId::new(),
        TurnInput::Message(Box::new(AcceptedTurnInput::new(InputMessage::user(text)))),
    )
}

async fn finish(ticket: TurnTicket) -> crate::host::TurnResult {
    tokio::time::timeout(Duration::from_secs(10), ticket)
        .await
        .expect("the durable turn ends")
        .expect("the durable turn succeeds")
}

async fn tasks(store: &PostgresWorkflowEventStore, session_id: SessionId) -> Vec<TaskInfo> {
    store
        .list_tasks(
            TaskFilter {
                workflow_id: Some(session_id.uuid()),
                ..TaskFilter::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn a_tool_turn_runs_on_postgres_with_every_step_routed() {
    let Some(store) = store().await else { return };
    let (runtime, session_id) = runtime(
        LlmSimConfig::sequence(vec!["checking".into(), "tool done".into()])
            .with_tool_call_sequence(vec![
                vec![ToolCall {
                    id: "call-1".into(),
                    name: "no_such_tool".into(),
                    arguments: serde_json::json!({}),
                }],
                vec![],
            ]),
    )
    .await;
    let backend = DurableBackend::postgres(store.clone(), 2);
    let session = backend.attach(session_id, runtime);

    let result = finish(session.start_turn(message(session_id, "hi")).await.unwrap()).await;
    assert!(result.success, "{result:?}");
    assert_eq!(result.response, "tool done");
    assert_eq!(result.tool_calls_count, 1);
    assert_eq!(result.iterations, 2);
    assert!(!session.is_running(session_id).await);

    // Input with its first reason, act, reason: each tagged with this
    // backend's key.
    let steps = tasks(&store, session_id).await;
    let mut types: Vec<&str> = steps
        .iter()
        .map(|task| task.activity_type.as_str())
        .collect();
    types.sort_unstable();
    let plain: Vec<&str> = types
        .iter()
        .map(|activity| activity.split_once('@').expect("routed").0)
        .collect();
    assert_eq!(plain, ["act", "process_input", "reason"]);
    let key = types[0].split_once('@').unwrap().1;
    assert!(
        types.iter().all(|activity| activity.ends_with(key)),
        "{types:?}"
    );

    // The session takes its next turn.
    let next = finish(
        session
            .start_turn(message(session_id, "again"))
            .await
            .unwrap(),
    )
    .await;
    assert!(next.success, "{next:?}");
    backend.shutdown().await;
}

#[tokio::test]
async fn a_backend_never_claims_another_backends_steps() {
    let Some(store) = store().await else { return };
    // `stopped` queues its session's turn but runs no workers, standing in
    // for a process whose workers are busy elsewhere.
    let stopped = DurableBackend::postgres(store.clone(), 1);
    stopped.shutdown().await;
    let (stopped_runtime, stopped_id) = runtime(LlmSimConfig::fixed("not here")).await;
    let stopped_session = stopped.attach(stopped_id, stopped_runtime);

    let running = DurableBackend::postgres(store.clone(), 2);
    let (runtime, session_id) = runtime(LlmSimConfig::fixed("here")).await;
    let session = running.attach(session_id, runtime);

    let _queued = stopped_session
        .start_turn(message(stopped_id, "wait"))
        .await
        .unwrap();
    let result = finish(session.start_turn(message(session_id, "go")).await.unwrap()).await;
    assert_eq!(result.response, "here");
    // A worker that claimed the other session's step could not run it and
    // would have failed it; it is still waiting for its own backend.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let steps = tasks(&store, stopped_id).await;
    assert_eq!(steps.len(), 1, "{steps:?}");
    assert_eq!(steps[0].status, TaskStatus::Pending, "{steps:?}");
    assert!(stopped_session.is_running(stopped_id).await);
    running.shutdown().await;
}

#[tokio::test]
async fn a_session_attached_again_ends_the_workflow_a_gone_backend_left_behind() {
    let Some(store) = store().await else { return };
    let (runtime, session_id) = runtime(LlmSimConfig::fixed("recovered")).await;

    // The gone backend: its turn's first step was claimed by a worker that
    // then died with the process, holding it.
    let gone = DurableBackend::postgres(store.clone(), 1);
    gone.shutdown().await;
    let gone_session = gone.attach(session_id, runtime.clone());
    let _lost = gone_session
        .start_turn(message(session_id, "lost"))
        .await
        .unwrap();
    let held = tasks(&store, session_id).await;
    assert_eq!(held.len(), 1, "{held:?}");
    let dead_worker = format!("dead-{}", uuid::Uuid::now_v7());
    WorkerRegistry::register_worker(
        &store,
        WorkerInfo::new(dead_worker.clone(), [held[0].activity_type.clone()]),
    )
    .await
    .unwrap();
    let claimed = TaskQueue::claim_task(
        &store,
        &dead_worker,
        std::slice::from_ref(&held[0].activity_type),
        1,
    )
    .await
    .unwrap();
    assert_eq!(claimed.len(), 1, "the dead worker holds the step");
    drop(gone_session);
    drop(gone);

    // The session comes back on a new backend, sharing the database.
    let backend = DurableBackend::postgres(store.clone(), 1);
    let session = backend.attach(session_id, runtime);
    assert!(
        session.is_running(session_id).await,
        "the left-behind workflow still runs"
    );
    let result = finish(
        session
            .start_turn(message(session_id, "again"))
            .await
            .unwrap(),
    )
    .await;
    assert!(result.success, "{result:?}");
    assert_eq!(result.response, "recovered");

    let held = store.get_task(claimed[0].id).await.unwrap();
    assert_ne!(held.status, TaskStatus::Claimed, "{held:?}");
    assert_eq!(held.last_error.as_deref(), Some(LEFT_BEHIND));
    assert_eq!(
        TaskStore::get_workflow_status(&store, session_id.uuid())
            .await
            .unwrap(),
        WorkflowStatus::Completed
    );
    backend.shutdown().await;
}
