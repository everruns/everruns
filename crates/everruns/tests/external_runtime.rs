//! An external host drives an Agent's turns over its own canonical log, and a
//! turn cut off mid-act finishes on a runtime rebuilt from that log alone.
//! A turn whose answer committed before its holder died ends without asking
//! the model again.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use everruns::hosting::runtime::{AcceptedTurnInput, HostBackends, TurnId, TurnSteering};
use everruns::prelude::*;
use everruns::{LlmSimConfig, SimToolCall, SimTurn};
use everruns_core::events::{Event, EventRequest};
use everruns_core::host::{
    EventDurability, EventLog, EventLogError, EventPage, EventReadLimit, EventReadRequest,
    EventReader, InMemoryEventLog,
};
use serde_json::{Value, json};

fn call(name: &str, id: &str) -> SimTurn {
    SimTurn::ToolCalls(vec![SimToolCall {
        name: name.into(),
        arguments: json!({}),
        id: Some(id.into()),
    }])
}

async fn event_types(log: &InMemoryEventLog, session: SessionId) -> Vec<String> {
    let page = log
        .read_page(EventReadRequest::new(session, EventReadLimit::default()))
        .await
        .expect("read log");
    page.events.into_iter().map(|e| e.event_type).collect()
}

#[tokio::test]
async fn a_turn_runs_over_a_host_supplied_log() {
    let agent = Agent::builder()
        .instructions("Answer.")
        .model(Model::simulated("Hello from the host log."))
        .build()
        .expect("agent");
    let log = Arc::new(InMemoryEventLog::new());
    let session = SessionId::new();
    let runtime = agent
        .external_runtime(
            HostBackends::in_memory().with_event_log(log.clone()),
            session,
        )
        .await
        .expect("runtime");

    let input = AcceptedTurnInput::new("hi");
    let message = runtime
        .persist_accepted_input(session, input)
        .await
        .expect("admit");
    let turn = runtime
        .run_stored_turn(session, message, TurnId::new(), TurnSteering::new())
        .await
        .expect("turn");

    assert!(turn.success, "{:?}", turn.error);
    assert_eq!(turn.response, "Hello from the host log.");
    let types = event_types(&log, session).await;
    assert!(types.iter().any(|t| t == "input.message"), "{types:?}");
    assert!(types.iter().any(|t| t == "turn.completed"), "{types:?}");
}

#[tokio::test]
async fn a_turn_cut_off_mid_act_finishes_on_a_rebuilt_runtime() {
    // A tool that never returns stands in for a process that dies mid-act.
    let hang = FunctionTool::new(
        "deploy",
        "Deploy.",
        json!({ "type": "object", "properties": {} }),
        |_args: Value| async move {
            std::future::pending::<()>().await;
            Ok::<_, String>(json!({}))
        },
    );
    let sim = LlmSimConfig::scripted(vec![
        call("deploy", "call_1"),
        SimTurn::Assistant("The deploy was interrupted.".into()),
    ]);
    let agent = Agent::builder()
        .instructions("Deploy, then report.")
        .model(Model::simulated_with_config(sim))
        .tool(hang)
        .build()
        .expect("agent");
    let log = Arc::new(InMemoryEventLog::new());
    let session = SessionId::new();

    // First holder: admits the input, starts the turn, and is dropped while
    // the tool runs. Only the log survives.
    {
        let runtime = agent
            .external_runtime(
                HostBackends::in_memory().with_event_log(log.clone()),
                session,
            )
            .await
            .expect("runtime");
        let message = runtime
            .persist_accepted_input(session, AcceptedTurnInput::new("ship it"))
            .await
            .expect("admit");
        let cut = tokio::time::timeout(
            Duration::from_millis(500),
            runtime.run_stored_turn(session, message, TurnId::new(), TurnSteering::new()),
        )
        .await;
        assert!(cut.is_err(), "the hung tool holds the turn");
    }
    let before = event_types(&log, session).await;
    assert!(before.iter().any(|t| t == "tool.started"), "{before:?}");
    assert!(!before.iter().any(|t| t == "turn.completed"), "{before:?}");

    // Second holder: a fresh runtime over the same log continues the turn.
    let runtime = agent
        .external_runtime(
            HostBackends::in_memory().with_event_log(log.clone()),
            session,
        )
        .await
        .expect("rebuilt runtime");
    let turn = runtime
        .resume_interrupted_turn(session, TurnSteering::new())
        .await
        .expect("resume");

    assert!(turn.success, "{:?}", turn.error);
    assert_eq!(turn.response, "The deploy was interrupted.");
    let after = event_types(&log, session).await;
    assert_eq!(
        after.iter().filter(|t| *t == "turn.started").count(),
        1,
        "the turn keeps its identity: {after:?}"
    );
    assert!(after.iter().any(|t| t == "turn.completed"), "{after:?}");
    // Rebuilding the runtime starts no second session.
    assert_eq!(
        after.iter().filter(|t| *t == "session.started").count(),
        1,
        "{after:?}"
    );
    // The hung call is settled as interrupted, never run a second time.
    assert_eq!(
        after.iter().filter(|t| *t == "tool.started").count(),
        1,
        "{after:?}"
    );
}

/// Commits through `inner` until the turn tries to end, then hangs: the
/// holder dies after its answer is durable and before `turn.completed`.
struct DiesBeforeTurnEnd {
    inner: Arc<InMemoryEventLog>,
}

#[async_trait]
impl EventReader for DiesBeforeTurnEnd {
    async fn read_page(&self, request: EventReadRequest) -> Result<EventPage, EventLogError> {
        self.inner.read_page(request).await
    }
}

#[async_trait]
impl EventLog for DiesBeforeTurnEnd {
    async fn append(&self, request: EventRequest) -> Result<Event, EventLogError> {
        if request.event_type == "turn.completed" {
            std::future::pending::<()>().await;
        }
        self.inner.append(request).await
    }

    async fn append_conditional(
        &self,
        request: EventRequest,
        expected_last_sequence: Option<i32>,
    ) -> Result<Event, EventLogError> {
        if request.event_type == "turn.completed" {
            std::future::pending::<()>().await;
        }
        self.inner
            .append_conditional(request, expected_last_sequence)
            .await
    }

    fn durability(&self) -> EventDurability {
        self.inner.durability()
    }
}

#[tokio::test]
async fn an_answered_turn_cut_off_before_its_end_completes_without_the_model() {
    // One scripted answer: a second model call would fail the turn.
    let sim = LlmSimConfig::scripted(vec![SimTurn::Assistant("Done.".into())]);
    let agent = Agent::builder()
        .instructions("Answer.")
        .model(Model::simulated_with_config(sim))
        .build()
        .expect("agent");
    let log = Arc::new(InMemoryEventLog::new());
    let session = SessionId::new();
    {
        let dying = Arc::new(DiesBeforeTurnEnd { inner: log.clone() });
        let runtime = agent
            .external_runtime(HostBackends::in_memory().with_event_log(dying), session)
            .await
            .expect("runtime");
        let message = runtime
            .persist_accepted_input(session, AcceptedTurnInput::new("hi"))
            .await
            .expect("admit");
        let cut = tokio::time::timeout(
            Duration::from_millis(500),
            runtime.run_stored_turn(session, message, TurnId::new(), TurnSteering::new()),
        )
        .await;
        assert!(cut.is_err(), "the turn hangs at its end");
    }
    let before = event_types(&log, session).await;
    assert!(
        before.iter().any(|t| t == "output.message.completed"),
        "{before:?}"
    );
    assert!(!before.iter().any(|t| t == "turn.completed"), "{before:?}");

    let runtime = agent
        .external_runtime(
            HostBackends::in_memory().with_event_log(log.clone()),
            session,
        )
        .await
        .expect("rebuilt runtime");
    let turn = runtime
        .resume_interrupted_turn(session, TurnSteering::new())
        .await
        .expect("resume");

    assert!(turn.success, "{:?}", turn.error);
    assert_eq!(turn.response, "Done.");
    let after = event_types(&log, session).await;
    assert_eq!(
        after.iter().filter(|t| *t == "reason.started").count(),
        1,
        "the model is not asked again: {after:?}"
    );
    assert_eq!(
        after.iter().filter(|t| *t == "turn.completed").count(),
        1,
        "{after:?}"
    );
    assert!(after.iter().any(|t| t == "session.idled"), "{after:?}");
}

/// Refuses `tool.started`, as a log does once another holder owns the turn.
struct RefusesToolStart {
    inner: Arc<InMemoryEventLog>,
}

#[async_trait]
impl EventReader for RefusesToolStart {
    async fn read_page(&self, request: EventReadRequest) -> Result<EventPage, EventLogError> {
        self.inner.read_page(request).await
    }
}

#[async_trait]
impl EventLog for RefusesToolStart {
    async fn append(&self, request: EventRequest) -> Result<Event, EventLogError> {
        if request.event_type == "tool.started" {
            return Err(EventLogError::Backend {
                detail: "claim lost".into(),
            });
        }
        self.inner.append(request).await
    }

    async fn append_conditional(
        &self,
        request: EventRequest,
        expected_last_sequence: Option<i32>,
    ) -> Result<Event, EventLogError> {
        if request.event_type == "tool.started" {
            return Err(EventLogError::Backend {
                detail: "claim lost".into(),
            });
        }
        self.inner
            .append_conditional(request, expected_last_sequence)
            .await
    }

    fn durability(&self) -> EventDurability {
        self.inner.durability()
    }
}

#[tokio::test]
async fn a_call_that_may_not_run_twice_does_not_run_unless_its_start_is_stored() {
    // A tool not declared idempotent runs at most once: a holder that cannot
    // record the start (it lost the turn) must not run it unseen.
    let runs = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = runs.clone();
    let pay = FunctionTool::new(
        "pay",
        "Pay.",
        json!({ "type": "object", "properties": {} }),
        move |_args: Value| {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok::<_, String>(json!({ "paid": true }))
            }
        },
    );
    let sim = LlmSimConfig::scripted(vec![
        call("pay", "call_1"),
        SimTurn::Assistant("Payment not made.".into()),
    ]);
    let agent = Agent::builder()
        .instructions("Pay, then report.")
        .model(Model::simulated_with_config(sim))
        .tool(pay)
        .build()
        .expect("agent");
    let log = Arc::new(InMemoryEventLog::new());
    let session = SessionId::new();
    let runtime = agent
        .external_runtime(
            HostBackends::in_memory()
                .with_event_log(Arc::new(RefusesToolStart { inner: log.clone() })),
            session,
        )
        .await
        .expect("runtime");
    let message = runtime
        .persist_accepted_input(session, AcceptedTurnInput::new("pay"))
        .await
        .expect("admit");
    let result = runtime
        .run_stored_turn(session, message, TurnId::new(), TurnSteering::new())
        .await;
    assert!(result.is_ok(), "the turn carries on with the failure");

    assert_eq!(
        runs.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "the call ran without a stored start"
    );
}
