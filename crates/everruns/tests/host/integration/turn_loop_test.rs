//! The host-generic turn loop and resume-from-log, driven the way a host
//! other than `InProcessRuntime` drives them.
//!
//! A turn parks on a client-side tool; its result is recorded as a plain
//! `tool.completed` event. Folding the log into the open turn and driving it
//! with `drive_turn` through a test-owned `TurnInbox` (not the runtime's own
//! steering) continues the same turn to completion, delivers input at reason
//! boundaries, and closes the inbox when the turn finishes.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use async_trait::async_trait;
use everruns_contracts::driver_registry::DriverRegistry;
use everruns_contracts::error::Result;
use everruns_contracts::model_spec::ModelSpec;
use everruns_contracts::provider::DriverId;
use everruns_contracts::tool_types::{ClientSideTool, ToolCall, ToolDefinition};
use everruns_contracts::typed_id::{AgentId, HarnessId, MessageId, SessionId, TurnId};
use everruns_core::engine::{TurnPlan, TurnState};
use everruns_core::events::ToolCompletedData;
use everruns_core::host::{
    AcceptedTurnInput, AgentBuilder, HarnessBuilder, HostComposition, InProcessExecution,
    InProcessRuntime, PendingInput, SessionBuilder, TurnDrive, TurnInbox, TurnLogFold, drive_turn,
    in_process_internal_org_id, resume_entry_plan, resume_plan_from_log,
};
use everruns_core::{CapabilityRegistry, ContentPart, InputMessage, RuntimeMessageRole};
use everruns_llmsim::{LlmSimConfig, LlmSimRuntimeExt, SimToolCall, SimTurn};
use everruns_test_support::TestMathCapability;
use serde_json::json;

/// A runtime whose session exposes the client-side `confirm` tool and whose
/// model calls it first, then answers with `answers` in order.
async fn client_tool_runtime(seed: u128, answers: &[&str]) -> (InProcessRuntime, SessionId) {
    let harness_id = HarnessId::from_seed(seed);
    let agent_id = AgentId::from_seed(seed);
    let session_id = SessionId::from_seed(seed);
    let mut session = SessionBuilder::new(harness_id)
        .id(session_id)
        .agent(agent_id)
        .tool(ToolDefinition::ClientSide(ClientSideTool::new(
            "confirm",
            "Ask the person to confirm.",
            json!({ "type": "object" }),
        )))
        .build();
    // The client declares it answers a pause.
    session.hints = Some([("setup_connection".to_string(), json!(true))].into());
    let mut script = vec![SimTurn::ToolCalls(vec![SimToolCall {
        name: "confirm".to_string(),
        arguments: json!({ "what": "deploy" }),
        id: Some("call_confirm".to_string()),
    }])];
    script.extend(
        answers
            .iter()
            .map(|answer| SimTurn::Assistant(answer.to_string())),
    );
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(TestMathCapability);
    let runtime = everruns::batteries::runtime_builder()
        .host_composition(HostComposition::new(capabilities, DriverRegistry::new()))
        .harness(
            HarnessBuilder::new("deploy", "You deploy things.")
                .id(harness_id)
                .build(),
        )
        .agent(
            AgentBuilder::new("deployer", "Confirm before deploying.")
                .id(agent_id)
                .max_iterations(8)
                .build(),
        )
        .session(session)
        .llm_sim_as_default(LlmSimConfig::scripted(script))
        .default_model(ModelSpec::on((DriverId::LlmSim).as_str(), "llmsim-model"))
        .build()
        .await
        .expect("runtime builds");
    (runtime, session_id)
}

/// Run the first turn until it parks on `confirm`, then record the call's
/// result as a host with its own event store would: one `tool.completed`.
async fn park_and_record_result(runtime: &InProcessRuntime, session_id: SessionId) -> TurnId {
    let parked = runtime
        .run_text_turn(session_id, "Deploy.")
        .await
        .expect("turn runs");
    let calls = runtime
        .parked_tool_calls(session_id)
        .expect("the turn parked on the client-side call");
    assert_eq!(calls.turn_id, parked.turn_id);
    runtime
        .record_parked_tool_results(
            session_id,
            vec![ToolCompletedData::success(
                "call_confirm".to_string(),
                "confirm".to_string(),
                vec![ContentPart::text("confirmed")],
                None,
            )],
        )
        .await
        .expect("result recorded");
    parked.turn_id
}

/// A turn inbox owned by the test, not by the runtime.
///
/// `ready` input is delivered at the next reason's start. `late` input
/// "arrives while the first reason runs": it becomes ready at that reason's
/// end, where a reason that would finish takes it.
struct TestInbox<'a> {
    runtime: &'a InProcessRuntime,
    ready: Mutex<Vec<AcceptedTurnInput>>,
    late: Mutex<Vec<AcceptedTurnInput>>,
    closed: AtomicBool,
    takes: AtomicUsize,
    parks: AtomicUsize,
}

impl<'a> TestInbox<'a> {
    fn new(
        runtime: &'a InProcessRuntime,
        ready: Vec<AcceptedTurnInput>,
        late: Vec<AcceptedTurnInput>,
    ) -> Self {
        Self {
            runtime,
            ready: Mutex::new(ready),
            late: Mutex::new(late),
            closed: AtomicBool::new(false),
            takes: AtomicUsize::new(0),
            parks: AtomicUsize::new(0),
        }
    }

    async fn persist(
        &self,
        session_id: SessionId,
        inputs: Vec<AcceptedTurnInput>,
    ) -> Result<Vec<MessageId>> {
        let mut ids = Vec::new();
        for input in inputs {
            ids.push(
                self.runtime
                    .persist_accepted_input(session_id, input)
                    .await?,
            );
        }
        Ok(ids)
    }
}

#[async_trait]
impl TurnInbox for TestInbox<'_> {
    async fn take(&self, session_id: SessionId) -> Result<Vec<MessageId>> {
        self.takes.fetch_add(1, Ordering::SeqCst);
        let inputs: Vec<_> = self.ready.lock().unwrap().drain(..).collect();
        self.persist(session_id, inputs).await
    }

    async fn after_reason(
        &self,
        session_id: SessionId,
        would_finish: bool,
    ) -> Result<PendingInput> {
        let inputs = {
            let mut ready = self.ready.lock().unwrap();
            ready.extend(self.late.lock().unwrap().drain(..));
            if !would_finish {
                return Ok(PendingInput::new(ready.len(), Vec::new()));
            }
            if ready.is_empty() {
                self.closed.store(true, Ordering::SeqCst);
            }
            ready.drain(..).collect::<Vec<_>>()
        };
        let count = inputs.len();
        Ok(PendingInput::new(
            count,
            self.persist(session_id, inputs).await?,
        ))
    }

    async fn close(&self, _session_id: SessionId) -> Result<()> {
        self.closed.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn park(
        &self,
        _session_id: SessionId,
        _turn_id: TurnId,
        _client_tool_calls: Vec<ToolCall>,
        _resume: TurnState,
    ) -> Result<()> {
        self.parks.fetch_add(1, Ordering::SeqCst);
        self.closed.store(true, Ordering::SeqCst);
        Ok(())
    }
}

/// Resume `session_id`'s open turn from its log and drive it through `inbox`.
async fn resume_from_log(
    runtime: &InProcessRuntime,
    session_id: SessionId,
    inbox: &TestInbox<'_>,
) -> everruns_core::host::TurnResult {
    let snapshot = runtime
        .resolved_execution_snapshot(session_id)
        .await
        .expect("snapshot");
    let org_id = in_process_internal_org_id(&snapshot.organization_id);
    let events = runtime.events().await.expect("events");
    let (state, act) = resume_plan_from_log(runtime, org_id, session_id, events, None)
        .await
        .expect("resume plan")
        .expect("the parked turn is open in the log");
    assert!(
        act.input.tool_calls.is_empty(),
        "the client-side call has its result; nothing runs again"
    );
    let plan = resume_entry_plan(&state, act);
    assert!(matches!(plan, TurnPlan::ScheduleAct(_)), "{plan:?}");
    let drive = TurnDrive::for_state(&state, snapshot.workspace_id);
    drive_turn(runtime, inbox, drive, InProcessExecution::new(state), plan)
        .await
        .expect("turn resumes")
}

fn count_events(events: &[everruns_core::events::Event], event_type: &str) -> usize {
    events
        .iter()
        .filter(|event| event.data.event_type() == event_type)
        .count()
}

#[tokio::test]
async fn a_parked_turn_resumes_from_its_log_once_its_result_is_recorded() {
    let (runtime, session_id) = client_tool_runtime(961, &["Deployed."]).await;
    let turn_id = park_and_record_result(&runtime, session_id).await;

    let inbox = TestInbox::new(&runtime, Vec::new(), Vec::new());
    let resumed = resume_from_log(&runtime, session_id, &inbox).await;

    assert!(resumed.success, "{resumed:?}");
    assert_eq!(resumed.response, "Deployed.");
    assert_eq!(resumed.turn_id, turn_id, "the same turn continues");
    assert!(
        inbox.closed.load(Ordering::SeqCst),
        "a finished turn closes"
    );
    assert_eq!(inbox.parks.load(Ordering::SeqCst), 0);
    let events = runtime.events().await.expect("events");
    assert_eq!(count_events(&events, "turn.completed"), 1);
    assert_eq!(count_events(&events, "turn.started"), 1);
    assert!(
        TurnLogFold::from_events(events).is_none(),
        "the log has no open turn left"
    );
}

#[tokio::test]
async fn drive_turn_delivers_inbox_input_at_reason_boundaries_and_closes_on_finish() {
    let (runtime, session_id) = client_tool_runtime(962, &["Deployed.", "Noted both."]).await;
    park_and_record_result(&runtime, session_id).await;

    let inbox = TestInbox::new(
        &runtime,
        vec![AcceptedTurnInput::new(InputMessage::user("early note"))],
        vec![AcceptedTurnInput::new(InputMessage::user("late note"))],
    );
    let resumed = resume_from_log(&runtime, session_id, &inbox).await;

    assert!(resumed.success, "{resumed:?}");
    assert_eq!(
        resumed.iterations, 2,
        "the late input keeps the turn going for one more reason"
    );
    assert_eq!(inbox.takes.load(Ordering::SeqCst), 2);
    assert!(inbox.closed.load(Ordering::SeqCst));

    // Both inputs land before the reason that saw them: the early one ahead
    // of the first resumed answer, the late one between the two answers.
    let texts: Vec<(RuntimeMessageRole, String)> = runtime
        .messages(session_id)
        .await
        .expect("messages")
        .into_iter()
        .filter(|message| {
            matches!(
                message.role,
                RuntimeMessageRole::User | RuntimeMessageRole::Agent
            )
        })
        .map(|message| (message.role.clone(), message.content_to_llm_string()))
        .filter(|(_, text)| !text.is_empty())
        .collect();
    let position = |needle: &str| {
        texts
            .iter()
            .position(|(_, text)| text == needle)
            .unwrap_or_else(|| panic!("{needle:?} in {texts:?}"))
    };
    assert!(position("early note") < position("Deployed."), "{texts:?}");
    assert!(position("Deployed.") < position("late note"), "{texts:?}");
    assert!(position("late note") < position("Noted both."), "{texts:?}");

    let events = runtime.events().await.expect("events");
    assert_eq!(count_events(&events, "turn.completed"), 1);
}
