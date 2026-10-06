//! Recovery matrix: a process crash at every durable turn step boundary.
//!
//! A turn is a chain of queued tasks: `process_input` (the input step plus the
//! first reason), then `act` and `reason` steps, then the workflow completes.
//! After a step's activity succeeds the driver makes two separate store
//! writes: it completes the task (draining `USER_MESSAGE` wakes at a drain
//! boundary), then it enqueues the next step or completes the workflow (see
//! `turn_driver::execute_task` and `schedule_next_activity`). This matrix
//! crashes the driving process at each point around those writes, at each
//! boundary, then runs only the recovery that exists today (the stale-task
//! reaper plus a healthy worker claiming from the queue) and checks that the
//! turn still finishes and that a steering wake is still answered.
//!
//! A crash is simulated by [`CrashingStore`]: from the crash point on, every
//! store call the dying process makes fails and changes nothing, exactly as if
//! the process had exited. The "reply lost" point commits a write and then
//! reports a failure, as a transport error after the server committed does.
//!
//! Boundaries are identified by the task whose completion crosses them. The
//! scripted model calls a tool twice, then answers, so every boundary has a
//! task of its own:
//!
//! | boundary          | completing task  | hands off to |
//! |-------------------|------------------|--------------|
//! | input → reason    | `process_input`  | `act`        |
//! | act → reason      | first `act`      | `reason`     |
//! | reason → act      | first `reason`   | `act`        |
//! | reason → complete | second `reason`  | workflow end |

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use everruns_contracts::typed_id::{SessionId, TurnId};
use everruns_durable::{
    ActivityOptions, ClaimedTask, EventLog, HeartbeatResponse, InMemoryWorkflowEventStore,
    NoopReapHandler, Pagination, SignalStore, StoreError, TaskDefinition, TaskFailureOutcome,
    TaskFilter, TaskQueue, TaskStatus, WorkerInfo, WorkerRegistry, WorkflowError,
    WorkflowEventStore, WorkflowSignal, WorkflowStatus, reap_stale_tasks,
};
use everruns_llmsim::{LlmSimConfig, LlmSimRuntimeExt, OnExhausted, SimToolCall, SimTurn};
use uuid::Uuid;

use crate::core::{InputMessage, RuntimeMessageRole};
use crate::durable_runner::DurableTurnInput;
use crate::engine::ReasonInput;
use crate::host::{
    AcceptedTurnInput, InProcessRuntime, RuntimeHostAdapter, in_process_internal_org_id,
};
use crate::task_heartbeat::CancelSignals;
use crate::task_store::TaskStore;
use crate::turn_driver::{TurnTaskDriver, TurnTaskHost};

const TURN_ACTIVITIES: [&str; 3] = ["process_input", "reason", "act"];
const FINAL_ANSWER: &str = "durable done";
const STEER_TEXT: &str = "steer: also mention the weather";

// ---------------------------------------------------------------------------
// The matrix axes
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Boundary {
    InputToReason,
    ActToReason,
    ReasonToAct,
    ReasonToComplete,
}

impl Boundary {
    /// The task whose completion crosses this boundary: its activity type and
    /// which claim of that type it is (0-based). Counting is deterministic
    /// because nothing is retried before the crash.
    fn task(self) -> (&'static str, usize) {
        match self {
            Self::InputToReason => ("process_input", 0),
            Self::ActToReason => ("act", 0),
            Self::ReasonToAct => ("reason", 0),
            Self::ReasonToComplete => ("reason", 1),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Crash {
    /// The task is claimed, then the process dies before the activity runs:
    /// no output was produced or persisted.
    BeforeOutput,
    /// The activity ran (its side effects landed), then the process died
    /// before the task completion was written.
    BeforeComplete,
    /// The completion (and its wake drain) committed, but the reply was lost:
    /// the driver sees an error, as after a transport failure.
    CompleteReplyLost,
    /// The completion (and its wake drain) committed, then the process died
    /// before the next step was enqueued or the workflow completed.
    BeforeEnqueue,
    /// The next step was enqueued (or the workflow completed), then the
    /// process died.
    AfterEnqueue,
}

/// Whether the driver hands each next step to the queue or runs it claimed
/// on enqueue (`TurnTaskDriver::chain_steps`, the platform worker's default).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Queued,
    Chained,
}

// ---------------------------------------------------------------------------
// The crashing store
// ---------------------------------------------------------------------------

/// A `TaskStore` for one process that dies at a chosen point.
///
/// It folds the wake drain into the completion the way the worker's gRPC
/// store does (the server completes, then drains, in one request), so the
/// drain is committed exactly when the completion is.
struct CrashingStore<S> {
    inner: Arc<S>,
    target: (&'static str, usize),
    crash: Crash,
    steer: Option<Steer>,
    state: Mutex<CrashState>,
}

#[derive(Default)]
struct CrashState {
    claims_seen: HashMap<String, usize>,
    target_id: Option<Uuid>,
    /// The target completed; its hand-off write is next.
    hand_off_next: bool,
    fired: bool,
    dead: bool,
}

fn crashed() -> StoreError {
    StoreError::Database("simulated process crash".into())
}

impl<S: WorkflowEventStore> CrashingStore<S> {
    fn new(inner: Arc<S>, boundary: Boundary, crash: Crash, steer: Option<Steer>) -> Self {
        Self {
            inner,
            target: boundary.task(),
            crash,
            steer,
            state: Mutex::new(CrashState::default()),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, CrashState> {
        self.state.lock().unwrap()
    }

    fn alive(&self) -> Result<(), StoreError> {
        if self.state().dead {
            Err(crashed())
        } else {
            Ok(())
        }
    }

    fn die(&self) -> StoreError {
        let mut state = self.state();
        state.fired = true;
        state.dead = true;
        crashed()
    }

    fn fired(&self) -> bool {
        self.state().fired
    }

    fn dead(&self) -> bool {
        self.state().dead
    }

    /// Note a task handed to this process. Returns true when the process must
    /// die now because the target was claimed and the crash is before output.
    fn observe_claim(&self, task: &ClaimedTask) -> bool {
        let mut state = self.state();
        let seen = state
            .claims_seen
            .entry(task.activity_type.clone())
            .or_default();
        let nth = *seen;
        *seen += 1;
        if (task.activity_type.as_str(), nth) != self.target || state.target_id.is_some() {
            return false;
        }
        state.target_id = Some(task.id);
        self.crash == Crash::BeforeOutput
    }

    fn is_target(&self, task: &ClaimedTask) -> bool {
        self.state().target_id == Some(task.id)
    }

    /// Before a hand-off write (enqueue the next step, or end the workflow).
    /// Returns whether the write must be followed by death (after-enqueue).
    fn before_hand_off(&self) -> Result<bool, StoreError> {
        self.alive()?;
        let mut state = self.state();
        if !state.hand_off_next {
            return Ok(false);
        }
        state.hand_off_next = false;
        match self.crash {
            Crash::BeforeEnqueue => {
                drop(state);
                Err(self.die())
            }
            Crash::AfterEnqueue => Ok(true),
            _ => Ok(false),
        }
    }

    fn after_hand_off<T>(&self, result: Result<T, StoreError>, die: bool) -> Result<T, StoreError> {
        if die {
            // The write committed (or failed on its own); the process is gone
            // either way and never acts on the result.
            let _ = result;
            return Err(self.die());
        }
        result
    }
}

#[async_trait::async_trait]
impl<S: WorkflowEventStore> TaskStore for CrashingStore<S> {
    async fn register_worker(&self, worker: WorkerInfo) -> Result<(), StoreError> {
        self.alive()?;
        TaskStore::register_worker(&*self.inner, worker).await
    }

    async fn worker_heartbeat(
        &self,
        worker_id: &str,
        current_load: usize,
        accepting_tasks: bool,
    ) -> Result<(), StoreError> {
        self.alive()?;
        TaskStore::worker_heartbeat(&*self.inner, worker_id, current_load, accepting_tasks).await
    }

    async fn deregister_worker(&self, worker_id: &str) -> Result<usize, StoreError> {
        self.alive()?;
        TaskStore::deregister_worker(&*self.inner, worker_id).await
    }

    async fn claim_task(
        &self,
        worker_id: &str,
        activity_types: &[String],
        max_tasks: usize,
    ) -> Result<Vec<ClaimedTask>, StoreError> {
        self.alive()?;
        let tasks =
            TaskStore::claim_task(&*self.inner, worker_id, activity_types, max_tasks).await?;
        for task in &tasks {
            if self.observe_claim(task) {
                return Err(self.die());
            }
        }
        Ok(tasks)
    }

    async fn heartbeat_task(
        &self,
        task_id: Uuid,
        worker_id: &str,
        details: Option<serde_json::Value>,
    ) -> Result<HeartbeatResponse, StoreError> {
        self.alive()?;
        TaskStore::heartbeat_task(&*self.inner, task_id, worker_id, details).await
    }

    async fn get_workflow_status(&self, workflow_id: Uuid) -> Result<WorkflowStatus, StoreError> {
        self.alive()?;
        TaskStore::get_workflow_status(&*self.inner, workflow_id).await
    }

    async fn record_activity_started(&self, task: &ClaimedTask, worker_id: &str) {
        if !self.dead() {
            TaskStore::record_activity_started(&*self.inner, task, worker_id).await;
        }
    }

    async fn complete_task_and_record(
        &self,
        task: &ClaimedTask,
        worker_id: &str,
        output: serde_json::Value,
    ) -> Result<(), StoreError> {
        self.complete_task_and_drain(task, worker_id, output, None)
            .await
            .map(|_| ())
    }

    async fn complete_task_and_drain(
        &self,
        task: &ClaimedTask,
        worker_id: &str,
        output: serde_json::Value,
        drain: Option<&str>,
    ) -> Result<Option<usize>, StoreError> {
        self.alive()?;
        let target = self.is_target(task);
        if target {
            if let Some(steer) = &self.steer {
                steer.send(&*self.inner).await;
            }
            if self.crash == Crash::BeforeComplete {
                return Err(self.die());
            }
        }
        // Complete, then drain, as the server's CompleteDurableTask does.
        TaskStore::complete_task_and_record(&*self.inner, task, worker_id, output).await?;
        let drained = match drain {
            Some(signal_type) => Some(
                SignalStore::consume_pending_signals_by_type(
                    &*self.inner,
                    task.workflow_id.expect("turn tasks belong to a workflow"),
                    signal_type,
                )
                .await?
                .len(),
            ),
            None => None,
        };
        if target {
            match self.crash {
                Crash::CompleteReplyLost => return Err(self.die()),
                Crash::BeforeEnqueue | Crash::AfterEnqueue => self.state().hand_off_next = true,
                Crash::BeforeOutput | Crash::BeforeComplete => {}
            }
        }
        Ok(drained)
    }

    async fn fail_task_and_record(
        &self,
        task: &ClaimedTask,
        error: &str,
        retryable: bool,
    ) -> Result<TaskFailureOutcome, StoreError> {
        self.alive()?;
        TaskStore::fail_task_and_record(&*self.inner, task, error, retryable).await
    }

    async fn enqueue_task_and_record(
        &self,
        workflow_id: Uuid,
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
    ) -> Result<Uuid, StoreError> {
        let die = self.before_hand_off()?;
        let result = TaskStore::enqueue_task_and_record(
            &*self.inner,
            workflow_id,
            activity_id,
            activity_type,
            input,
        )
        .await;
        self.after_hand_off(result, die)
    }

    async fn enqueue_claimed_task_and_record(
        &self,
        workflow_id: Uuid,
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
        worker_id: &str,
    ) -> Result<Option<ClaimedTask>, StoreError> {
        let die = self.before_hand_off()?;
        let result = TaskStore::enqueue_claimed_task_and_record(
            &*self.inner,
            workflow_id,
            activity_id,
            activity_type,
            input,
            worker_id,
        )
        .await;
        let claimed = self.after_hand_off(result, die)?;
        if let Some(task) = &claimed
            && self.observe_claim(task)
        {
            return Err(self.die());
        }
        Ok(claimed)
    }

    async fn update_workflow_status(
        &self,
        workflow_id: Uuid,
        status: WorkflowStatus,
        output: Option<serde_json::Value>,
        error: Option<WorkflowError>,
    ) -> Result<(), StoreError> {
        let die = self.before_hand_off()?;
        let result =
            TaskStore::update_workflow_status(&*self.inner, workflow_id, status, output, error)
                .await;
        self.after_hand_off(result, die)
    }

    async fn complete_workflow(
        &self,
        workflow_id: Uuid,
        event_output: serde_json::Value,
        stored_output: Option<serde_json::Value>,
        error: Option<WorkflowError>,
    ) -> Result<(), StoreError> {
        let die = self.before_hand_off()?;
        let result = TaskStore::complete_workflow(
            &*self.inner,
            workflow_id,
            event_output,
            stored_output,
            error,
        )
        .await;
        self.after_hand_off(result, die)
    }

    async fn consume_pending_signals(
        &self,
        workflow_id: Uuid,
    ) -> Result<Vec<WorkflowSignal>, StoreError> {
        self.alive()?;
        TaskStore::consume_pending_signals(&*self.inner, workflow_id).await
    }

    async fn consume_pending_signals_by_type(
        &self,
        workflow_id: Uuid,
        signal_type: &str,
    ) -> Result<Vec<WorkflowSignal>, StoreError> {
        self.alive()?;
        TaskStore::consume_pending_signals_by_type(&*self.inner, workflow_id, signal_type).await
    }
}

// ---------------------------------------------------------------------------
// Steering, as the platform delivers it
// ---------------------------------------------------------------------------

/// A steering message that joins the running turn: persisted as a user
/// message, then announced with a durable `USER_MESSAGE` wake signal, the
/// way the server steers an active run.
#[derive(Clone)]
struct Steer {
    runtime: InProcessRuntime,
    session_id: SessionId,
}

impl Steer {
    async fn send<S: SignalStore + ?Sized>(&self, store: &S) {
        self.runtime
            .append_accepted_inputs(
                self.session_id,
                TurnId::new(),
                vec![AcceptedTurnInput::new(InputMessage::user(STEER_TEXT))],
            )
            .await
            .unwrap();
        store
            .send_signal(
                self.session_id.uuid(),
                WorkflowSignal::new(crate::durable_turn::USER_MESSAGE, serde_json::json!({})),
            )
            .await
            .unwrap();
    }
}

// ---------------------------------------------------------------------------
// The harness
// ---------------------------------------------------------------------------

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

/// What a cell left behind once recovery had run.
#[derive(Debug)]
pub(crate) struct Outcome {
    pub status: WorkflowStatus,
    /// Pending or claimed tasks of the turn's workflow.
    pub live_tasks: usize,
    /// The session ends with an agent answer.
    pub answered: bool,
    /// With steering: an agent message follows the steering message.
    pub steering_answered: Option<bool>,
    /// `USER_MESSAGE` wakes still pending on the workflow.
    pub pending_wakes: usize,
}

impl Outcome {
    /// The turn finished and every steering message got an answer.
    pub fn recovered(&self) -> bool {
        self.status == WorkflowStatus::Completed
            && self.live_tasks == 0
            && self.answered
            && self.steering_answered != Some(false)
    }
}

pub(crate) struct Turn<S> {
    pub store: Arc<S>,
    pub runtime: InProcessRuntime,
    pub session_id: SessionId,
    pub org_id: i64,
    pub harness_id: everruns_contracts::typed_id::HarnessId,
    pub agent_id: Option<everruns_contracts::typed_id::AgentId>,
}

impl<S> Turn<S> {
    pub fn workflow_id(&self) -> Uuid {
        self.session_id.uuid()
    }
}

/// Two tool calls, then the final answer, repeated if a step is retried.
fn script() -> LlmSimConfig {
    let tool = |id: &str| {
        SimTurn::ToolCalls(vec![SimToolCall {
            name: "no_such_tool".into(),
            arguments: serde_json::json!({}),
            id: Some(id.into()),
        }])
    };
    LlmSimConfig::scripted(vec![
        tool("call-1"),
        tool("call-2"),
        SimTurn::Assistant(FINAL_ANSWER.into()),
    ])
    .with_on_exhausted(OnExhausted::RepeatLast)
}

/// Start a turn on `store` the way the runner does: the input message is
/// persisted, the session's workflow is Running and `process_input` queued.
pub(crate) async fn start_turn<S: WorkflowEventStore>(store: Arc<S>) -> Turn<S> {
    let runtime = InProcessRuntime::builder()
        .llm_sim_as_default(script())
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

    let input = AcceptedTurnInput::new(InputMessage::user("hello"));
    let input_message_id = input.message_id();
    runtime
        .append_accepted_inputs(session_id, TurnId::new(), vec![input])
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
    let workflow_id = session_id.uuid();
    store
        .create_workflow(workflow_id, "turn", serde_json::json!({}), None)
        .await
        .unwrap();
    EventLog::update_workflow_status(&*store, workflow_id, WorkflowStatus::Running, None, None)
        .await
        .unwrap();
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
    for worker in ["crashing-worker", "healthy-worker"] {
        WorkerRegistry::register_worker(&*store, WorkerInfo::new(worker, TURN_ACTIVITIES))
            .await
            .unwrap();
    }
    Turn {
        store,
        runtime,
        session_id,
        org_id,
        harness_id: snapshot.harness_id,
        agent_id: snapshot.agent_id,
    }
}

fn activity_types() -> Vec<String> {
    TURN_ACTIVITIES.map(String::from).to_vec()
}

/// Drive the turn on a process that crashes at `crash` on `boundary`.
pub(crate) async fn crash_turn<S: WorkflowEventStore>(
    turn: &Turn<S>,
    boundary: Boundary,
    crash: Crash,
    mode: Mode,
    steer: bool,
) {
    let steer = steer.then(|| Steer {
        runtime: turn.runtime.clone(),
        session_id: turn.session_id,
    });
    let store = Arc::new(CrashingStore::new(
        turn.store.clone(),
        boundary,
        crash,
        steer,
    ));
    let driver = TurnTaskDriver::new(
        store.clone(),
        RuntimeHosts(turn.runtime.clone()),
        "crashing-worker",
        Duration::from_secs(30),
    )
    .chain_steps(mode == Mode::Chained);
    for _ in 0..20 {
        if store.dead() {
            break;
        }
        let Ok(tasks) =
            TaskStore::claim_task(&*store, "crashing-worker", &activity_types(), 1).await
        else {
            break;
        };
        let Some(task) = tasks.into_iter().next() else {
            break;
        };
        // The step's own error is the crash surfacing; recovery is below.
        let _ = driver.execute_task(&task).await;
    }
    assert!(
        store.fired(),
        "{boundary:?}/{crash:?}/{mode:?}: the crash point was never reached"
    );
}

/// Everything that recovers a turn today: the stale-task reaper, and a
/// healthy worker claiming whatever the queue holds.
pub(crate) async fn recover<S: WorkflowEventStore>(turn: &Turn<S>, mode: Mode) {
    let driver = TurnTaskDriver::new(
        turn.store.clone(),
        RuntimeHosts(turn.runtime.clone()),
        "healthy-worker",
        Duration::from_secs(30),
    )
    .chain_steps(mode == Mode::Chained);
    for _ in 0..30 {
        // The crashed process's claims stop heartbeating; any age is stale.
        tokio::time::sleep(Duration::from_millis(2)).await;
        let reaped = reap_stale_tasks(&*turn.store, Duration::ZERO, &NoopReapHandler)
            .await
            .unwrap();
        let tasks = TaskQueue::claim_task(&*turn.store, "healthy-worker", &activity_types(), 1)
            .await
            .unwrap();
        let Some(task) = tasks.into_iter().next() else {
            if reaped.reclaimed_ids.is_empty() {
                break;
            }
            continue;
        };
        let _ = driver.execute_task(&task).await;
    }
}

pub(crate) async fn outcome<S: WorkflowEventStore>(turn: &Turn<S>, steer: bool) -> Outcome {
    let workflow_id = turn.workflow_id();
    let status = EventLog::get_workflow_status(&*turn.store, workflow_id)
        .await
        .unwrap();
    let live_tasks = TaskQueue::list_tasks(
        &*turn.store,
        TaskFilter {
            workflow_id: Some(workflow_id),
            ..TaskFilter::default()
        },
        Pagination::default(),
    )
    .await
    .unwrap()
    .into_iter()
    .filter(|task| matches!(task.status, TaskStatus::Pending | TaskStatus::Claimed))
    .count();
    let messages = turn.runtime.messages(turn.session_id).await.unwrap();
    let answered = messages
        .last()
        .is_some_and(|m| m.role == RuntimeMessageRole::Agent);
    let steering_answered = steer.then(|| {
        let steered = messages
            .iter()
            .position(|m| {
                m.role == RuntimeMessageRole::User && m.content_to_llm_string().contains(STEER_TEXT)
            })
            .expect("the steering message was persisted");
        messages[steered..]
            .iter()
            .any(|m| m.role == RuntimeMessageRole::Agent)
    });
    let pending_wakes = SignalStore::consume_pending_signals_by_type(
        &*turn.store,
        workflow_id,
        crate::durable_turn::USER_MESSAGE,
    )
    .await
    .unwrap()
    .len();
    Outcome {
        status,
        live_tasks,
        answered,
        steering_answered,
        pending_wakes,
    }
}

/// Run one cell on the in-memory store.
pub(crate) async fn run_cell(boundary: Boundary, crash: Crash, mode: Mode, steer: bool) -> Outcome {
    let turn = start_turn(Arc::new(InMemoryWorkflowEventStore::new())).await;
    crash_turn(&turn, boundary, crash, mode, steer).await;
    recover(&turn, mode).await;
    outcome(&turn, steer).await
}

async fn assert_recovers(boundary: Boundary, crash: Crash, steer: bool) {
    for mode in [Mode::Queued, Mode::Chained] {
        let outcome = run_cell(boundary, crash, mode, steer).await;
        assert!(
            outcome.recovered(),
            "{boundary:?} / {crash:?} / {mode:?} / steer={steer}: turn did not recover: {outcome:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Controls: the harness itself
// ---------------------------------------------------------------------------

#[tokio::test]
async fn control_a_turn_without_a_crash_completes() {
    for mode in [Mode::Queued, Mode::Chained] {
        let turn = start_turn(Arc::new(InMemoryWorkflowEventStore::new())).await;
        recover(&turn, mode).await;
        let outcome = outcome(&turn, false).await;
        assert!(outcome.recovered(), "{mode:?}: {outcome:?}");
    }
}

#[tokio::test]
async fn every_crash_point_is_reached_at_every_boundary() {
    // `crash_turn` asserts the crash fired. With a steering wake pending at a
    // drain boundary, the wake is consumed exactly when the completion
    // commits, as the server's complete-and-drain does.
    use Boundary::*;
    use Crash::*;
    for boundary in [InputToReason, ActToReason, ReasonToAct, ReasonToComplete] {
        for crash in [
            BeforeOutput,
            BeforeComplete,
            CompleteReplyLost,
            BeforeEnqueue,
            AfterEnqueue,
        ] {
            for mode in [Mode::Queued, Mode::Chained] {
                let steer =
                    matches!(boundary, ActToReason | ReasonToComplete) && crash != BeforeOutput;
                let turn = start_turn(Arc::new(InMemoryWorkflowEventStore::new())).await;
                crash_turn(&turn, boundary, crash, mode, steer).await;
                if steer {
                    let drained = crash != BeforeComplete;
                    let outcome = outcome(&turn, true).await;
                    assert_eq!(
                        outcome.pending_wakes,
                        usize::from(!drained),
                        "{boundary:?} / {crash:?} / {mode:?}: {outcome:?}"
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The matrix: cells that recover today
// ---------------------------------------------------------------------------

macro_rules! recovers {
    ($($name:ident: $boundary:ident, $crash:ident;)*) => {
        $(
            #[tokio::test]
            async fn $name() {
                assert_recovers(Boundary::$boundary, Crash::$crash, false).await;
            }
        )*
    };
}

recovers! {
    input_to_reason_crash_before_output_recovers: InputToReason, BeforeOutput;
    input_to_reason_crash_before_complete_recovers: InputToReason, BeforeComplete;
    input_to_reason_crash_after_enqueue_recovers: InputToReason, AfterEnqueue;
    act_to_reason_crash_before_output_recovers: ActToReason, BeforeOutput;
    act_to_reason_crash_before_complete_recovers: ActToReason, BeforeComplete;
    act_to_reason_crash_after_enqueue_recovers: ActToReason, AfterEnqueue;
    reason_to_act_crash_before_output_recovers: ReasonToAct, BeforeOutput;
    reason_to_act_crash_before_complete_recovers: ReasonToAct, BeforeComplete;
    reason_to_act_crash_after_enqueue_recovers: ReasonToAct, AfterEnqueue;
    reason_to_complete_crash_before_output_recovers: ReasonToComplete, BeforeOutput;
    reason_to_complete_crash_before_complete_recovers: ReasonToComplete, BeforeComplete;
    reason_to_complete_crash_after_enqueue_recovers: ReasonToComplete, AfterEnqueue;
}

#[tokio::test]
async fn steering_survives_a_crash_before_the_draining_completion() {
    // The wake is still pending when the step reruns, so the rerun drains it.
    assert_recovers(Boundary::ActToReason, Crash::BeforeComplete, true).await;
    assert_recovers(Boundary::ReasonToComplete, Crash::BeforeComplete, true).await;
}

#[tokio::test]
async fn steering_survives_a_crash_after_the_next_step_is_enqueued() {
    assert_recovers(Boundary::ActToReason, Crash::AfterEnqueue, true).await;
    // The drained wake turns the final reason's hand-off into one more
    // reason, which is enqueued before the crash: this cell is also the
    // harness's check that a wake at the final reason gets its answer.
    assert_recovers(Boundary::ReasonToComplete, Crash::AfterEnqueue, true).await;
}

// ---------------------------------------------------------------------------
// The same cells on PostgreSQL
// ---------------------------------------------------------------------------

/// The matrix on `PostgresWorkflowEventStore`, in a scratch schema of the
/// `DATABASE_URL` database so the zero-threshold reaper and the plain claims
/// cannot touch another test's tasks. Skips without `DATABASE_URL` unless
/// `EVERRUNS_REQUIRE_POSTGRES_TESTS` is set, as `durable_backend_postgres_tests`
/// does.
pub(crate) mod postgres {
    use std::sync::Arc;

    use everruns_durable::PostgresWorkflowEventStore;
    use sqlx::AssertSqlSafe;
    use uuid::Uuid;

    use super::{Boundary, Crash, Mode, Outcome, crash_turn, outcome, recover, start_turn};

    pub(crate) struct ScratchSchema {
        name: String,
        admin: PostgresWorkflowEventStore,
        pub store: Arc<PostgresWorkflowEventStore>,
    }

    impl ScratchSchema {
        /// Decision: both stores come from `PostgresWorkflowEventStore::connect`
        /// (durable owns connection construction, see
        /// `scripts/lib/check-database-driver-isolation.sh`); the scratch one
        /// pins `search_path` through the URL.
        pub(crate) async fn create() -> Option<Self> {
            let Ok(url) = std::env::var("DATABASE_URL") else {
                assert!(
                    !require_postgres(),
                    "EVERRUNS_REQUIRE_POSTGRES_TESTS is set but DATABASE_URL is not"
                );
                eprintln!("DATABASE_URL is unset; skipping the PostgreSQL recovery matrix");
                return None;
            };
            let admin = PostgresWorkflowEventStore::connect(&url)
                .await
                .expect("DATABASE_URL connects and takes the durable schema");
            let name = format!("recovery_matrix_{}", Uuid::now_v7().simple());
            // `name` is generated from a UUID above, never external input.
            sqlx::raw_sql(AssertSqlSafe(format!("CREATE SCHEMA {name}")))
                .execute(admin.pool())
                .await
                .expect("create scratch schema");
            let separator = if url.contains('?') { '&' } else { '?' };
            let scratch_url = format!("{url}{separator}options%5Bsearch_path%5D={name}");
            let store = PostgresWorkflowEventStore::connect(&scratch_url)
                .await
                .expect("the scratch schema takes the durable schema");
            let current: String = sqlx::query_scalar("SELECT current_schema()")
                .fetch_one(store.pool())
                .await
                .expect("current_schema");
            assert_eq!(current, name, "the scratch store must stay in its schema");
            Some(Self {
                name,
                admin,
                store: Arc::new(store),
            })
        }

        pub(crate) async fn drop(self) {
            self.store.pool().close().await;
            sqlx::raw_sql(AssertSqlSafe(format!("DROP SCHEMA {} CASCADE", self.name)))
                .execute(self.admin.pool())
                .await
                .expect("drop scratch schema");
        }
    }

    fn require_postgres() -> bool {
        std::env::var("EVERRUNS_REQUIRE_POSTGRES_TESTS").is_ok_and(|v| {
            let v = v.trim();
            !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false")
        })
    }

    /// Run `cells` one after another in one scratch schema and return the
    /// ones that did not recover. `None` when the test skips.
    pub(crate) async fn failing_cells(cells: &[(Boundary, Crash, bool)]) -> Option<Vec<String>> {
        let schema = ScratchSchema::create().await?;
        let mut failing = Vec::new();
        for &(boundary, crash, steer) in cells {
            for mode in [Mode::Queued, Mode::Chained] {
                let turn = start_turn(schema.store.clone()).await;
                crash_turn(&turn, boundary, crash, mode, steer).await;
                recover(&turn, mode).await;
                let outcome: Outcome = outcome(&turn, steer).await;
                if !outcome.recovered() {
                    failing.push(format!(
                        "{boundary:?} / {crash:?} / {mode:?} / steer={steer}: {outcome:?}"
                    ));
                }
            }
        }
        schema.drop().await;
        Some(failing)
    }
}

#[tokio::test]
async fn postgres_cells_that_recover() {
    use Boundary::*;
    use Crash::*;
    let mut cells = Vec::new();
    for boundary in [InputToReason, ActToReason, ReasonToAct, ReasonToComplete] {
        for crash in [BeforeOutput, BeforeComplete, AfterEnqueue] {
            cells.push((boundary, crash, false));
        }
    }
    cells.push((ActToReason, BeforeComplete, true));
    cells.push((ReasonToComplete, BeforeComplete, true));
    cells.push((ActToReason, AfterEnqueue, true));
    cells.push((ReasonToComplete, AfterEnqueue, true));
    let Some(failing) = postgres::failing_cells(&cells).await else {
        return;
    };
    assert!(
        failing.is_empty(),
        "cells did not recover:\n{}",
        failing.join("\n")
    );
}

// ---------------------------------------------------------------------------
// The matrix: cells that reproduce a stuck turn today
// ---------------------------------------------------------------------------
//
// Once the completing write commits, nothing re-plans the step after it: the
// task is `completed`, the workflow stays `running`, no task is pending or
// claimed, and the reaper only looks at claimed tasks. Kept failing on
// purpose until the hand-off is made recoverable.

recovers! {
    input_to_reason_crash_complete_reply_lost_recovers: InputToReason, CompleteReplyLost;
    input_to_reason_crash_before_enqueue_recovers: InputToReason, BeforeEnqueue;
    act_to_reason_crash_complete_reply_lost_recovers: ActToReason, CompleteReplyLost;
    act_to_reason_crash_before_enqueue_recovers: ActToReason, BeforeEnqueue;
    reason_to_act_crash_complete_reply_lost_recovers: ReasonToAct, CompleteReplyLost;
    reason_to_act_crash_before_enqueue_recovers: ReasonToAct, BeforeEnqueue;
    reason_to_complete_crash_complete_reply_lost_recovers: ReasonToComplete, CompleteReplyLost;
    reason_to_complete_crash_before_enqueue_recovers: ReasonToComplete, BeforeEnqueue;
}

#[tokio::test]
async fn steering_wake_is_not_lost_when_the_hand_off_after_its_drain_never_happens() {
    // The final reason's completion drains the wake that arrived as the turn
    // wound down; the crash then drops the extra reason that wake asked for.
    // Recovery aside, the wake itself must survive: still pending, or answered.
    for boundary in [Boundary::ActToReason, Boundary::ReasonToComplete] {
        for mode in [Mode::Queued, Mode::Chained] {
            let turn = start_turn(Arc::new(InMemoryWorkflowEventStore::new())).await;
            crash_turn(&turn, boundary, Crash::BeforeEnqueue, mode, true).await;
            let outcome = outcome(&turn, true).await;
            assert!(
                outcome.pending_wakes == 1 || outcome.steering_answered == Some(true),
                "{boundary:?} / {mode:?}: the steering wake was consumed and nothing will act on it: {outcome:?}"
            );
        }
    }
}

#[tokio::test]
async fn a_later_message_starts_a_turn_on_a_session_stuck_between_complete_and_enqueue() {
    use crate::runner::AgentRunner;

    let turn = start_turn(Arc::new(InMemoryWorkflowEventStore::new())).await;
    crash_turn(
        &turn,
        Boundary::ReasonToComplete,
        Crash::BeforeEnqueue,
        Mode::Queued,
        false,
    )
    .await;
    recover(&turn, Mode::Queued).await;

    // The user sends another message, the way the server does: persist it,
    // then start a run, which steers the run instead when one is active.
    let later = AcceptedTurnInput::new(InputMessage::user("anyone there?"));
    let later_id = later.message_id();
    turn.runtime
        .append_accepted_inputs(turn.session_id, TurnId::new(), vec![later])
        .await
        .unwrap();
    let runner = crate::DurableRunner::new_with_shared_store(turn.store.clone());
    runner
        .start_run(
            turn.org_id,
            turn.session_id,
            turn.harness_id,
            turn.agent_id,
            later_id,
            None,
        )
        .await
        .unwrap();
    let wakes_after_send = SignalStore::consume_pending_signals_by_type(
        &*turn.store,
        turn.workflow_id(),
        crate::durable_turn::USER_MESSAGE,
    )
    .await
    .unwrap()
    .len();
    recover(&turn, Mode::Queued).await;

    let messages = turn.runtime.messages(turn.session_id).await.unwrap();
    let later_at = messages
        .iter()
        .position(|m| m.id == later_id)
        .expect("the later message was persisted");
    let answered = messages[later_at..]
        .iter()
        .any(|m| m.role == RuntimeMessageRole::Agent);
    let outcome = outcome(&turn, false).await;
    assert!(
        answered,
        "the later message was never answered (the send became {wakes_after_send} \
         steering wake(s) on a run nothing drives): {outcome:?}"
    );
}

#[tokio::test]
async fn postgres_cells_stuck_between_complete_and_enqueue() {
    use Boundary::*;
    use Crash::*;
    let mut cells = Vec::new();
    for boundary in [InputToReason, ActToReason, ReasonToAct, ReasonToComplete] {
        for crash in [CompleteReplyLost, BeforeEnqueue] {
            cells.push((boundary, crash, false));
        }
    }
    let Some(failing) = postgres::failing_cells(&cells).await else {
        return;
    };
    assert!(
        failing.is_empty(),
        "cells did not recover:\n{}",
        failing.join("\n")
    );
}
