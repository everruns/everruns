//! Unit tests for [`InProcessBackend`], the turn backend the session actor
//! runs its turns on.
//!
//! They live in the facade because it is the crate that can build a runtime
//! over the deterministic simulated model.

use std::time::Duration;

use everruns_contracts::error::AgentLoopError;
use everruns_contracts::typed_id::{SessionId, TurnId};
use everruns_core::host::{
    AcceptedTurnInput, HostBackends, InProcessBackend, InProcessRuntime, TurnBackend, TurnInput,
    TurnRequest, TurnTicket,
};
use everruns_core::turn::TurnStopReason;

use crate::{Agent, Model};

async fn backend(model: Model) -> (InProcessBackend, SessionId) {
    let (backend, _runtime, session_id) = backend_with_runtime(model).await;
    (backend, session_id)
}

async fn backend_with_runtime(model: Model) -> (InProcessBackend, InProcessRuntime, SessionId) {
    let agent = Agent::builder()
        .instructions("You are concise.")
        .model(model)
        .build()
        .expect("valid agent");
    let session_id = SessionId::new();
    let runtime = agent
        .build_runtime_with_backends(
            HostBackends::in_memory(),
            session_id,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("runtime builds");
    (InProcessBackend::new(runtime.clone()), runtime, session_id)
}

async fn start(backend: &InProcessBackend, session_id: SessionId, text: &str) -> TurnTicket {
    let input = AcceptedTurnInput::new(everruns_core::InputMessage::user(text));
    backend
        .start_turn(TurnRequest::new(
            session_id,
            TurnId::new(),
            TurnInput::Message(Box::new(input)),
        ))
        .await
        .expect("turn starts")
}

/// A slow model keeps the turn in flight; polling its ticket briefly lets
/// the turn begin without letting it finish.
async fn begin(ticket: &mut TurnTicket) {
    assert!(
        tokio::time::timeout(Duration::from_millis(50), ticket)
            .await
            .is_err(),
        "the slow turn is still running"
    );
}

#[tokio::test]
async fn a_started_turn_runs_to_its_result_and_then_leaves_the_backend() {
    let (backend, session_id) = backend(Model::simulated("Sure.")).await;
    assert!(!backend.is_running(session_id).await);
    assert_eq!(backend.active_count().await, 0);

    let ticket = start(&backend, session_id, "hi").await;
    assert_eq!(ticket.session_id(), session_id);
    assert!(backend.is_running(session_id).await);
    assert_eq!(backend.active_count().await, 1);

    let turn_id = ticket.turn_id();
    let result = ticket.await.expect("turn completes");
    assert!(result.success);
    assert_eq!(result.response, "Sure.");
    assert_eq!(result.turn_id, turn_id, "the turn keeps the requested id");
    assert!(!backend.is_running(session_id).await);
    assert_eq!(backend.active_count().await, 0);

    // The session is free for its next turn.
    let next = start(&backend, session_id, "again").await;
    assert!(next.await.expect("second turn completes").success);
}

#[tokio::test]
async fn cancel_stops_a_running_turn_and_resolves_its_ticket_as_cancelled() {
    let (backend, session_id) = backend(Model::simulated_delayed(
        "eventually",
        Duration::from_secs(30),
    ))
    .await;
    let mut ticket = start(&backend, session_id, "hi").await;
    begin(&mut ticket).await;
    assert!(backend.is_running(session_id).await);

    assert!(backend.cancel(session_id).await.expect("cancel succeeds"));
    assert!(!backend.is_running(session_id).await);
    assert_eq!(backend.active_count().await, 0);

    let error = ticket.await.expect_err("a cancelled turn has no result");
    assert!(matches!(error, AgentLoopError::Cancelled), "{error:?}");
    assert!(
        !backend.cancel(session_id).await.expect("cancel succeeds"),
        "nothing is left to cancel"
    );
}

#[tokio::test]
async fn a_session_runs_one_turn_at_a_time_and_a_dropped_ticket_frees_it() {
    let (backend, session_id) = backend(Model::simulated_delayed(
        "eventually",
        Duration::from_secs(30),
    ))
    .await;
    let mut first = start(&backend, session_id, "first").await;
    begin(&mut first).await;

    let input = AcceptedTurnInput::new(everruns_core::InputMessage::user("second"));
    let error = backend
        .start_turn(TurnRequest::new(
            session_id,
            TurnId::new(),
            TurnInput::Message(Box::new(input)),
        ))
        .await
        .expect_err("the session already runs a turn");
    assert!(error.to_string().contains("already runs a turn"), "{error}");

    // Dropping the ticket drops the in-process turn with it.
    drop(first);
    assert!(!backend.is_running(session_id).await);
    assert_eq!(backend.active_count().await, 0);
    let _second = start(&backend, session_id, "second").await;
    assert!(backend.is_running(session_id).await);
}

#[tokio::test]
async fn turns_of_other_sessions_are_not_running() {
    let (backend, session_id) = backend(Model::simulated_delayed(
        "eventually",
        Duration::from_secs(30),
    ))
    .await;
    let mut ticket = start(&backend, session_id, "hi").await;
    begin(&mut ticket).await;

    let other = SessionId::new();
    assert!(!backend.is_running(other).await);
    assert!(!backend.cancel(other).await.expect("cancel succeeds"));
    assert!(
        backend.is_running(session_id).await,
        "cancelling another session leaves this turn running"
    );
}

#[tokio::test]
async fn resuming_without_a_parked_turn_fails_through_the_ticket() {
    let (backend, session_id) = backend(Model::simulated("Sure.")).await;
    let ticket = backend
        .start_turn(TurnRequest::new(
            session_id,
            TurnId::new(),
            TurnInput::ToolResults(Vec::new()),
        ))
        .await
        .expect("the backend accepts the turn");
    let error = ticket.await.expect_err("nothing is parked");
    assert!(error.to_string().contains("no turn waiting"), "{error}");
    assert!(!backend.is_running(session_id).await);

    // The failed resume released the session.
    let result = start(&backend, session_id, "hi").await.await;
    assert_ne!(
        result.expect("turn completes").stop_reason,
        TurnStopReason::Cancelled
    );
}

#[tokio::test]
async fn a_stored_message_starts_a_turn_without_writing_it_again() {
    let (backend, runtime, session_id) = backend_with_runtime(Model::simulated("Sure.")).await;
    let input = AcceptedTurnInput::new(everruns_core::InputMessage::user("hi"));
    let message_id = runtime
        .persist_accepted_input(session_id, input)
        .await
        .expect("the caller records the message");

    let turn_id = TurnId::new();
    let result = backend
        .start_turn(TurnRequest::new(
            session_id,
            turn_id,
            TurnInput::StoredMessage { message_id },
        ))
        .await
        .expect("turn starts")
        .await
        .expect("turn completes");
    assert!(result.success);
    assert_eq!(result.response, "Sure.");
    assert_eq!(result.turn_id, turn_id, "the turn keeps the requested id");
    let copies = runtime
        .messages(session_id)
        .await
        .expect("history loads")
        .into_iter()
        .filter(|message| message.id == message_id)
        .count();
    assert_eq!(
        copies, 1,
        "the backend wrote the stored message no second time"
    );
    assert!(!backend.is_running(session_id).await);
}

#[tokio::test]
async fn recorded_tool_results_with_nothing_parked_fail_and_release_the_session() {
    let (backend, session_id) = backend(Model::simulated("Sure.")).await;
    let error = backend
        .start_turn(TurnRequest::new(
            session_id,
            TurnId::new(),
            TurnInput::RecordedToolResults {
                resolution_id: uuid::Uuid::now_v7(),
            },
        ))
        .await
        .expect("the turn starts")
        .await
        .expect_err("nothing is parked");
    assert!(error.to_string().contains("no turn waiting"), "{error}");
    assert!(!backend.is_running(session_id).await);
    assert_eq!(backend.active_count().await, 0);

    // The failed resume released the session, so it still takes a turn.
    start(&backend, session_id, "hi")
        .await
        .await
        .expect("turn completes");
}

/// An actor runner over `runtime` whose leases share a table with `elsewhere`,
/// a second holder standing in for another process.
fn actor(
    runtime: InProcessRuntime,
) -> (
    everruns_core::host::ActorRunner,
    everruns_core::host::InMemorySessionLeases,
) {
    let leases = everruns_core::host::InMemorySessionLeases::new();
    let elsewhere = leases.another_holder();
    (
        everruns_core::host::ActorRunner::new(runtime, std::sync::Arc::new(leases))
            .with_lease_ttl(Duration::from_millis(300)),
        elsewhere,
    )
}

#[tokio::test]
async fn a_turn_waits_out_the_lease_a_dead_process_left_and_then_runs() {
    let (_, runtime, session_id) = backend_with_runtime(Model::simulated("Sure.")).await;
    let (runner, elsewhere) = actor(runtime);
    // The other holder took the lease and stopped renewing, as a process
    // that died mid-turn does.
    use everruns_core::host::SessionLeases as _;
    elsewhere
        .acquire(session_id, Duration::from_millis(100))
        .await
        .unwrap()
        .expect("free");

    let input = AcceptedTurnInput::new(everruns_core::InputMessage::user("hi"));
    let ticket = runner
        .start_turn(TurnRequest::new(
            session_id,
            TurnId::new(),
            TurnInput::Message(Box::new(input)),
        ))
        .await
        .expect("the lease expires within one lease life");
    assert_eq!(ticket.await.expect("turn completes").response, "Sure.");
}

#[tokio::test]
async fn a_turn_on_a_session_a_live_process_runs_fails() {
    let (_, runtime, session_id) = backend_with_runtime(Model::simulated("Sure.")).await;
    let (runner, elsewhere) = actor(runtime);
    use everruns_core::host::SessionLeases as _;
    elsewhere
        .acquire(session_id, Duration::from_secs(30))
        .await
        .unwrap()
        .expect("free");

    let input = AcceptedTurnInput::new(everruns_core::InputMessage::user("hi"));
    let error = runner
        .start_turn(TurnRequest::new(
            session_id,
            TurnId::new(),
            TurnInput::Message(Box::new(input)),
        ))
        .await
        .expect_err("another process holds the session");
    assert!(
        error.to_string().contains("runs in another process"),
        "{error}"
    );
}
