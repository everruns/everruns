//! A worker dies while its `reason` step is in flight: the task is reclaimed
//! once its lease (the heartbeat) goes stale, a new worker runs it, and the
//! turn completes with every message and step completion recorded once.
//!
//! This is a crash at the turn level, above the durable crate's failpoints:
//! worker A runs the turn's real steps through [`TurnTaskDriver`] until its
//! `reason` step blocks inside the model call, and is then killed by dropping
//! its task mid-await. Nothing it held is released: the claim stays with A
//! and its heartbeat simply stops, as when the process dies. The stale-task
//! reaper (`reap_stale_tasks`, what the server runs) puts the step back once
//! the heartbeat is older than the threshold, and worker B claims it as
//! attempt 2.
//!
//! Decisions:
//! - Heartbeats every 50 ms and a 400 ms stale threshold, so the whole test
//!   takes about a second; the production defaults are seconds and minutes.
//! - The model call that blocks is the second one, the `reason` task after
//!   the act step. The first reason runs inside `process_input`, whose retry
//!   would also replay turn start; the separate `reason` task is the step a
//!   worker most often holds while a model is slow.
//! - Each test runs its own task queue ([`Routing::unique`]) so the
//!   PostgreSQL variant can share a database with other suites and never
//!   claims their rows. The reaper itself is store-wide, as in production,
//!   and the test asserts only on its own task. It would also reclaim
//!   another suite's claim left unheartbeated for 400 ms, which is why the
//!   PostgreSQL suites run with `--test-threads=1`.
//! - The PostgreSQL variant runs against `DATABASE_URL` and skips, passing,
//!   without it unless `EVERRUNS_REQUIRE_POSTGRES_TESTS` is set (CI's durable
//!   PostgreSQL shard sets it), like `durable_backend_postgres_tests`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::driver_registry::{ChatDriver, LlmCallConfig, LlmResponseStream, Message};
use everruns_contracts::error::Result as LlmResult;
use everruns_contracts::runtime_provider::{Provider, ProviderEndpoint};
use everruns_contracts::tool_types::ToolCall;
use everruns_contracts::typed_id::{SessionId, TurnId};
use everruns_llmsim::{LLMSIM_MODEL_ID, LlmSimConfig, LlmSimDriver};
use tokio::sync::Notify;
use uuid::Uuid;

use crate::backend_store::{RoutedStore, Routing};
use crate::core::InputMessage;
use crate::durable::{
    EventLog, InMemoryWorkflowEventStore, NoopReapHandler, PostgresWorkflowEventStore, TaskStatus,
    WorkerInfo, WorkflowEventStore, WorkflowStatus, reap_stale_tasks,
};
use crate::durable_runner::DurableTurnInput;
use crate::engine::ReasonInput;
use crate::host::{
    AcceptedTurnInput, InProcessRuntime, RuntimeHostAdapter, in_process_internal_org_id,
};
use crate::task_heartbeat::CancelSignals;
use crate::turn_driver::{TurnTaskDriver, TurnTaskHost};
use crate::turn_store::TurnStore;

const TURN_ACTIVITIES: [&str; 3] = ["process_input", "reason", "act"];
const HEARTBEAT: Duration = Duration::from_millis(50);
const STALE_AFTER: Duration = Duration::from_millis(400);
/// The model call that never answers: the `reason` task after the act.
const BLOCKED_CALL: usize = 1;

#[tokio::test]
async fn a_reason_step_a_dead_worker_held_is_reclaimed_and_completes_once_on_memory() {
    crash_mid_reason(Arc::new(InMemoryWorkflowEventStore::new())).await;
}

#[tokio::test]
async fn a_reason_step_a_dead_worker_held_is_reclaimed_and_completes_once_on_postgres() {
    let Some(store) = postgres().await else {
        return;
    };
    crash_mid_reason(Arc::new(store)).await;
}

async fn crash_mid_reason<S: WorkflowEventStore + 'static>(shared: Arc<S>) {
    let entered = Arc::new(Notify::new());
    let llm = GatedDriver {
        calls: AtomicUsize::new(0),
        entered: entered.clone(),
        inner: LlmSimDriver::new(
            LlmSimConfig::sequence(vec!["checking".into(), "survived the crash".into()])
                .with_tool_call_sequence(vec![
                    vec![ToolCall {
                        id: "call-1".into(),
                        name: "no_such_tool".into(),
                        arguments: serde_json::json!({}),
                    }],
                    vec![],
                ]),
        ),
    };
    let runtime = InProcessRuntime::builder()
        .provider_with_default_model(Provider::new("llmsim", llm), LLMSIM_MODEL_ID)
        .single_session(|session| session)
        .build()
        .await
        .unwrap();
    let session_id = runtime.default_session_id().unwrap();
    let store = Arc::new(RoutedStore::new(shared.clone(), Routing::unique()));
    let workflow_id = start_turn(&runtime, session_id, &shared, &store).await;

    // Worker A runs the input step (first reason, the tool call) and the act.
    let worker_a = register(&store, "a").await;
    let driver_a = driver(&store, &runtime, &worker_a);
    for expected in ["process_input", "act"] {
        let task = claim_one(&store, &worker_a).await;
        assert_eq!(task.activity_type, expected);
        driver_a.execute_task(&task).await.unwrap();
    }

    // A claims the second reason and blocks in the model call; then it dies.
    let held = claim_one(&store, &worker_a).await;
    assert_eq!(held.activity_type, "reason");
    assert_eq!(held.attempt, 1);
    let running = {
        let driver_a = driver_a.clone();
        let held = held.clone();
        tokio::spawn(async move { driver_a.execute_task(&held).await })
    };
    tokio::time::timeout(Duration::from_secs(10), entered.notified())
        .await
        .expect("worker A reaches the model call");
    running.abort();
    assert!(running.await.unwrap_err().is_cancelled());
    drop(driver_a);

    let info = shared.get_task(held.id).await.unwrap();
    assert_eq!(info.status, TaskStatus::Claimed, "{info:?}");
    assert_eq!(info.claimed_by.as_deref(), Some(worker_a.as_str()));
    // The lease has not run out yet: the reaper leaves the step with A.
    let early = reap_stale_tasks(&*shared, STALE_AFTER, &NoopReapHandler)
        .await
        .unwrap();
    assert!(!early.reclaimed_ids.contains(&held.id), "{early:?}");
    let worker_b = register(&store, "b").await;
    assert!(
        store
            .claim_task(&worker_b, &activity_types(), 1)
            .await
            .unwrap()
            .is_empty(),
        "a live lease is not claimable"
    );

    // Once A's heartbeat is stale the reaper returns the step to the queue.
    tokio::time::sleep(STALE_AFTER + Duration::from_millis(200)).await;
    let reaped = reap_stale_tasks(&*shared, STALE_AFTER, &NoopReapHandler)
        .await
        .unwrap();
    assert!(reaped.reclaimed_ids.contains(&held.id), "{reaped:?}");
    assert!(
        reaped.dead_tasks.iter().all(|dead| dead.task_id != held.id)
            && reaped.sealed_tasks.iter().all(|s| s.task_id != held.id),
        "{reaped:?}"
    );

    // Worker B takes over the same task row as its second attempt and the
    // turn runs to the end.
    let driver_b = driver(&store, &runtime, &worker_b);
    let retry = claim_one(&store, &worker_b).await;
    assert_eq!(retry.id, held.id);
    assert_eq!(retry.activity_type, "reason");
    assert_eq!(retry.attempt, 2);
    driver_b.execute_task(&retry).await.unwrap();
    assert!(
        store
            .claim_task(&worker_b, &activity_types(), 1)
            .await
            .unwrap()
            .is_empty(),
        "nothing is left to run"
    );
    assert_eq!(
        EventLog::get_workflow_status(&*shared, workflow_id)
            .await
            .unwrap(),
        WorkflowStatus::Completed
    );
    let info = shared.get_task(held.id).await.unwrap();
    assert_eq!(info.status, TaskStatus::Completed, "{info:?}");
    assert_eq!(info.claimed_by.as_deref(), Some(worker_b.as_str()));

    // Exactly once for everything that settles: one assistant message per
    // reason, one completion per step, one turn.
    let messages = runtime.messages(session_id).await.unwrap();
    let texts: Vec<String> = messages
        .iter()
        .map(|message| message.content_to_llm_string())
        .collect();
    for text in ["checking", "survived the crash"] {
        let found = texts.iter().filter(|t| t.contains(text)).count();
        assert_eq!(found, 1, "{text}: {texts:#?}");
    }
    let events = runtime.events().await.unwrap();
    let count = |kind: &str| events.iter().filter(|e| e.event_type == kind).count();
    let kinds: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
    for (kind, expected) in [
        ("turn.started", 1),
        ("turn.completed", 1),
        ("turn.failed", 0),
        ("reason.completed", 2),
        ("output.message.completed", 2),
        ("act.started", 1),
        ("tool.started", 1),
        ("tool.completed", 1),
        ("act.completed", 1),
    ] {
        assert_eq!(count(kind), expected, "{kind}: {kinds:?}");
    }
    let completed_ids = message_ids(&events, "output.message.completed");
    let unique: std::collections::HashSet<&String> = completed_ids.iter().collect();
    assert_eq!(
        unique.len(),
        completed_ids.len(),
        "each message completes once"
    );

    // At least once for what a step announces before its model call: the
    // dead attempt's `reason.started` and `output.message.started` were
    // stored before it blocked, and the retry announces its own. The dead
    // attempt's message never completes and nothing replaces it, so a
    // consumer sees one orphaned started message per lost attempt. Pinned
    // here so a change that dedupes or closes it updates this test on
    // purpose. The worker stores these same events (write-behind, still
    // before the provider call), so the platform behaves the same.
    assert_eq!(count("reason.started"), 3, "{kinds:?}");
    let started_ids = message_ids(&events, "output.message.started");
    assert_eq!(started_ids.len(), 3, "{kinds:?}");
    let orphans: Vec<&String> = started_ids
        .iter()
        .filter(|id| !completed_ids.contains(id))
        .collect();
    assert_eq!(orphans.len(), 1, "{started_ids:?} vs {completed_ids:?}");
}

/// The `message_id` of every `kind` event (`output.message.*`), in order.
fn message_ids(events: &[crate::core::Event], kind: &str) -> Vec<String> {
    events
        .iter()
        .filter(|event| event.event_type == kind)
        .map(|event| {
            let data = serde_json::to_value(&event.data).unwrap();
            let id = data
                .get("message_id")
                .or_else(|| data.pointer("/message/id"))
                .and_then(serde_json::Value::as_str);
            id.expect("an output message event names its message")
                .to_owned()
        })
        .collect()
}

/// Store the turn's input and enqueue its first step, as the server does.
async fn start_turn<S: WorkflowEventStore>(
    runtime: &InProcessRuntime,
    session_id: SessionId,
    shared: &Arc<S>,
    store: &RoutedStore<S>,
) -> Uuid {
    let snapshot = runtime
        .load_resolved_turn(0, session_id)
        .await
        .unwrap()
        .snapshot;
    let input = AcceptedTurnInput::new(InputMessage::user("hello"));
    let input_message_id = input.message_id();
    runtime
        .append_accepted_inputs(session_id, TurnId::new(), vec![input])
        .await
        .unwrap();
    let workflow_id = Uuid::now_v7();
    shared
        .create_workflow(workflow_id, "turn", serde_json::json!({}), None)
        .await
        .unwrap();
    EventLog::update_workflow_status(&**shared, workflow_id, WorkflowStatus::Running, None, None)
        .await
        .unwrap();
    let turn_input = DurableTurnInput {
        org_id: in_process_internal_org_id(&snapshot.organization_id),
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
        .enqueue_task_and_record(
            workflow_id,
            format!("input_{}", Uuid::now_v7()),
            "process_input".into(),
            serde_json::to_value(&turn_input).unwrap(),
        )
        .await
        .unwrap();
    workflow_id
}

async fn register<S: WorkflowEventStore>(store: &RoutedStore<S>, name: &str) -> String {
    let worker_id = format!("crash-{name}-{}", Uuid::now_v7().simple());
    store
        .register_worker(WorkerInfo::new(worker_id.clone(), TURN_ACTIVITIES))
        .await
        .unwrap();
    worker_id
}

fn driver<S: WorkflowEventStore + 'static>(
    store: &Arc<RoutedStore<S>>,
    runtime: &InProcessRuntime,
    worker_id: &str,
) -> TurnTaskDriver<RoutedStore<S>, RuntimeHosts> {
    TurnTaskDriver::new(
        store.clone(),
        RuntimeHosts(runtime.clone()),
        worker_id,
        HEARTBEAT,
    )
}

async fn claim_one<S: WorkflowEventStore>(
    store: &RoutedStore<S>,
    worker_id: &str,
) -> crate::durable::ClaimedTask {
    let mut tasks = store
        .claim_task(worker_id, &activity_types(), 1)
        .await
        .unwrap();
    assert_eq!(tasks.len(), 1, "one step is ready: {tasks:?}");
    tasks.remove(0)
}

fn activity_types() -> [String; 3] {
    TURN_ACTIVITIES.map(String::from)
}

/// The test database, or `None` (the test skips) without `DATABASE_URL`.
async fn postgres() -> Option<PostgresWorkflowEventStore> {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        let required = std::env::var("EVERRUNS_REQUIRE_POSTGRES_TESTS").is_ok_and(|v| {
            let v = v.trim();
            !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false")
        });
        assert!(
            !required,
            "EVERRUNS_REQUIRE_POSTGRES_TESTS is set but DATABASE_URL is not"
        );
        eprintln!("DATABASE_URL is unset; skipping the PostgreSQL worker crash test");
        return None;
    };
    Some(
        PostgresWorkflowEventStore::connect(&url)
            .await
            .expect("DATABASE_URL connects and takes the durable schema"),
    )
}

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

/// llmsim, except that model call [`BLOCKED_CALL`] never answers: it tells
/// the test it started and then waits until its worker is killed. The
/// blocked call does not reach llmsim, so llmsim's own script sees only the
/// calls that answer.
struct GatedDriver {
    calls: AtomicUsize,
    entered: Arc<Notify>,
    inner: LlmSimDriver,
}

#[async_trait]
impl ChatDriver for GatedDriver {
    async fn chat_completion_stream(
        &self,
        endpoint: &ProviderEndpoint,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> LlmResult<LlmResponseStream> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == BLOCKED_CALL {
            self.entered.notify_one();
            std::future::pending::<()>().await;
        }
        self.inner
            .chat_completion_stream(endpoint, messages, config)
            .await
    }
}
