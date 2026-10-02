//! Durable timers and child workflows, end to end through the executor.
//!
//! Each case runs against the in-memory store and, with `postgres-tests`,
//! against PostgreSQL (DATABASE_URL, as in `postgres_integration_test`). The
//! engine's own tasks are claimed by a worker registered for
//! `SYSTEM_ACTIVITY_TYPES`, as a deployment would run them.

use std::time::Duration;

use everruns_durable::prelude::*;
use everruns_durable::{SYSTEM_ACTIVITY_TYPES, TaskDefinition};
use serde_json::{Value, json};
use uuid::Uuid;

// --- workflows ---------------------------------------------------------------

/// Sleeps on a timer, then completes with how many times it woke.
struct Sleeper {
    woke: u32,
}

impl Workflow for Sleeper {
    const TYPE: &'static str = "test_sleeper";
    type Input = Value;
    type Output = Value;

    fn new(_: Value) -> Self {
        Self { woke: 0 }
    }
    fn on_start(&mut self) -> Vec<WorkflowAction> {
        vec![WorkflowAction::timer("nap", Duration::from_millis(200))]
    }
    fn on_timer_fired(&mut self, _: &str) -> Vec<WorkflowAction> {
        self.woke += 1;
        vec![WorkflowAction::complete(json!(self.woke))]
    }
    fn on_activity_completed(&mut self, _: &str, _: Value) -> Vec<WorkflowAction> {
        vec![]
    }
    fn on_activity_failed(&mut self, _: &str, _: &ActivityError) -> Vec<WorkflowAction> {
        vec![]
    }
    fn is_completed(&self) -> bool {
        self.woke > 0
    }
    fn result(&self) -> Option<Value> {
        (self.woke > 0).then(|| json!(self.woke))
    }
}

/// Runs one activity; completes with its result or fails with its error.
struct Child {
    outcome: Option<Result<Value, WorkflowError>>,
}

impl Workflow for Child {
    const TYPE: &'static str = "test_child";
    type Input = Value;
    type Output = Value;

    fn new(_: Value) -> Self {
        Self { outcome: None }
    }
    fn on_start(&mut self) -> Vec<WorkflowAction> {
        vec![WorkflowAction::schedule_activity(
            "work",
            "child_work",
            json!({}),
        )]
    }
    fn on_activity_completed(&mut self, _: &str, result: Value) -> Vec<WorkflowAction> {
        self.outcome = Some(Ok(result.clone()));
        vec![WorkflowAction::complete(result)]
    }
    fn on_activity_failed(&mut self, _: &str, error: &ActivityError) -> Vec<WorkflowAction> {
        let error = WorkflowError::new(&error.message).with_code("child_failed");
        self.outcome = Some(Err(error.clone()));
        vec![WorkflowAction::fail(error)]
    }
    fn is_completed(&self) -> bool {
        self.outcome.is_some()
    }
    fn result(&self) -> Option<Value> {
        self.outcome.clone()?.ok()
    }
    fn error(&self) -> Option<WorkflowError> {
        self.outcome.clone()?.err()
    }
}

/// Starts a child of the type named in its input and finishes as the child did.
struct Parent {
    child_type: String,
    outcome: Option<Result<Value, WorkflowError>>,
}

impl Workflow for Parent {
    const TYPE: &'static str = "test_parent";
    type Input = String;
    type Output = Value;

    fn new(child_type: String) -> Self {
        Self {
            child_type,
            outcome: None,
        }
    }
    fn on_start(&mut self) -> Vec<WorkflowAction> {
        let child = WorkflowAction::child_workflow("kid", &self.child_type, json!({}));
        vec![child]
    }
    fn on_child_workflow_completed(
        &mut self,
        child_id: &str,
        result: Value,
    ) -> Vec<WorkflowAction> {
        let result = json!({ "child": child_id, "result": result });
        self.outcome = Some(Ok(result.clone()));
        vec![WorkflowAction::complete(result)]
    }
    fn on_child_workflow_failed(&mut self, _: &str, error: &WorkflowError) -> Vec<WorkflowAction> {
        self.outcome = Some(Err(error.clone()));
        vec![WorkflowAction::fail(error.clone())]
    }
    fn on_activity_completed(&mut self, _: &str, _: Value) -> Vec<WorkflowAction> {
        vec![]
    }
    fn on_activity_failed(&mut self, _: &str, _: &ActivityError) -> Vec<WorkflowAction> {
        vec![]
    }
    fn is_completed(&self) -> bool {
        self.outcome.is_some()
    }
    fn result(&self) -> Option<Value> {
        self.outcome.clone()?.ok()
    }
    fn error(&self) -> Option<WorkflowError> {
        self.outcome.clone()?.err()
    }
}

// --- harness -----------------------------------------------------------------

/// An executor with all test workflows registered and a worker for the
/// engine's tasks, returned with that worker's id.
async fn executor<S: WorkflowEventStore>(store: S) -> (WorkflowExecutor<S>, String) {
    let mut executor = WorkflowExecutor::new(store);
    executor.register::<Sleeper>();
    executor.register::<Child>();
    executor.register::<Parent>();
    let worker = format!("engine-{}", Uuid::now_v7().simple());
    let info = WorkerInfo::new(worker.clone(), SYSTEM_ACTIVITY_TYPES);
    executor.store().register_worker(info).await.unwrap();
    (executor, worker)
}

async fn events<S: WorkflowEventStore>(e: &WorkflowExecutor<S>, id: Uuid) -> Vec<WorkflowEvent> {
    let events = e.store().load_events(id).await.unwrap();
    events.into_iter().map(|(_, event)| event).collect()
}

/// Run the engine's tasks until none are left, as a polling loop would.
/// A shared database may hold other runs' tasks, so this only drains.
async fn drain<S: WorkflowEventStore>(e: &WorkflowExecutor<S>, worker: &str) {
    while e.run_system_tasks(worker, 50).await.unwrap() > 0 {}
}

/// The child the parent started: the only `ChildWorkflowStarted` in its history.
async fn child_of<S: WorkflowEventStore>(e: &WorkflowExecutor<S>, parent: Uuid) -> Uuid {
    let started: Vec<Uuid> = events(e, parent)
        .await
        .into_iter()
        .filter_map(|event| match event {
            WorkflowEvent::ChildWorkflowStarted { workflow_id, .. } => Some(workflow_id),
            _ => None,
        })
        .collect();
    assert_eq!(started.len(), 1, "one child started");
    started[0]
}

// --- cases -------------------------------------------------------------------

async fn timer_fires_when_due_and_only_once<S: WorkflowEventStore>(store: S) {
    let (e, worker) = executor(store).await;
    let id = e.start_workflow::<Sleeper>(json!({}), None).await.unwrap();

    drain(&e, &worker).await;
    let status = e.store().get_workflow_status(id).await.unwrap();
    assert_eq!(status, WorkflowStatus::Running, "the timer is not due yet");

    tokio::time::sleep(Duration::from_millis(300)).await;
    drain(&e, &worker).await;
    let info = e.store().get_workflow_info(id).await.unwrap();
    assert_eq!(info.status, WorkflowStatus::Completed);
    assert_eq!(info.result, Some(json!(1)));

    // A redelivered timer task (a worker crashed after firing) is a no-op.
    let redelivered = TaskDefinition {
        workflow_id: Some(id),
        activity_id: "nap".into(),
        activity_type: SYSTEM_ACTIVITY_TYPES[0].into(),
        input: Value::Null,
        options: ActivityOptions::default(),
    };
    e.store().enqueue_task(redelivered).await.unwrap();
    drain(&e, &worker).await;
    let fired = events(&e, id).await.into_iter();
    let fired = fired.filter(|ev| matches!(ev, WorkflowEvent::TimerFired { .. }));
    assert_eq!(fired.count(), 1);
}

async fn parent_receives_child_result<S: WorkflowEventStore>(store: S) {
    let (e, worker) = executor(store).await;
    let parent = e
        .start_workflow::<Parent>(Child::TYPE.to_string(), None)
        .await
        .unwrap();
    drain(&e, &worker).await;

    let child = child_of(&e, parent).await;
    let status = e.store().get_workflow_status(child).await.unwrap();
    assert_eq!(status, WorkflowStatus::Running, "the child was started");

    // Replaying the parent does not schedule the child a second time.
    e.process_workflow(parent).await.unwrap();
    drain(&e, &worker).await;
    child_of(&e, parent).await;

    e.on_activity_completed(child, "work", json!("done"))
        .await
        .unwrap();
    drain(&e, &worker).await;

    let info = e.store().get_workflow_info(parent).await.unwrap();
    assert_eq!(info.status, WorkflowStatus::Completed);
    assert_eq!(
        info.result,
        Some(json!({ "child": "kid", "result": "done" }))
    );
}

async fn parent_receives_child_failure<S: WorkflowEventStore>(store: S) {
    let (e, worker) = executor(store).await;
    let parent = e
        .start_workflow::<Parent>(Child::TYPE.to_string(), None)
        .await
        .unwrap();
    drain(&e, &worker).await;
    let child = child_of(&e, parent).await;

    let error = ActivityError::non_retryable("no capacity");
    e.on_activity_failed(child, "work", error, false)
        .await
        .unwrap();
    drain(&e, &worker).await;

    let info = e.store().get_workflow_info(parent).await.unwrap();
    assert_eq!(info.status, WorkflowStatus::Failed);
    assert_eq!(info.error.unwrap().code.as_deref(), Some("child_failed"));
}

async fn unregistered_child_type_fails_the_parent<S: WorkflowEventStore>(store: S) {
    let (e, worker) = executor(store).await;
    let parent = e
        .start_workflow::<Parent>("not_registered".to_string(), None)
        .await
        .unwrap();
    drain(&e, &worker).await;

    let info = e.store().get_workflow_info(parent).await.unwrap();
    assert_eq!(info.status, WorkflowStatus::Failed);
    let code = info.error.unwrap().code;
    assert_eq!(code.as_deref(), Some("child_not_started"));
}

macro_rules! cases {
    ($($case:ident),* $(,)?) => {
        mod memory {
            $(
                #[tokio::test]
                async fn $case() {
                    let store = everruns_durable::InMemoryWorkflowEventStore::new();
                    super::$case(store).await;
                }
            )*
        }

        #[cfg(feature = "postgres-tests")]
        mod postgres {
            $(
                #[tokio::test]
                async fn $case() {
                    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
                        "postgres://postgres:postgres@localhost:5432/everruns_test".to_string()
                    });
                    let pool = sqlx::PgPool::connect(&url).await.expect("connect to PostgreSQL");
                    let store = everruns_durable::PostgresWorkflowEventStore::new(pool);
                    super::$case(store).await;
                }
            )*
        }
    };
}

cases!(
    timer_fires_when_due_and_only_once,
    parent_receives_child_result,
    parent_receives_child_failure,
    unregistered_child_type_fails_the_parent,
);
